//! Ingest: candidate storage plans compete on cost; only verified ones count.
//!
//! Candidates for new content:
//! - plain bytes (always valid)
//! - a slice plan: `concat` of byte ranges of stored content plus one new
//!   literal blob, proposed by the data-driven matcher
//! - container unpacking: `deflate-pack(manifest, members)`
//! - hints: derivations proposed by a caller (e.g. `repack` re-offering
//!   recorded derivations)
//!
//! Each derivation candidate is evaluated and must reproduce the bytes
//! exactly. The cheapest valid candidate is written. Any other proposer,
//! including an AI, goes through `put_derivation`, which records a verified
//! derivation as a ledger entry.

use std::collections::HashMap;

use crate::matcher::{self, Index, Part};
use crate::repo::{Cx, decode_plain, plain_encoding};
use crate::{Arg, Derivation, Error, Form, Id, Object, Put, Repo, Store, container, func};

/// Bound on nested container expansion (a zip inside a zip ...).
const MAX_UNPACK_DEPTH: usize = 4;

struct Candidate {
    cost: usize,
    derivation: Derivation,
    /// New literal content the derivation reads: (id, plain encoding, bytes).
    literal: Option<(Id, Vec<u8>, Vec<u8>)>,
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
        let mut best: Option<Candidate> = None;
        let mut offer = |c: Candidate| {
            if c.cost < best.as_ref().map_or(plain.len(), |b| b.cost) {
                best = Some(c);
            }
        };
        for d in hints {
            if self.verify(d, bytes, HashMap::new()) {
                offer(Candidate {
                    cost: derivation_size(d),
                    derivation: d.clone(),
                    literal: None,
                });
            }
        }
        if depth < MAX_UNPACK_DEPTH
            && let Some(c) = self.container_candidate(bytes, depth)?
        {
            offer(c);
        }
        if let Some(c) = self.slice_candidate(bytes)? {
            offer(c);
        }
        let Some(c) = best else {
            self.materialize(id, &plain, bytes)?;
            return Ok(Put {
                id,
                form: Form::Blob,
            });
        };
        if let Some((lit_id, encoded, lit)) = c.literal {
            self.materialize(lit_id, &encoded, &lit)?;
        }
        self.record(c.derivation)?;
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
        if !self.verify(&d, bytes, HashMap::new()) {
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

    fn verify(&self, d: &Derivation, bytes: &[u8], known: HashMap<Id, Vec<u8>>) -> bool {
        let mut cx = Cx::default();
        cx.known = known;
        d.output == Id::of_content(bytes)
            && self
                .eval(&d.func, &d.args, &mut cx)
                .is_ok_and(|out| out == bytes)
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
        if !self.verify(&d, bytes, HashMap::new()) {
            return Ok(None);
        }
        Ok(Some(Candidate {
            cost: derivation_size(&d),
            derivation: d,
            literal: None,
        }))
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
        let mut args: Vec<Arg> = Vec::new();
        let mut lit_pos = 0;
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
        if !self.verify(&d, bytes, HashMap::from([(lit_id, literal.clone())])) {
            return Ok(None);
        }
        let new_literal = !literal.is_empty() && !self.has_content(&lit_id)?;
        let literal = new_literal.then(|| (lit_id, plain_encoding(&literal), literal));
        let cost = derivation_size(&d) + literal.as_ref().map_or(0, |l| l.1.len());
        Ok(Some(Candidate {
            cost,
            derivation: d,
            literal,
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
