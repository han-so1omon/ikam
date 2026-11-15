//! An incremental cold pass (E268, opt-in, library only).
//!
//! `Repo::repack` re-plans every live content from scratch, several times
//! (with and without a dictionary, with and without template induction),
//! so its cost grows with the store, not with what changed: E260 measured
//! 29 to 87 s on 2 MB stores and no finish in 12 minutes on 20 MB.
//! `Repo::repack_incremental` does the cold work only for what is new
//! since the last pass:
//!
//! - New is what the refs reach now and did not reach at the last pass:
//!   objects (commits, trees, graph chunks) and contents with what their
//!   derivations read. The refs of the last pass are kept as a plain
//!   content under `meta/cold-pass`, so a pass reads the old history once
//!   to know what to skip. Refs under `meta/` are not roots.
//! - The new contents are re-encoded as `recompress` does (the store's
//!   level, the store's dictionary, another new content as a context when
//!   that is cheaper), and new small contents are grouped as
//!   `group_small` does when `ColdPassOpts::group` is on (off by default). Nothing old is touched,
//!   no dictionary is trained and no templates are induced: those stay
//!   with the full repack. A new content that some stored encoding
//!   already reads as its group, dictionary or context (an ingest's
//!   context frame, an earlier pass's group) keeps its encoding too,
//!   since those readers allow no chains.
//!
//! Ids never change and every encoding is checked as in `recompress`, so
//! every file reads the same; what only the full repack would find
//! (a better dictionary, templates across old and new files) is left on
//! the table, and E268 measures how much. Without a marker (a first
//! pass), everything live is new.

use std::collections::HashSet;

use crate::{Error, Id, Repo, Store};

/// The ref naming the roots of the last cold pass.
pub const COLD_PASS_REF: &str = "meta/cold-pass";

/// How a cold pass encodes what is new.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColdPassOpts {
    /// Group new small contents (as `group_small` does in a full repack).
    /// Fewer bytes (E268: 34% fewer on generated files, 13 to 24% fewer
    /// on databases), but a read of a member decodes its group up to the
    /// member: database scans read 4 to 13 times slower and appends that
    /// encode against grouped rows slow down too. Default off.
    pub group: bool,
}

/// What a cold pass looked at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColdPass {
    /// Objects and contents new since the last pass.
    pub new: usize,
    /// Stored bytes of those that still exist, before and after the pass
    /// (groups the pass made are counted after).
    pub before: usize,
    pub after: usize,
}

impl<S: Store + Sync> Repo<S> {
    /// Re-encode and group what is new since the last cold pass, then mark
    /// the refs as passed. See the module docs.
    pub fn repack_incremental(&mut self) -> Result<ColdPass, Error> {
        self.repack_incremental_with(ColdPassOpts::default())
    }

    /// `repack_incremental` with options.
    pub fn repack_incremental_with(&mut self, opts: ColdPassOpts) -> Result<ColdPass, Error> {
        let mut roots: Vec<Id> = self
            .store
            .refs()?
            .into_iter()
            .filter(|(name, _)| !name.starts_with("meta/"))
            .map(|(_, id)| id)
            .collect();
        roots.sort();
        roots.dedup();
        let previous = match self.store.get_ref(COLD_PASS_REF)? {
            Some(marker) => self
                .read_content(&marker)
                .map(|b| {
                    b.chunks_exact(32)
                        .map(|c| Id::from_bytes(c.try_into().unwrap()))
                        .collect()
                })
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let (old_objects, old_files) = self.reachable_from(previous, &HashSet::new())?;
        let (old_content, old_records) = self.closure(old_files, |_| true)?;
        let (objects, files) = self.reachable_from(roots.clone(), &old_objects)?;
        let mut file_roots: Vec<Id> = files
            .iter()
            .filter(|f| !old_content.contains(f))
            .copied()
            .collect();
        file_roots.sort();
        file_roots.dedup();
        let (content, records) = self.closure(file_roots.clone(), |_| true)?;
        let new: HashSet<Id> = objects
            .into_iter()
            .chain(content)
            .chain(records)
            .filter(|id| !old_content.contains(id) && !old_records.contains(id))
            .collect();
        let stored = |repo: &Self, ids: &mut dyn Iterator<Item = Id>| -> usize {
            ids.filter_map(|id| repo.store.read(&id).ok().map(|e| e.len()))
                .sum()
        };
        // What the pass encodes against (contexts, groups' members) must be
        // durable before an encoding that needs it replaces another.
        self.store.sync()?;
        let before = stored(self, &mut new.iter().copied());
        let ids_before: HashSet<Id> = self.store.ids()?.into_iter().collect();
        // A new content that a stored encoding already reads as its group,
        // dictionary or context (an ingest's context frame, or an earlier
        // pass's group) keeps its encoding: those readers allow no chains.
        let mut needed = crate::dict::Needed::default();
        for id in self.store.ids()? {
            needed.add(&self.store.read(&id)?);
        }
        let keep: HashSet<Id> = new.iter().filter(|id| needed.names(id)).copied().collect();
        let editable: HashSet<Id> = new.difference(&keep).copied().collect();
        // As in a full re-plan: group small contents first, then recompress
        // the new contents and the groups just made. The other order can
        // group a content that a context encoding already reads as its base.
        if opts.group {
            self.group_small_only(Some(&editable))?;
        }
        let made: Vec<Id> = self
            .store
            .ids()?
            .into_iter()
            .filter(|id| !ids_before.contains(id))
            .collect();
        let mut fresh = new.clone();
        fresh.extend(made.iter().copied());
        self.recompress_only(&file_roots, Some(&fresh), &keep)?;
        let after = stored(self, &mut new.iter().copied().chain(made));
        let list: Vec<u8> = roots.iter().flat_map(|id| *id.as_bytes()).collect();
        let marker = Id::of_content(&list);
        let encoded = self.encode_plain(&list);
        self.store.write(marker, &encoded)?;
        let current = self.store.get_ref(COLD_PASS_REF)?;
        self.store.set_ref(COLD_PASS_REF, current, marker)?;
        Ok(ColdPass {
            new: new.len(),
            before,
            after,
        })
    }
}
