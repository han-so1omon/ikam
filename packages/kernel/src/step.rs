//! Step kinds (E441, an option): what a stored step may do with floats and
//! nondeterminism, chosen by whoever records the step.
//!
//! - *pure* (the default, unchanged): `apply` records a derivation; floats
//!   are off; repack may drop the output and rebuild it.
//! - *pinned-float*: [`Repo::apply_pinned`] records the derivation
//!   `wasm-float(module, args...)`, which runs the module with floats on and
//!   every NaN canonical (`wasm::Floats::Pinned`). Still pure: rebuilt like
//!   any derivation, on any platform (E409's rules 1 to 3 hold by
//!   construction in the interpreter; rule 4, math shipped inside the
//!   module, is the module's to keep).
//! - *recorded*: [`Repo::record_step`] keeps the output's bytes and a record
//!   of `func(args)` that is never used to rebuild them. Floats, seeds and
//!   work done outside the kernel (a command, an agent) are all allowed. A
//!   rerun ([`Repo::rerun`]) is a check: it reports same or different, and
//!   the first output stays the one of record.
//!
//! A recorded step may name a check step ([`Check`]): when a rerun differs,
//! `verdict = check.func(first, now, check.args...)` is applied as an
//! ordinary derivation, so the bound a step is held to lives in the graph
//! with the step, versioned and shared like it, and the verdict has the same
//! provenance as any derived content. [`crate::func::within_f64`] is one
//! such check (ULP, absolute and relative bounds on f64 outputs); any pure
//! function, builtin or WASM, can be another.
//!
//! The record is a [`Run`] object of its own (tag `E`), outside the ledger,
//! so nothing that rebuilds content ever sees it. E441 also measured a
//! second design, a derivation of the builtin `recorded`; it was dropped
//! (E455), and the builtin stays only to refuse evaluation. Nothing in the
//! default paths calls this module.

use std::collections::HashSet;

use crate::dict::object_tag;
use crate::{Arg, Check, Derivation, Error, Id, Object, Repo, Run, Store, func};

/// What a rerun of a recorded step found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rerun {
    /// The output of record (the first one).
    pub first: Id,
    /// The rerun's output (stored as content, so a verdict can name it).
    pub now: Id,
    pub same: bool,
    /// The check step's verdict, when the step names one and they differ.
    pub verdict: Option<Id>,
    /// Whether the verdict's first word is `within` (or `same`).
    pub within: Option<bool>,
}

impl<S: Store> Repo<S> {
    /// A pinned-float step: `wasm-float(module, args...)`, a pure
    /// derivation whose module may use floats.
    pub fn apply_pinned(&mut self, module: Id, args: Vec<Arg>) -> Result<Id, Error> {
        let mut all = vec![Arg::Whole(module)];
        all.extend(args);
        self.apply(func::pinned_float(), all)
    }

    /// Record a step whose output is kept. With `output`, the work was done
    /// elsewhere (a command, an agent) and these are its bytes; without, the
    /// kernel runs `func` (a WASM module, floats pinned) on `args` now.
    /// Returns the output's id and the record's id.
    pub fn record_step(
        &mut self,
        func: Id,
        args: Vec<Arg>,
        output: Option<&[u8]>,
        check: Option<Check>,
    ) -> Result<(Id, Id), Error> {
        // A graph root is not content (E452): a run's arguments are.
        if let Some(a) = args.iter().find(|a| a.is_graph()) {
            return Err(crate::object::graph_not_content(a.id()));
        }
        let bytes = match output {
            Some(b) => b.to_vec(),
            None => self.run_step(&func, &args)?,
        };
        let output = self.put_content(&bytes)?.id;
        let record = self.write_run(Run {
            output,
            func,
            args,
            check,
            props: Vec::new(),
        })?;
        Ok((output, record))
    }

    /// Store a recorded step's record (design `Run`) and return its id.
    pub(crate) fn write_run(&mut self, run: Run) -> Result<Id, Error> {
        let encoded = Object::Run(run).encode();
        let id = Id::of(&encoded);
        self.write(id, &encoded)?;
        Ok(id)
    }

    /// Every recorded step of `output`.
    pub fn step_records(&self, output: &Id) -> Result<Vec<(Id, Run)>, Error> {
        let mut out = Vec::new();
        for id in self.store.ids()? {
            if object_tag(&self.store.read_head(&id, 2)?) == Some(b'E')
                && let Object::Run(r) = self.get(&id)?
                && r.output == *output
            {
                out.push((id, r));
            }
        }
        Ok(out)
    }

    /// Rerun the recorded step `record` and compare with its output of
    /// record. `now` is the rerun's bytes for work done elsewhere; without
    /// it the kernel runs the step's function again. Nothing recorded
    /// changes: a differing rerun is reported, and judged by the step's
    /// check when it names one.
    pub fn rerun(&mut self, record: &Id, now: Option<&[u8]>) -> Result<Rerun, Error> {
        let run = match self.get(record)? {
            Object::Run(r) => r,
            _ => return Err(Error::WrongKind(*record)),
        };
        let bytes = match now {
            Some(b) => b.to_vec(),
            None => self.run_step(&run.func, &run.args)?,
        };
        let now = self.put_content(&bytes)?.id;
        let same = now == run.output;
        let (mut verdict, mut within) = (None, None);
        if let Some(c) = run.check.filter(|_| !same) {
            let mut args = vec![Arg::Whole(run.output), Arg::Whole(now)];
            args.extend(c.args);
            let v = self.apply(c.func, args)?;
            let text = self.read_content(&v)?;
            within = Some(text.starts_with(b"within") || text.starts_with(b"same"));
            verdict = Some(v);
        }
        Ok(Rerun {
            first: run.output,
            now,
            same,
            verdict,
            within,
        })
    }

    pub(crate) fn run_step(&self, func: &Id, args: &[Arg]) -> Result<Vec<u8>, Error> {
        if func::is_builtin(func) {
            return Ok(self.evaluate(func, args)?.0);
        }
        let mut all = vec![Arg::Whole(*func)];
        all.extend_from_slice(args);
        Ok(self.evaluate(&func::pinned_float(), &all)?.0)
    }

    /// Recorded steps whose outputs are in `content`, added
    /// to `records`, with what their functions, arguments and checks need
    /// added to `content` (through derivations `follow` accepts), to a
    /// fixed point: a step's input may be another step's output. A full
    /// scan of the store for run objects.
    pub(crate) fn add_runs(
        &self,
        content: &mut HashSet<Id>,
        records: &mut HashSet<Id>,
        follow: impl Fn(&Derivation) -> bool + Copy,
    ) -> Result<(), Error> {
        let mut runs = Vec::new();
        for id in self.store.ids()? {
            if object_tag(&self.store.read_head(&id, 2)?) == Some(b'E')
                && let Object::Run(r) = self.get(&id)?
            {
                runs.push((id, r));
            }
        }
        loop {
            let mut stack = Vec::new();
            for (id, r) in &runs {
                if content.contains(&r.output) && records.insert(*id) {
                    stack.extend(r.args.iter().map(Arg::id));
                    stack.extend(Some(r.func).filter(|f| !func::is_builtin(f)));
                    if let Some(c) = &r.check {
                        stack.extend(c.args.iter().map(Arg::id));
                        stack.extend(Some(c.func).filter(|f| !func::is_builtin(f)));
                    }
                }
            }
            if stack.is_empty() {
                return Ok(());
            }
            let (c, r) = self.closure(stack, follow)?;
            content.extend(c);
            records.extend(r);
        }
    }
}

/// One step a lineage walk found (E427).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// A pure derivation (not a storage plan): rebuilt from its inputs.
    Derived { record: Id, func: Id, args: Vec<Id> },
    /// A recorded step: kept, never rebuilt; a rerun is a check.
    Recorded {
        record: Id,
        func: Id,
        args: Vec<Id>,
        check: Option<(Id, Vec<Id>)>,
    },
    /// A view's pin (`sampler.rs`): the commit and tree a view was read
    /// from.
    Pin { commit: Id, tree: Id },
}

/// What a lineage walk from one content found: every content it reached
/// with the steps that made it, the commits named by pins, and what it
/// needed but could not find.
#[derive(Clone, Debug, Default)]
pub struct Lineage {
    /// In walk order (breadth first from the start), each content with the
    /// steps that made it; a content with none is a leaf (put as bytes).
    pub nodes: Vec<(Id, Vec<Found>)>,
    pub commits: Vec<Id>,
    pub missing: Vec<Id>,
}

impl Lineage {
    /// Every recorded step reached.
    pub fn recorded(&self) -> impl Iterator<Item = (&Id, &Found)> {
        (self.nodes.iter())
            .flat_map(|(id, fs)| fs.iter().map(move |f| (id, f)))
            .filter(|(_, f)| matches!(f, Found::Recorded { .. }))
    }
}

impl<S: Store> Repo<S> {
    /// Walk back from `id` through every step that made it (E427): recorded
    /// steps (their function, arguments and check), pure derivations other
    /// than storage plans (their function and arguments), and view pins
    /// (the commit a view was read from), to a fixed point. Leaves are
    /// contents put as bytes. Run records are found with one scan of the
    /// store.
    pub fn lineage(&self, id: &Id) -> Result<Lineage, Error> {
        let mut runs: std::collections::HashMap<Id, Vec<(Id, Run)>> = Default::default();
        for rid in self.store.ids()? {
            if object_tag(&self.store.read_head(&rid, 2)?) == Some(b'E')
                && let Object::Run(r) = self.get(&rid)?
            {
                runs.entry(r.output).or_default().push((rid, r));
            }
        }
        let mut out = Lineage::default();
        let mut seen = HashSet::from([*id]);
        let mut queue = std::collections::VecDeque::from([*id]);
        let ids = |args: &[Arg]| -> Vec<Id> { args.iter().map(Arg::id).collect() };
        while let Some(c) = queue.pop_front() {
            if !self.has_content(&c)? {
                out.missing.push(c);
                continue;
            }
            let mut found = Vec::new();
            for d in self.derivations(&c)? {
                if !func::is_plan(&d.func) && d.func != func::recorded() {
                    found.push(Found::Derived {
                        record: Object::Derivation(d.clone()).id(),
                        func: d.func,
                        args: ids(&d.args),
                    });
                }
            }
            for (rid, r) in runs.get(&c).into_iter().flatten() {
                found.push(Found::Recorded {
                    record: *rid,
                    func: r.func,
                    args: ids(&r.args),
                    check: r.check.as_ref().map(|k| (k.func, ids(&k.args))),
                });
            }
            let bytes = self.read_content(&c)?;
            if let Some((commit, tree)) = crate::sampler::parse_pin(&bytes) {
                if !self.store.has(&commit) || !self.store.has(&tree) {
                    out.missing.push(commit);
                }
                out.commits.push(commit);
                found.push(Found::Pin { commit, tree });
            }
            for f in &found {
                let next: Vec<Id> = match f {
                    Found::Derived { func, args, .. } => {
                        std::iter::once(*func).chain(args.iter().copied()).collect()
                    }
                    Found::Recorded {
                        func, args, check, ..
                    } => std::iter::once(*func)
                        .chain(args.iter().copied())
                        .chain(
                            check
                                .iter()
                                .flat_map(|(f, a)| std::iter::once(*f).chain(a.iter().copied())),
                        )
                        .collect(),
                    Found::Pin { .. } => vec![],
                };
                for n in next {
                    if !func::is_builtin(&n) && seen.insert(n) {
                        queue.push_back(n);
                    }
                }
            }
            out.nodes.push((c, found));
        }
        Ok(out)
    }
}

/// ULP distance between two f64 values: their bit patterns mapped to one
/// ordered integer line (so -0.0 and 0.0 are one apart).
fn ulps(a: f64, b: f64) -> u64 {
    let key = |x: f64| {
        let b = x.to_bits() as i64;
        if b < 0 { i64::MIN - b } else { b }
    };
    key(a).abs_diff(key(b))
}

/// `within-f64(first, now, spec)`: see `func::within_f64`.
pub(crate) fn within(args: &[Vec<u8>]) -> Result<Vec<u8>, Error> {
    let [first, now, spec] = args else {
        return Err(Error::Exec("within-f64: three arguments".into()));
    };
    // Unset bounds never pass on their own.
    let (mut max_ulp, mut max_abs, mut max_rel) = (0u64, -1.0f64, -1.0f64);
    let spec = std::str::from_utf8(spec).map_err(|_| Error::Exec("within-f64: spec".into()))?;
    for part in spec.split_whitespace() {
        let bad = || Error::Exec(format!("within-f64: {part}"));
        let (k, v) = part.split_once('=').ok_or_else(bad)?;
        match k {
            "ulp" => max_ulp = v.parse().map_err(|_| bad())?,
            "abs" => max_abs = v.parse().map_err(|_| bad())?,
            "rel" => max_rel = v.parse().map_err(|_| bad())?,
            _ => return Err(bad()),
        }
    }
    if first == now {
        return Ok(b"same\n".to_vec());
    }
    if first.len() != now.len() || first.len() % 8 != 0 {
        return Ok(format!("outside length {} {}\n", first.len(), now.len()).into_bytes());
    }
    let f64s = |b: &[u8]| -> Vec<f64> {
        b.chunks_exact(8)
            .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
            .collect()
    };
    let (a, b) = (f64s(first), f64s(now));
    let (mut ulp, mut abs, mut rel, mut ok) = (0u64, 0.0f64, 0.0f64, true);
    for (x, y) in a.iter().zip(&b) {
        if x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()) {
            continue;
        }
        if x.is_nan() || y.is_nan() || x.is_infinite() || y.is_infinite() {
            ok = false;
            ulp = u64::MAX;
            abs = f64::INFINITY;
            rel = f64::INFINITY;
            continue;
        }
        let (u, d) = (ulps(*x, *y), (x - y).abs());
        let r = if *x == 0.0 {
            f64::INFINITY
        } else {
            d / x.abs()
        };
        ok &= u <= max_ulp || d <= max_abs || r <= max_rel;
        ulp = ulp.max(u);
        abs = abs.max(d);
        rel = rel.max(r);
    }
    let word = if ok { "within" } else { "outside" };
    Ok(format!("{word} ulp={ulp} abs={abs:e} rel={rel:e}\n").into_bytes())
}
