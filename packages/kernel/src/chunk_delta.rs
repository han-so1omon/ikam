//! E448: a new version of a tree or graph chunk stored as a zstd frame
//! against the stored chunk it most resembles (opt-in, `Repo::object_contexts`).
//!
//! A tree or graph chunk is stored as content (`g 2 content`, `dict.rs`),
//! so a new version is already planned as slices of earlier ones: a
//! `concat` record (two 32-byte ids, then a source id and a range for
//! every piece) and a literal content holding the bytes that changed. For
//! a chunk with one changed node that is about 170 bytes of record and
//! literal overhead around 40 to 80 bytes of change, paid again by every
//! chunk on the path to the root. The store already has an encoding that
//! names one base and holds only what differs from it: "Q", a zstd frame
//! with a related content as its context, which repack writes (E057). Here
//! ingest may write it too, for object contents, at the moment a version
//! is stored: the base is the most related stored content the seed index
//! finds, and the frame competes on the planner's own score (bytes plus
//! `read_weight` times decode work) against the slice and template plans
//! and the plain encoding.
//!
//! Chains stay bounded as they are for "Q": a base is always stored in a
//! plain encoding that is neither "Q", "M" nor "S", so a read decodes at
//! most one base. When the most related content is itself a frame, its
//! own base is tried in its place (the version the frames of a hot chunk
//! share), so a chunk edited again and again is framed against the last
//! version stored whole until the frame stops paying, and then stored
//! whole itself and becomes the next base. gc keeps a frame's base (`needs`
//! names it), the drop of a stored copy at ingest checks it (`is_needed`),
//! and repack re-plans frames like any other encoding.

use crate::dict::{contexts, zstd_frame};
use crate::{Error, Id, Repo, Store, matcher};

/// Related contents tried as a base, most related first.
const BASES: usize = 3;
/// Contents shorter than this are never framed: the 33 bytes of tag and
/// base id leave nothing to save.
const FRAME_MIN: usize = 256;

impl<S: Store> Repo<S> {
    /// The best "Q" encoding of an object's content `bytes` against a
    /// related stored content, with its planner score, if one exists.
    pub(crate) fn object_context_frame(
        &mut self,
        bytes: &[u8],
    ) -> Result<Option<(Vec<u8>, f64)>, Error> {
        if bytes.len() < FRAME_MIN {
            return Ok(None);
        }
        self.ensure_index()?;
        let (near, _) = matcher::neighbors(bytes, self.index.as_ref().unwrap(), BASES);
        let mut bases: Vec<Id> = Vec::new();
        for (n, _) in near {
            let Ok(stored) = self.store.read(&n) else {
                continue; // derived: not a base
            };
            let candidate = match stored.first() {
                // A frame's own base: the version its hot chunk shares.
                Some(b'Q') => contexts(&stored)
                    .first()
                    .and_then(|c| <[u8; 32]>::try_from(*c).ok())
                    .map(Id::from_bytes),
                Some(b'M' | b'S') => None,
                _ => Some(n),
            };
            if let Some(b) = candidate
                && !bases.contains(&b)
            {
                bases.push(b);
            }
        }
        let mut best: Option<(Vec<u8>, f64)> = None;
        for base in bases {
            let Ok(context) = self.related_base(&base) else {
                continue;
            };
            // Decode work: the frame's output and its base, both zstd.
            let work = (bytes.len() + context.len()) as f64;
            if self.read_weight * work >= best.as_ref().map_or(f64::MAX, |b| b.1) {
                continue;
            }
            let Some(z) = zstd_frame(bytes, &context, self.level) else {
                continue;
            };
            let q = [&b"Q"[..], base.as_bytes(), &z].concat();
            let score = q.len() as f64 + self.read_weight * work;
            if best.as_ref().is_none_or(|b| score < b.1) {
                best = Some((q, score));
            }
        }
        Ok(best)
    }
}
