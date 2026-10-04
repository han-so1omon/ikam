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
use crate::repo::{Cx, decode_plain, plain_encoding};
use crate::{
    Arg, Derivation, Error, Form, Id, Object, Put, Repo, Store, container, func, template,
};

/// Bound on nested container expansion (a zip inside a zip ...).
const MAX_UNPACK_DEPTH: usize = 4;
/// Similar stored contents considered for templates.
const NEIGHBORS: usize = 3;
/// Deepest derivation nesting a new plan may need to rebuild its output.
/// Reads stay bounded; deeper layering must come from a deliberate pass.
const MAX_PLAN_DEPTH: usize = 8;

/// New content a candidate writes: (id, plain encoding, bytes).
type Literal = (Id, Vec<u8>, Vec<u8>);

/// A storage plan for one new content.
struct Candidate {
    /// Net bytes written: may be negative when it frees a stored copy.
    cost: isize,
    /// `derivations[0]` produces the new content; others re-express
    /// existing content through shared parts.
    derivations: Vec<Derivation>,
    /// New content the derivations read.
    literals: Vec<Literal>,
    /// Existing content whose stored bytes the plan makes redundant.
    drop: Option<Id>,
}

impl Candidate {
    fn single(d: Derivation, literals: Vec<Literal>) -> Candidate {
        let cost = derivation_size(&d) + literals.iter().map(|l| l.1.len()).sum::<usize>();
        Candidate {
            cost: cost as isize,
            derivations: vec![d],
            literals,
            drop: None,
        }
    }
}

impl<S: Store> Repo<S> {
    pub fn put_content(&mut self, bytes: &[u8]) -> Result<Put, Error> {
        self.put_content_with(bytes, 0, &[])
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
        let plain = plain_encoding(bytes);
        let mut candidates = Vec::new();
        for d in hints {
            if self.verify(d, bytes, &HashMap::new()) {
                candidates.push(Candidate::single(d.clone(), vec![]));
            }
        }
        if depth < MAX_UNPACK_DEPTH {
            candidates.extend(self.container_candidate(bytes, depth)?);
        }
        candidates.extend(self.slice_candidate(bytes)?);
        candidates.extend(self.template_candidates(bytes)?);
        let best = candidates
            .into_iter()
            .filter(|c| c.cost < plain.len() as isize)
            .min_by_key(|c| c.cost);
        let Some(c) = best else {
            self.materialize(id, &plain, bytes)?;
            return Ok(Put {
                id,
                form: Form::Blob,
            });
        };
        for (lit_id, encoded, lit) in &c.literals {
            self.materialize(*lit_id, encoded, lit)?;
        }
        for d in c.derivations {
            self.record(d)?;
        }
        if let Some(old) = c.drop {
            self.store.delete(&old)?;
        }
        Ok(Put {
            id,
            form: Form::Derived,
        })
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
        if !self.verify(&d, bytes, &HashMap::new()) {
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

    fn verify(&self, d: &Derivation, bytes: &[u8], known: &HashMap<Id, Vec<u8>>) -> bool {
        let mut cx = Cx::default();
        cx.known = known.clone();
        d.output == Id::of_content(bytes)
            && self
                .eval(&d.func, &d.args, &mut cx)
                .is_ok_and(|out| out == bytes)
            && cx.deepest < MAX_PLAN_DEPTH
    }

    /// `(id, plain encoding, bytes)` for content not yet stored or derivable.
    fn new_literal(&self, bytes: Vec<u8>) -> Result<Option<Literal>, Error> {
        let id = Id::of_content(&bytes);
        Ok((!self.has_content(&id)?).then(|| (id, plain_encoding(&bytes), bytes)))
    }

    fn container_candidate(
        &mut self,
        bytes: &[u8],
        depth: usize,
    ) -> Result<Option<Candidate>, Error> {
        let Some((manifest, members)) = container::plan_zip(bytes) else {
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
            .then(|| Candidate::single(d, vec![])))
    }

    fn slice_candidate(&mut self, bytes: &[u8]) -> Result<Option<Candidate>, Error> {
        self.ensure_index()?;
        let store = &self.store;
        let load = |id: &Id| store.read(id).ok().and_then(|e| decode_plain(id, &e).ok());
        let plan = matcher::plan(bytes, self.index.as_ref().unwrap(), load);
        let mut literal = Vec::new();
        for part in &plan {
            if let Part::Input(r) = part {
                literal.extend_from_slice(&bytes[r.clone()]);
            }
        }
        let lit_id = Id::of_content(&literal);
        let (mut args, mut lit_pos) = (Vec::new(), 0);
        for part in &plan {
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
        if !self.verify(&d, bytes, &HashMap::from([(lit_id, literal.clone())])) {
            return Ok(None);
        }
        let literal = if literal.is_empty() {
            None
        } else {
            self.new_literal(literal)?
        };
        Ok(Some(Candidate::single(d, literal.into_iter().collect())))
    }

    /// Template candidates against the most similar stored contents: fit an
    /// existing template, or anti-unify with a neighbour into a new one.
    fn template_candidates(&mut self, bytes: &[u8]) -> Result<Vec<Candidate>, Error> {
        self.ensure_index()?;
        let (neighbors, seeds) = matcher::neighbors(bytes, self.index.as_ref().unwrap(), NEIGHBORS);
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
                if self.verify(&d, bytes, &HashMap::from([(f, fillers.clone())])) {
                    out.push(Candidate::single(
                        d,
                        self.new_literal(fillers)?.into_iter().collect(),
                    ));
                }
            }
        }
        if let Some((n, shared)) = neighbors.first()
            && matcher::similar(*shared, seeds)
        {
            out.extend(self.new_template_candidate(bytes, *n)?);
        }
        Ok(out)
    }

    /// Anti-unify `bytes` with stored content `y`; both become `fill`s of a
    /// new shared template, and `y`'s stored bytes are credited back. Only a
    /// plain document qualifies as `y`: not a template, fillers, or content
    /// with derivations, so templates never chain through repeated pairing.
    fn new_template_candidate(&self, bytes: &[u8], y: Id) -> Result<Option<Candidate>, Error> {
        let Ok(stored) = self.store.read(&y) else {
            return Ok(None);
        };
        let Ok(y_bytes) = decode_plain(&y, &stored) else {
            return Ok(None);
        };
        if !self.derivations(&y)?.is_empty() || template::decode_parts(&y_bytes).is_some() {
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
        if !self.verify(&dx, bytes, &known) || !self.verify(&dy, &y_bytes, &known) {
            return Ok(None);
        }
        let mut literals = Vec::new();
        for part in [t, fy, fx] {
            literals.extend(self.new_literal(part)?);
        }
        literals.dedup_by_key(|l| l.0);
        let written = derivation_size(&dx)
            + derivation_size(&dy)
            + literals.iter().map(|l| l.1.len()).sum::<usize>();
        let cost = written as isize - stored.len() as isize;
        Ok(Some(Candidate {
            cost,
            derivations: vec![dx, dy],
            literals,
            drop: Some(y),
        }))
    }

    pub(crate) fn ensure_index(&mut self) -> Result<(), Error> {
        if self.index.is_none() {
            let mut index = Index::default();
            for id in self.store.ids()? {
                if let Ok(bytes) = decode_plain(&id, &self.store.read(&id)?) {
                    index.add(id, &bytes);
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
