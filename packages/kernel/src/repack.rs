//! Repack: re-plan the storage of all live content at once.
//!
//! Ingest decides one file at a time, so the result depends on arrival
//! order and cannot see that a template will pay off over many future
//! files. Repack re-ingests live content into a fresh in-memory store,
//! largest first, re-offering every recorded derivation as a candidate, then
//! drops stored bytes wherever a recorded derivation rebuilds them. It does
//! this twice, with and without template induction (one template per
//! cluster of similar contents, generalized over all of them), and keeps the
//! smaller plan. The plan is verified completely and replaces the old
//! representation only if it is smaller. Ids never change, so trees, commits
//! and refs are untouched.
//!
//! `concat` and `fill` derivations are storage plans and are replaced by the
//! re-plan; the literals, templates and fillers they read are not kept for
//! their own sake. Every other derivation (functions, container packing,
//! proposals recorded through `put_derivation`) is a fact: it is
//! re-recorded, and its inputs are kept.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};

use crate::matcher::{self, Index};
use crate::repo::{Cx, decode_plain};
use crate::{Arg, Derivation, Error, Id, MemStore, Repo, Store, func, template};

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
        let (_, roots) = self.reachable()?;
        let (content, _) = self.closure(roots, |d| !func::is_plan(&d.func))?;
        let mut files: Vec<(Id, Vec<u8>)> = content
            .iter()
            .map(|id| Ok((*id, self.read_content(id)?)))
            .collect::<Result<_, Error>>()?;
        files.sort_by_key(|(id, bytes)| (Reverse(bytes.len()), *id));

        let plain = self.replan(&files, false)?;
        let templated = self.replan(&files, true)?;
        let fresh = if templated.content_bytes()? < plain.content_bytes()? {
            templated
        } else {
            plain
        };
        let (before, after) = (self.content_bytes()?, fresh.content_bytes()?);
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

    /// A verified fresh store holding exactly `files` and what they need.
    fn replan(&self, files: &[(Id, Vec<u8>)], induce: bool) -> Result<Repo<MemStore>, Error> {
        let mut fresh = Repo::new(MemStore::default());
        let mut hints: HashMap<Id, Vec<Derivation>> = HashMap::new();
        for (id, _) in files {
            hints.insert(*id, self.derivations(id)?);
        }
        if induce {
            for group in template_groups(files) {
                let members: Vec<&[u8]> = group.iter().map(|&i| files[i].1.as_slice()).collect();
                let Some(t) = template::induce(&members) else {
                    continue;
                };
                let t_id = fresh.put_content(&t)?.id;
                for &i in &group {
                    if let Some(fillers) = template::fit(&t, &files[i].1) {
                        let f_id = fresh.put_content(&fillers)?.id;
                        let d = Derivation {
                            output: files[i].0,
                            func: func::fill(),
                            args: vec![Arg::Whole(t_id), Arg::Whole(f_id)],
                        };
                        hints.entry(files[i].0).or_default().push(d);
                    }
                }
            }
        }
        for (id, bytes) in files {
            fresh.put_content_with(bytes, 0, &hints[id])?;
        }
        // Re-record facts whose inputs survived the re-plan.
        for (id, _) in files {
            for d in self.derivations(id)? {
                if !func::is_plan(&d.func)
                    && d.args
                        .iter()
                        .all(|a| fresh.has_content(&a.id()).unwrap_or(false))
                {
                    fresh.record(d)?;
                }
            }
        }
        fresh.drop_derivable_bytes(files)?;
        fresh.prune(files)?;
        for (id, bytes) in files {
            if fresh.read_content(id)? != *bytes {
                return Err(Error::Corrupt(*id));
            }
        }
        if !fresh.fsck()?.is_empty() {
            return Err(Error::Exec("repack: re-plan failed verification".into()));
        }
        Ok(fresh)
    }

    /// Delete everything `files` do not need (unused templates, fillers,
    /// literals and container members left over from rejected candidates).
    fn prune(&mut self, files: &[(Id, Vec<u8>)]) -> Result<(), Error> {
        let (content, records) =
            self.closure(files.iter().map(|(id, _)| *id).collect(), |_| true)?;
        for id in self.store.ids()? {
            if !content.contains(&id) && !records.contains(&id) {
                self.store.delete(&id)?;
            }
        }
        self.reset_projections();
        Ok(())
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

    pub(crate) fn content_bytes(&self) -> Result<usize, Error> {
        let mut total = 0;
        for id in self.store.ids()? {
            if self.is_content_or_derivation(&id)? {
                total += self.store.read(&id)?.len();
            }
        }
        Ok(total)
    }
}

/// Clusters (indices into `files`) of contents linked to their most similar
/// other content, when the two are `matcher::similar`.
fn template_groups(files: &[(Id, Vec<u8>)]) -> Vec<Vec<usize>> {
    let mut index = Index::default();
    for (id, bytes) in files {
        index.add(*id, bytes);
    }
    let position: HashMap<Id, usize> = files
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i))
        .collect();
    let mut parent: Vec<usize> = (0..files.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for (i, (id, bytes)) in files.iter().enumerate() {
        let (near, seeds) = matcher::neighbors(bytes, &index, 2);
        if let Some(j) = near
            .iter()
            .filter(|(n, shared)| n != id && matcher::similar(*shared, seeds))
            .find_map(|(n, _)| position.get(n))
        {
            let (a, b) = (root(&mut parent, i), root(&mut parent, *j));
            parent[a] = b;
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..files.len() {
        groups.entry(root(&mut parent, i)).or_default().push(i);
    }
    let mut groups: Vec<Vec<usize>> = groups.into_values().filter(|g| g.len() >= 2).collect();
    groups.sort();
    groups
}
