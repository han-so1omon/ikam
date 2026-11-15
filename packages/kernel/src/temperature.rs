//! Temperature by age (E263, opt-in, library only).
//!
//! The Python-era design tiered fragments hot, warm and cold by age; here
//! a tier is the read weight a content is planned at (E069: stored bytes
//! worth one decoded byte). New content is written at the repo's
//! `read_weight` (heavy, so it may stay raw and read fast); a temperature
//! pass re-encodes what has aged at a lighter weight, so it takes fewer
//! bytes and decodes slower.
//!
//! Age is commit depth (deterministic): a content is hot if the tree of
//! one of the newest `hot_commits` commits of some ref reaches it, walking
//! each ref's history breadth-first from its head; every other content
//! the refs reach is cold. `Repo::temperature_pass`:
//!
//! - re-encodes the cold contents not cooled before at `cold_read_weight`;
//! - re-encodes, at the repo's own `read_weight`, the contents cooled
//!   before that are hot again (promotion: a revert, or an old version
//!   copied back), so promotion happens only in a pass, never on read;
//! - keeps the cooled set as a plain content under `meta/cooled`, so the
//!   next pass touches only what changed temperature.
//!
//! Contents that a stored encoding reads as its group, dictionary or
//! context keep their encoding, as in the cold pass (E268): those readers
//! allow no chains. Every encoding is checked as `recompress` checks it,
//! so every file reads the same. A database's log reaches every event it
//! ever logged, so under commit depth its events never age; only the
//! graph chunks newer versions replaced do.

use std::collections::{HashSet, VecDeque};

use crate::{Error, Id, Object, Repo, Store};

/// The ref naming the contents a temperature pass left cooled.
pub const COOLED_REF: &str = "meta/cooled";

#[derive(Clone, Copy, Debug)]
pub struct TemperatureOpts {
    /// The newest commits of each ref whose trees are hot; 0 makes
    /// everything cold (the whole store at the cold weight).
    pub hot_commits: usize,
    /// The read weight cold contents are re-encoded at.
    pub cold_read_weight: f64,
}

/// What a temperature pass did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Temperature {
    /// Contents reached from the newest commits / from older ones only.
    pub hot: usize,
    pub cold: usize,
    /// Cold contents re-encoded now, and cooled ones made hot again.
    pub cooled: usize,
    pub promoted: usize,
    /// Stored bytes of the cooled and promoted contents, before and after.
    pub before: usize,
    pub after: usize,
}

impl<S: Store + Sync> Repo<S> {
    /// The newest `n` commits of each ref (not under `meta/`), breadth-first
    /// from its head.
    fn newest_commits(&self, n: usize) -> Result<Vec<Id>, Error> {
        let mut out = HashSet::new();
        for (name, head) in self.store.refs()? {
            if name.starts_with("meta/") {
                continue;
            }
            let (mut queue, mut seen) = (VecDeque::from([head]), HashSet::from([head]));
            let mut taken = 0;
            while let Some(id) = queue.pop_front() {
                if taken == n {
                    break;
                }
                let Ok(Object::Commit(c)) = self.get(&id) else {
                    continue;
                };
                out.insert(id);
                taken += 1;
                for p in c.parents {
                    if seen.insert(p) {
                        queue.push_back(p);
                    }
                }
            }
        }
        let mut v: Vec<Id> = out.into_iter().collect();
        v.sort();
        Ok(v)
    }

    /// Contents a set of roots reaches, with what their derivations read.
    fn contents_from(&self, roots: Vec<Id>) -> Result<HashSet<Id>, Error> {
        let (_, files) = self.reachable_from(roots, &HashSet::new())?;
        Ok(self.closure(files, |_| true)?.0)
    }

    fn cooled_list(&self) -> Result<HashSet<Id>, Error> {
        Ok(match self.store.get_ref(COOLED_REF)? {
            Some(marker) => self
                .read_content(&marker)?
                .chunks_exact(32)
                .map(|c| Id::from_bytes(c.try_into().unwrap()))
                .collect(),
            None => HashSet::new(),
        })
    }

    /// Re-encode aged contents at `cold_read_weight` and promote the ones
    /// that are hot again. See the module docs.
    pub fn temperature_pass(&mut self, opts: TemperatureOpts) -> Result<Temperature, Error> {
        let mut roots: Vec<Id> = self
            .store
            .refs()?
            .into_iter()
            .filter(|(name, _)| !name.starts_with("meta/"))
            .map(|(_, id)| id)
            .collect();
        roots.sort();
        roots.dedup();
        let hot_trees: Vec<Id> = self
            .newest_commits(opts.hot_commits)?
            .iter()
            .map(|c| self.commit_tree(c))
            .collect::<Result<_, _>>()?;
        let hot = self.contents_from(hot_trees)?;
        let all = self.contents_from(roots.clone())?;
        let cold: HashSet<Id> = all.difference(&hot).copied().collect();
        let was = self.cooled_list()?;
        // Only stored contents: a derived content without bytes is skipped.
        let stored_here = |r: &Self, ids: &mut dyn Iterator<Item = Id>| -> HashSet<Id> {
            ids.filter(|id| r.store.has(id)).collect()
        };
        let cool = stored_here(self, &mut cold.difference(&was).copied());
        let promote = stored_here(self, &mut was.intersection(&hot).copied());
        let mut needed = crate::dict::Needed::default();
        for id in self.store.ids()? {
            needed.add(&self.store.read(&id)?);
        }
        let keep: HashSet<Id> = cool
            .iter()
            .chain(&promote)
            .filter(|id| needed.names(id))
            .copied()
            .collect();
        let size = |r: &Self, ids: &HashSet<Id>| -> usize {
            ids.iter()
                .filter_map(|id| r.store.read(id).ok().map(|e| e.len()))
                .sum()
        };
        let before = size(self, &cool) + size(self, &promote);
        self.store.sync()?;
        let heavy = self.read_weight;
        self.read_weight = opts.cold_read_weight;
        let cooled = self.recompress_only(&roots, Some(&cool), &keep);
        self.read_weight = heavy;
        cooled?;
        // A cooled content may be a frame against another (no chains, so
        // recompress leaves frames alone): write it plain first.
        for id in promote.difference(&keep) {
            let bytes = self.read_content(id)?;
            let plain = self.encode_plain(&bytes);
            self.store.replace(*id, &plain)?;
        }
        self.recompress_only(&roots, Some(&promote), &keep)?;
        let after = size(self, &cool) + size(self, &promote);
        // Cooled now: what was, less what came back, plus what aged now
        // (a kept content stays as it was, and is listed as it is).
        let mut list: Vec<Id> = was
            .iter()
            .filter(|id| !promote.contains(id) || keep.contains(id))
            .chain(cool.iter().filter(|id| !keep.contains(id)))
            .filter(|id| all.contains(id))
            .copied()
            .collect();
        list.sort();
        list.dedup();
        let bytes: Vec<u8> = list.iter().flat_map(|id| *id.as_bytes()).collect();
        let marker = Id::of_content(&bytes);
        let encoded = self.encode_plain(&bytes);
        self.store.write(marker, &encoded)?;
        let current = self.store.get_ref(COOLED_REF)?;
        self.store.set_ref(COOLED_REF, current, marker)?;
        Ok(Temperature {
            hot: hot.len(),
            cold: cold.len(),
            cooled: cool.len() - cool.intersection(&keep).count(),
            promoted: promote.len() - promote.intersection(&keep).count(),
            before,
            after,
        })
    }
}
