//! Repack: re-plan the storage of all live content at once.
//!
//! Ingest decides one file at a time, so the result depends on arrival
//! order. Repack re-ingests live content into a fresh in-memory store,
//! largest first, re-offering every recorded derivation as a candidate. It
//! then drops stored bytes wherever a recorded derivation can rebuild them
//! without them, largest first.
//!
//! `concat` derivations are storage plans (which bytes to reuse) and are
//! replaced by the re-plan; the literal blobs they read are not kept for
//! their own sake. Every other derivation (functions, container packing,
//! proposals recorded through `put_derivation`) is a fact: it is
//! re-recorded, and its inputs are kept. The fresh store is verified completely, and
//! it replaces the old representation only if it is smaller. Ids never
//! change, so trees, commits and refs are untouched.

use std::cmp::Reverse;
use std::collections::HashSet;

use crate::repo::{Cx, decode_plain};
use crate::{Error, Id, MemStore, Repo, Store, func};

#[derive(Debug, PartialEq, Eq)]
pub struct Repacked {
    /// Bytes of content and derivation objects before and after.
    pub before: usize,
    pub after: usize,
    /// False when the re-plan was not smaller and nothing changed.
    pub applied: bool,
}

impl<S: Store> Repo<S> {
    pub fn repack(&mut self) -> Result<Repacked, Error> {
        self.gc()?;
        let (_, files) = self.reachable()?;
        let (content, _) = self.closure(files, |d| d.func != func::concat())?;
        let mut files: Vec<(Id, Vec<u8>)> = content
            .iter()
            .map(|id| Ok((*id, self.read_content(id)?)))
            .collect::<Result<_, Error>>()?;
        files.sort_by_key(|(id, bytes)| (Reverse(bytes.len()), *id));

        let mut fresh = Repo::new(MemStore::default());
        for (id, bytes) in &files {
            fresh.put_content_with(bytes, 0, &self.derivations(id)?)?;
        }
        // Re-record facts whose inputs survived the re-plan.
        for (id, _) in &files {
            for d in self.derivations(id)? {
                if d.func != func::concat()
                    && d.args
                        .iter()
                        .all(|a| fresh.has_content(&a.id()).unwrap_or(false))
                {
                    fresh.record(d)?;
                }
            }
        }
        fresh.drop_derivable_bytes(&files)?;
        for (id, bytes) in &files {
            if fresh.read_content(id)? != *bytes {
                return Err(Error::Corrupt(*id));
            }
        }
        if !fresh.fsck()?.is_empty() {
            return Err(Error::Exec("repack: re-plan failed verification".into()));
        }

        let before = self.content_bytes()?;
        let after = fresh.content_bytes()?;
        if after >= before {
            return Ok(Repacked {
                before,
                after: before,
                applied: false,
            });
        }
        let keep: HashSet<Id> = fresh.store.ids()?.into_iter().collect();
        for id in &keep {
            self.store.write(*id, &fresh.store.read(id)?)?;
        }
        for id in self.store.ids()? {
            if !keep.contains(&id) && self.is_content_or_derivation(&id)? {
                self.store.delete(&id)?;
            }
        }
        self.reset_projections();
        Ok(Repacked {
            before,
            after,
            applied: true,
        })
    }

    /// Delete stored bytes of content that a recorded derivation rebuilds
    /// without them, largest first. The derivation is already recorded, so
    /// each deletion saves the full stored size. Each deletion is checked against
    /// the current state, so everything stays reconstructible.
    fn drop_derivable_bytes(&mut self, files: &[(Id, Vec<u8>)]) -> Result<(), Error> {
        for (id, bytes) in files {
            if !self.store.has(id) {
                continue;
            }
            for d in self.derivations(id)? {
                let mut cx = Cx::default();
                cx.stack_guard(*id); // the derivation may not read `id` itself
                let rebuilt = self.eval(&d.func, &d.args, &mut cx);
                if rebuilt.as_ref().is_ok_and(|b| b == bytes) {
                    self.store.delete(id)?;
                    break;
                }
            }
        }
        self.reset_projections();
        Ok(())
    }

    fn is_content_or_derivation(&self, id: &Id) -> Result<bool, Error> {
        let encoded = self.store.read(id)?;
        Ok(decode_plain(id, &encoded).is_ok() || encoded.first() == Some(&b'D'))
    }

    fn content_bytes(&self) -> Result<usize, Error> {
        let mut total = 0;
        for id in self.store.ids()? {
            if self.is_content_or_derivation(&id)? {
                total += self.store.read(&id)?.len();
            }
        }
        Ok(total)
    }
}
