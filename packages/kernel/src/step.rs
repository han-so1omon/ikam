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
//! The recorded record is stored one of two ways, and E441 measures both:
//! [`Design::Run`] (a [`Run`] object of its own, tag `E`, outside the
//! ledger) and [`Design::Wrapped`] (a derivation whose function is the
//! builtin `recorded`, refused when evaluated). Nothing in the default
//! paths calls this module.

use std::collections::HashSet;

use crate::dict::object_tag;
use crate::{Arg, Check, Derivation, Error, Id, Object, Repo, Run, Store, func};

/// How a recorded step's record is stored (E441 compares the two).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Design {
    /// A `Run` object, outside the ledger: nothing that rebuilds content
    /// ever sees it.
    Run,
    /// A derivation `recorded(func, head, check args..., args...)` in the ledger, whose
    /// evaluation is refused.
    Wrapped,
}

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
        design: Design,
    ) -> Result<(Id, Id), Error> {
        let bytes = match output {
            Some(b) => b.to_vec(),
            None => self.run_step(&func, &args)?,
        };
        let output = self.put_content(&bytes)?.id;
        let record = match design {
            Design::Run => self.write_run(Run {
                output,
                func,
                args,
                check,
                props: Vec::new(),
            })?,
            Design::Wrapped => {
                // `recorded(func, head, check args..., args...)`. `head` is
                // text: `none`, or the check function's id and its argument
                // count. The check function cannot be an argument itself:
                // a builtin's id names no content, and an argument must.
                let (head, check_args) = match check {
                    None => ("none".to_string(), vec![]),
                    Some(c) => (format!("{} {}", c.func, c.args.len()), c.args),
                };
                let head = self.put_content(head.as_bytes())?.id;
                let mut all = vec![Arg::Whole(func), Arg::Whole(head)];
                // Measured as a cost of this design (E441): a WASM check
                // function named only in `head` is not kept live by gc.
                all.extend(check_args);
                all.extend(args);
                self.record(Derivation {
                    output,
                    func: func::recorded(),
                    args: all,
                })?
            }
        };
        Ok((output, record))
    }

    /// Store a recorded step's record (design `Run`) and return its id.
    pub(crate) fn write_run(&mut self, run: Run) -> Result<Id, Error> {
        let encoded = Object::Run(run).encode();
        let id = Id::of(&encoded);
        self.write(id, &encoded)?;
        Ok(id)
    }

    /// Every recorded step of `output`, in either design, as `Run`s.
    pub fn step_records(&self, output: &Id) -> Result<Vec<(Id, Run)>, Error> {
        let mut out = Vec::new();
        for d in self.derivations(output)? {
            if d.func == func::recorded() {
                let id = Object::Derivation(d.clone()).id();
                out.push((id, self.unwrap_run(&d)?));
            }
        }
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
            Object::Derivation(d) if d.func == func::recorded() => self.unwrap_run(&d)?,
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

    fn unwrap_run(&self, d: &Derivation) -> Result<Run, Error> {
        let bad = || Error::Decode("recorded step");
        let (Some(Arg::Whole(f)), Some(Arg::Whole(head))) = (d.args.first(), d.args.get(1)) else {
            return Err(bad());
        };
        let head = String::from_utf8(self.read_content(head)?).map_err(|_| bad())?;
        let (check, k) = match head.split_once(' ') {
            None if head == "none" => (None, 0),
            Some((func, k)) => (
                Some(func.parse::<Id>()?),
                k.parse::<usize>().map_err(|_| bad())?,
            ),
            None => return Err(bad()),
        };
        let rest = d.args.get(2..).filter(|r| r.len() >= k).ok_or_else(bad)?;
        Ok(Run {
            output: d.output,
            func: *f,
            args: rest[k..].to_vec(),
            check: check.map(|func| Check {
                func,
                args: rest[..k].to_vec(),
            }),
            props: Vec::new(),
        })
    }

    /// Recorded steps (design `Run`) whose outputs are in `content`, added
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
