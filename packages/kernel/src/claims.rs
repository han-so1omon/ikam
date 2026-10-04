//! Claims: unverified semantic relations, weighted by measured information.
//!
//! A claim says `subject predicate object` about two selections of content.
//! Its weight is measured, never asserted: the bits saved compressing the
//! subject with the object as a zstd dictionary, against compressing it
//! alone. That is a computable stand-in for conditional information. It is
//! zero for unrelated content, grows with shared structure and wording, and
//! it is exactly the kind of saving dedup can try to realize. `promote`
//! tries to turn a claim into a verified derivation of the subject from the
//! object, and keeps it only if it is smaller than what is stored now.

use std::collections::HashMap;

use crate::matcher::{self, Index};
use crate::{Arg, Claim, Error, Form, Id, Object, Put, Repo, Store};

impl<S: Store> Repo<S> {
    /// The bytes a selector denotes.
    pub fn select(&self, a: &Arg) -> Result<Vec<u8>, Error> {
        let bytes = self.read_content(&a.id())?;
        match *a {
            Arg::Whole(_) => Ok(bytes),
            Arg::Range { id, start, len } => usize::try_from(start)
                .ok()
                .zip(usize::try_from(start.saturating_add(len)).ok())
                .and_then(|(s, e)| bytes.get(s..e))
                .map(<[u8]>::to_vec)
                .ok_or_else(|| Error::Exec(format!("{id}: range {start}+{len} out of bounds"))),
        }
    }

    /// Bits saved compressing `subject` when `object` is known.
    pub fn gain_bits(&self, subject: &Arg, object: &Arg) -> Result<i64, Error> {
        Ok(conditional_gain(
            &self.select(subject)?,
            &self.select(object)?,
        ))
    }

    /// Record a claim; its weight is measured here. Returns the claim's id.
    pub fn claim(
        &mut self,
        subject: Arg,
        predicate: &str,
        object: Arg,
        by: Option<Id>,
    ) -> Result<Id, Error> {
        let gain_bits = self.gain_bits(&subject, &object)?;
        let encoded = Object::Claim(Claim {
            subject,
            predicate: predicate.into(),
            object,
            gain_bits,
            by,
        })
        .encode();
        let id = Id::of(&encoded);
        self.write(id, &encoded)?;
        Ok(id)
    }

    /// Every claim whose subject or object is `id`. A full scan.
    pub fn claims(&self, id: &Id) -> Result<Vec<(Id, Claim)>, Error> {
        let mut out = Vec::new();
        for cid in self.store.ids()? {
            if self.store.read(&cid)?.first() == Some(&b'L')
                && let Object::Claim(c) = self.get(&cid)?
                && (c.subject.id() == *id || c.object.id() == *id)
            {
                out.push((cid, c));
            }
        }
        Ok(out)
    }

    /// Up to `k` stored contents most informative about `id`, with measured
    /// gains, best first. Candidates come from shared seeds. Internal
    /// fragments (literal blobs, templates, fillers) are content too and may
    /// appear.
    pub fn relate(&mut self, id: &Id, k: usize) -> Result<Vec<(Id, i64)>, Error> {
        let bytes = self.read_content(id)?;
        self.ensure_index()?;
        let (near, _) = matcher::neighbors(&bytes, self.index.as_ref().unwrap(), k + 1);
        let mut out = Vec::new();
        for (n, _) in near.into_iter().filter(|(n, _)| n != id) {
            if let Ok(other) = self.read_content(&n) {
                out.push((n, conditional_gain(&bytes, &other)));
            }
        }
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out.truncate(k);
        Ok(out)
    }

    /// Turn a claim into storage: derive its subject (a whole content) from
    /// its object by reusing the object's bytes, verified, and keep it only
    /// if that is smaller than the subject's stored bytes. Returns the
    /// derived content, or `None` if nothing smaller was found.
    pub fn promote(&mut self, claim: &Id) -> Result<Option<Put>, Error> {
        let Object::Claim(c) = self.get(claim)? else {
            return Err(Error::WrongKind(*claim));
        };
        let (Arg::Whole(x), Arg::Whole(y)) = (c.subject, c.object) else {
            return Ok(None);
        };
        let Ok(stored) = self.store.read(&x) else {
            return Ok(None);
        }; // already derived only
        let (x_bytes, y_bytes) = (self.read_content(&x)?, self.read_content(&y)?);
        let mut index = Index::default();
        index.add(y, &y_bytes);
        let sources = HashMap::from([(y, y_bytes)]);
        let plan = matcher::plan(&x_bytes, &index, |id| sources.get(id).cloned());
        let Some(mut candidate) = self.concat_candidate(&x_bytes, &plan)? else {
            return Ok(None);
        };
        let known = candidate
            .literals
            .iter()
            .map(|(id, _, b)| (*id, b.clone()))
            .collect();
        // Dropping x is safe only if the derivation never needs x itself,
        // e.g. when the object is in turn derived from the subject.
        if candidate.cost >= stored.len() as isize
            || !self.verify_without(&candidate.derivations[0], &x_bytes, &known, Some(x))
        {
            return Ok(None);
        }
        candidate.drop = Some(x);
        self.apply_candidate(candidate)?;
        Ok(Some(Put {
            id: x,
            form: Form::Derived,
        }))
    }
}

/// Bits saved compressing `subject` with `object` as a raw-content zstd
/// dictionary, against compressing it alone (level 3 both ways).
pub(crate) fn conditional_gain(subject: &[u8], object: &[u8]) -> i64 {
    let alone = zstd::bulk::compress(subject, 3).map_or(subject.len(), |z| z.len());
    if object.is_empty() {
        return 0;
    }
    let given = zstd::bulk::Compressor::with_dictionary(3, object)
        .and_then(|mut c| c.compress(subject))
        .map_or(alone, |z| z.len());
    (alone as i64 - given as i64) * 8
}
