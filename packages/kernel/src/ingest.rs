//! Ingest: candidate storage plans compete on cost; only verified ones count.
//!
//! Candidates for new content:
//! - plain bytes (always valid)
//! - a slice plan: `concat` of byte ranges of stored content plus one new
//!   literal blob, proposed by the data-driven matcher
//! - container unpacking: `deflate-pack(manifest, members)`
//! - templates: `fill(template, fillers)` with an existing template, or a
//!   new template anti-unified with a similar stored content, which is then
//!   re-expressed through the same template
//! - hints: derivations proposed by a caller (e.g. `repack`)
//!
//! Every derivation in a candidate is evaluated and must reproduce its
//! output exactly. The cheapest valid candidate is written. Any other
//! proposer, including an AI, goes through `put_derivation`.

use std::collections::HashMap;

use crate::matcher::{self, Index, Part};
use crate::repo::Cx;
use crate::{
    Arg, Derivation, Error, Form, Id, Object, Put, Repo, Store, container, func, template,
};

/// Bound on nested container expansion (a zip inside a zip ...).
const MAX_UNPACK_DEPTH: usize = 4;
/// Similar stored contents considered for templates.
const NEIGHBORS: usize = 3;

/// New content a candidate writes: (id, plain encoding, bytes).
type Literal = (Id, Vec<u8>, Vec<u8>);

/// A storage plan for one new content.
pub(crate) struct Candidate {
    /// Net bytes written: may be negative when it frees a stored copy.
    pub(crate) cost: isize,
    /// Decode work (bytes produced) to rebuild what the plan derives.
    pub(crate) work: usize,
    /// `derivations[0]` produces the new content; others re-express
    /// existing content through shared parts.
    pub(crate) derivations: Vec<Derivation>,
    /// New content the derivations read.
    pub(crate) literals: Vec<Literal>,
    /// Existing content whose stored bytes the plan makes redundant.
    pub(crate) drop: Option<Id>,
}

impl Candidate {
    fn single(d: Derivation, literals: Vec<Literal>, work: usize) -> Candidate {
        let cost = derivation_size(&d) + literals.iter().map(|l| l.1.len()).sum::<usize>();
        Candidate {
            cost: cost as isize,
            work,
            derivations: vec![d],
            literals,
            drop: None,
        }
    }
}

impl<S: Store> Repo<S> {
    pub fn put_content(&mut self, bytes: &[u8]) -> Result<Put, Error> {
        let began = self.begin_memo();
        let out = self.put_content_with(bytes, 0, &[]);
        self.end_memo(began);
        out
    }

    pub(crate) fn put_content_with(
        &mut self,
        bytes: &[u8],
        depth: usize,
        hints: &[Derivation],
    ) -> Result<Put, Error> {
        let id = Id::of_content(bytes);
        if self.has_content(&id)? {
            return Ok(Put {
                id,
                form: Form::Existing,
            });
        }
        let plain = self.encode_plain(bytes);
        let mut candidates = Vec::new();
        let mut unpacked = false;
        for d in hints {
            if let Some(work) = self.verify(d, bytes, &HashMap::new()) {
                unpacked |= d.func == func::deflate_pack();
                candidates.push(Candidate::single(d.clone(), vec![], work));
            }
        }
        // A verified container fact (a re-plan's hint) is the plan unpacking
        // would find again, at the price of re-deflating every member at
        // each level it tries (E109: most of a re-plan of source archives).
        if depth < MAX_UNPACK_DEPTH && !unpacked {
            candidates.extend(self.container_candidate(bytes, depth)?);
        }
        // A slice or template plan rebuilds the whole content, so its read
        // work is at least its length: when that alone costs more than
        // storing it plain, the plan cannot win and is not searched for
        // (E092; the heavy read weight, every content).
        if self.read_weight * (bytes.len() as f64) < plain.len() as f64 {
            candidates.extend(self.slice_candidate(bytes)?);
            if self.object_templates || !self.writing_object {
                candidates.extend(self.template_candidates(bytes)?);
            }
        }
        let best = candidates
            .into_iter()
            .map(|c| (c.cost as f64 + self.read_weight * c.work as f64, c))
            .filter(|(score, _)| *score < plain.len() as f64)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, c)| c);
        let Some(c) = best else {
            self.materialize(id, &plain, bytes)?;
            return Ok(Put {
                id,
                form: Form::Blob,
            });
        };
        self.apply_candidate(c)?;
        Ok(Put {
            id,
            form: Form::Derived,
        })
    }

    /// Write a verified candidate: its new content, its derivations, then
    /// drop the stored copy it makes redundant (only after the rest exists).
    pub(crate) fn apply_candidate(&mut self, c: Candidate) -> Result<(), Error> {
        for (lit_id, encoded, lit) in &c.literals {
            self.materialize(*lit_id, encoded, lit)?;
        }
        for d in c.derivations {
            self.record(d)?;
        }
        // The dictionary, and anything another stored encoding is decoded
        // from (a dictionary, a group, a "Q" or "M" context), must stay
        // stored (E123: dropping a group lost its members).
        if let Some(old) = c.drop
            && Some(old) != self.dictionary().map(|(id, _)| *id)
            && !self.is_needed(&old)?
        {
            self.store.delete(&old)?;
        }
        Ok(())
    }

    /// Record `bytes = func(args)` if evaluating it reproduces `bytes`
    /// exactly. This is the entry point for semantic dedup: any planner,
    /// including an AI, may propose a derivation. Returns `None` (and writes
    /// nothing) if the proposal does not reproduce the bytes. A verified
    /// derivation is recorded even if the bytes are already stored: it is a
    /// ledger fact that `repack` may later use to drop the stored copy.
    pub fn put_derivation(
        &mut self,
        bytes: &[u8],
        func: Id,
        args: Vec<Arg>,
    ) -> Result<Option<Put>, Error> {
        let id = Id::of_content(bytes);
        let d = Derivation {
            output: id,
            func,
            args,
        };
        if self.verify(&d, bytes, &HashMap::new()).is_none() {
            return Ok(None);
        }
        let form = if self.has_content(&id)? {
            Form::Existing
        } else {
            Form::Derived
        };
        self.record(d)?;
        Ok(Some(Put { id, form }))
    }

    /// Decode work if `d` reproduces `bytes` exactly, else `None`.
    fn verify(&self, d: &Derivation, bytes: &[u8], known: &HashMap<Id, Vec<u8>>) -> Option<usize> {
        self.verify_without(d, bytes, known, None)
    }

    /// `verify`, optionally forbidding any read of `without`: required before
    /// dropping `without`'s stored copy, so the derivation that replaces it
    /// cannot depend on that copy (directly or through other derivations).
    pub(crate) fn verify_without(
        &self,
        d: &Derivation,
        bytes: &[u8],
        known: &HashMap<Id, Vec<u8>>,
        without: Option<Id>,
    ) -> Option<usize> {
        let mut cx = Cx {
            known: known.clone(),
            ..Cx::default()
        };
        if let Some(w) = without {
            cx.stack_guard(w);
        }
        let ok = d.output == Id::of_content(bytes)
            && self
                .eval(&d.func, &d.args, &mut cx)
                .is_ok_and(|out| out == bytes);
        ok.then_some(cx.work)
    }

    /// `(id, plain encoding, bytes)` for content not yet stored or derivable.
    fn new_literal(&self, bytes: Vec<u8>) -> Result<Option<Literal>, Error> {
        let id = Id::of_content(&bytes);
        Ok((!self.has_content(&id)?).then(|| (id, self.encode_plain(&bytes), bytes)))
    }

    fn container_candidate(
        &mut self,
        bytes: &[u8],
        depth: usize,
    ) -> Result<Option<Candidate>, Error> {
        let zip = match self.markup_layers {
            true => container::plan_zip_layered(bytes),
            false => container::plan_zip(bytes),
        };
        let Some((manifest, members)) = zip.or_else(|| container::plan_zlib(bytes)) else {
            return Ok(None);
        };
        // Members are stored whatever is chosen: they are content in their
        // own right (graph nodes, dedup sources). The choice left is only
        // between this record and also storing the container's bytes.
        let mut args = vec![Arg::Whole(
            self.put_content_with(&manifest, depth + 1, &[])?.id,
        )];
        for m in &members {
            args.push(Arg::Whole(self.put_content_with(m, depth + 1, &[])?.id));
        }
        let d = Derivation {
            output: Id::of_content(bytes),
            func: func::deflate_pack(),
            args,
        };
        Ok(self
            .verify(&d, bytes, &HashMap::new())
            .map(|work| Candidate::single(d, vec![], work)))
    }

    fn slice_candidate(&mut self, bytes: &[u8]) -> Result<Option<Candidate>, Error> {
        self.ensure_index()?;
        let store = &self.store;
        let load = |id: &Id| {
            if let Some(bytes) = self.memo_get(id) {
                return Some(bytes);
            }
            let e = store.read(id).ok()?;
            let bytes = self.decode_plain(id, &e).ok()?;
            if crate::dict::self_verified(&e) || Id::of_content(&bytes) == *id {
                self.memo_put(id, &bytes, e.len());
            }
            Some(bytes)
        };
        let plan = matcher::plan(bytes, self.index.as_ref().unwrap(), load);
        self.concat_candidate(bytes, &plan)
    }

    /// A `concat` candidate from a slice plan: reused ranges plus one new
    /// literal blob holding the unmatched bytes.
    pub(crate) fn concat_candidate(
        &self,
        bytes: &[u8],
        plan: &[Part],
    ) -> Result<Option<Candidate>, Error> {
        let mut literal = Vec::new();
        for part in plan {
            if let Part::Input(r) = part {
                literal.extend_from_slice(&bytes[r.clone()]);
            }
        }
        let lit_id = Id::of_content(&literal);
        let (mut args, mut lit_pos) = (Vec::new(), 0);
        for part in plan {
            let (id, start, len) = match part {
                Part::Input(r) => (lit_id, lit_pos, r.len()),
                Part::Existing { src, start, len } => (*src, *start, *len),
            };
            if let Part::Input(_) = part {
                lit_pos += len;
            }
            push_range(&mut args, id, start as u64, len as u64);
        }
        if args.iter().all(|a| a.id() == lit_id) {
            return Ok(None); // nothing reused: plain bytes are strictly smaller
        }
        let d = Derivation {
            output: Id::of_content(bytes),
            func: func::concat(),
            args,
        };
        let Some(work) = self.verify(&d, bytes, &HashMap::from([(lit_id, literal.clone())])) else {
            return Ok(None);
        };
        let literal = if literal.is_empty() {
            None
        } else {
            self.new_literal(literal)?
        };
        Ok(Some(Candidate::single(
            d,
            literal.into_iter().collect(),
            work,
        )))
    }

    /// Template candidates against the most similar stored contents: fit an
    /// existing template, or anti-unify with a neighbour into a new one.
    fn template_candidates(&mut self, bytes: &[u8]) -> Result<Vec<Candidate>, Error> {
        self.ensure_index()?;
        let (neighbors, _) = matcher::neighbors(bytes, self.index.as_ref().unwrap(), NEIGHBORS);
        let mut templates = Vec::new();
        for (n, _) in &neighbors {
            templates.extend(
                self.derivations(n)?
                    .iter()
                    .filter(|d| d.func == func::fill())
                    .map(|d| d.args[0].id()),
            );
            templates.push(*n); // any content that parses as a template may be one
        }
        templates.dedup();
        let mut out = Vec::new();
        for t in templates {
            let Ok(t_bytes) = self.read_content(&t) else {
                continue;
            };
            if let Some(fillers) = template::fit(&t_bytes, bytes) {
                let f = Id::of_content(&fillers);
                let d = Derivation {
                    output: Id::of_content(bytes),
                    func: func::fill(),
                    args: vec![Arg::Whole(t), Arg::Whole(f)],
                };
                if let Some(work) = self.verify(&d, bytes, &HashMap::from([(f, fillers.clone())])) {
                    out.push(Candidate::single(
                        d,
                        self.new_literal(fillers)?.into_iter().collect(),
                        work,
                    ));
                }
            }
        }
        if let Some((n, _)) = neighbors.first() {
            out.extend(self.new_template_candidate(bytes, *n)?);
        }
        Ok(out)
    }

    /// Anti-unify `bytes` with stored content `y`; both become `fill`s of a
    /// new shared template, and `y`'s stored bytes are credited back. Only a
    /// plain document qualifies as `y`: not a template, fillers, content
    /// with derivations (so templates never chain through repeated pairing),
    /// or the dictionary (which must stay stored).
    fn new_template_candidate(&self, bytes: &[u8], y: Id) -> Result<Option<Candidate>, Error> {
        // E143: `template_candidates` has just read `y` as a template, so
        // with `read_memo` on its bytes are kept; only the stored length is
        // needed besides them.
        let (y_bytes, stored_len) = match self.memo_get_stored(&y) {
            Some(kept) => kept,
            None => {
                let Ok(stored) = self.store.read(&y) else {
                    return Ok(None);
                };
                let Ok(y_bytes) = self.decode_plain(&y, &stored) else {
                    return Ok(None);
                };
                if crate::dict::self_verified(&stored) || Id::of_content(&y_bytes) == y {
                    self.memo_put(&y, &y_bytes, stored.len());
                }
                (y_bytes, stored.len())
            }
        };
        let is_dict = self.dictionary().is_some_and(|(d, _)| *d == y);
        if is_dict
            || !self.derivations(&y)?.is_empty()
            || template::decode_parts(&y_bytes).is_some()
        {
            return Ok(None);
        }
        let Some((t, fy, fx)) = template::anti_unify(&y_bytes, bytes) else {
            return Ok(None);
        };
        let (t_id, fy_id, fx_id) = (Id::of_content(&t), Id::of_content(&fy), Id::of_content(&fx));
        let known = HashMap::from([(t_id, t.clone()), (fy_id, fy.clone()), (fx_id, fx.clone())]);
        let fill = |output, f| Derivation {
            output,
            func: func::fill(),
            args: vec![Arg::Whole(t_id), Arg::Whole(f)],
        };
        let (dx, dy) = (fill(Id::of_content(bytes), fx_id), fill(y, fy_id));
        let (Some(work_x), Some(work_y)) = (
            self.verify(&dx, bytes, &known),
            self.verify_without(&dy, &y_bytes, &known, Some(y)),
        ) else {
            return Ok(None);
        };
        let mut literals = Vec::new();
        for part in [t, fy, fx] {
            literals.extend(self.new_literal(part)?);
        }
        literals.dedup_by_key(|l| l.0);
        let written = derivation_size(&dx)
            + derivation_size(&dy)
            + literals.iter().map(|l| l.1.len()).sum::<usize>();
        let cost = written as isize - stored_len as isize;
        Ok(Some(Candidate {
            cost,
            work: work_x + work_y,
            derivations: vec![dx, dy],
            literals,
            drop: Some(y),
        }))
    }

    pub(crate) fn ensure_index(&mut self) -> Result<(), Error> {
        if self.index.is_none() {
            let mut index = Index::default();
            // In id order, so the same objects give the same index (and the
            // same encodings) on any store and in any run (E136).
            let ids: Vec<Id> = match &self.seed_scope {
                None => {
                    let mut ids = self.store.ids()?;
                    ids.sort();
                    ids
                }
                // Only what these contents are made of (E131).
                Some(roots) => {
                    let (content, _) = self.closure(roots.clone(), |_| true)?;
                    let mut ids: Vec<Id> = content.into_iter().collect();
                    ids.sort();
                    ids
                }
            };
            let mut seen = std::collections::HashSet::new();
            let mut stack: Vec<Id> = ids.into_iter().rev().collect();
            while let Some(id) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                // E145: what this edit has already read (or was handed).
                if let Some(bytes) = self.memo_get(&id) {
                    index.add(id, &bytes);
                    continue;
                }
                let Ok(encoded) = self.store.read(&id) else {
                    continue; // derived: its arguments are in `ids`
                };
                match self.decode_plain(&id, &encoded) {
                    Ok(bytes) => {
                        if crate::dict::self_verified(&encoded) || Id::of_content(&bytes) == id {
                            self.memo_put(&id, &bytes, encoded.len());
                        }
                        index.add(id, &bytes)
                    }
                    // A tree or graph chunk named in a scope stands for the
                    // content holding its encoding, so its next version can
                    // repeat it (E133). On the whole store that content is
                    // indexed in its own right.
                    Err(_) if self.seed_scope.is_some() => {
                        if let Some(content) = crate::dict::tree_content(&encoded) {
                            let mut more: Vec<Id> = self
                                .closure(vec![content], |_| true)?
                                .0
                                .into_iter()
                                .collect();
                            more.sort_unstable_by(|a, b| b.cmp(a));
                            stack.extend(more);
                        }
                    }
                    Err(_) => {}
                }
            }
            self.index = Some(index);
        }
        Ok(())
    }
}

/// Append a range, merging it into the previous one when contiguous.
fn push_range(args: &mut Vec<Arg>, id: Id, start: u64, len: u64) {
    if len == 0 {
        return;
    }
    if let Some(Arg::Range {
        id: prev,
        start: s,
        len: l,
    }) = args.last_mut()
        && *prev == id
        && *s + *l == start
    {
        *l += len;
        return;
    }
    args.push(Arg::Range { id, start, len });
}

pub(crate) fn derivation_size(d: &Derivation) -> usize {
    Object::Derivation(d.clone()).encode().len()
}
