//! Drafts as branches (E266, opt-in, library only).
//!
//! The Python-era staging model wrote drafts to a staging area that stayed
//! invisible until promoted, atomically and with everything it needed, and
//! was discarded on failure. Here a draft is a ref under `drafts/`, made at
//! a target branch's head:
//!
//! - Edits on a draft are ordinary commits on its ref, so nothing on the
//!   target changes and readers of the target never see them.
//! - `Repo::promote_draft` merges the draft into its target (`Repo::merge`: a
//!   fast-forward when the target has not moved, otherwise a merge commit)
//!   and then deletes the draft's ref. A conflict writes nothing and leaves
//!   both refs as they were.
//! - `Repo::discard_draft` deletes the draft's ref. What only the draft reached
//!   is then unreachable, and the next `gc` removes it.
//!
//! Refs move by compare-and-swap, so a draft promoted or discarded by
//! another writer meanwhile is refused, not lost. A promote interrupted
//! between moving the target and deleting the draft leaves a draft whose
//! head is already in the target: promoting it again changes nothing and
//! deletes it. Needs a store that can delete refs (`Store::delete_ref`).

use crate::merge::Merged;
use crate::{Error, Id, Repo, Store};

/// The prefix of every draft's ref.
pub const DRAFTS: &str = "drafts/";

impl<S: Store> Repo<S> {
    /// Open a draft `name` at `target`'s head. Returns its ref name. Fails
    /// if a draft of that name exists.
    pub fn draft(&mut self, target: &str, name: &str) -> Result<String, Error> {
        let head = self.resolve(target)?;
        let draft = format!("{DRAFTS}{name}");
        self.store.set_ref(&draft, None, head)?;
        Ok(draft)
    }

    /// Every open draft and its head.
    pub fn drafts(&self) -> Result<Vec<(String, Id)>, Error> {
        let mut out: Vec<(String, Id)> = self
            .store
            .refs()?
            .into_iter()
            .filter(|(n, _)| n.starts_with(DRAFTS))
            .collect();
        out.sort();
        Ok(out)
    }

    /// Merge `draft` into `target` and delete the draft's ref, unless the
    /// merge reports conflicts (then nothing changes).
    pub fn promote_draft(
        &mut self,
        target: &str,
        draft: &str,
        message: &str,
    ) -> Result<Merged, Error> {
        let head = self.draft_head(draft)?;
        let merged = self.merge(target, &head.to_string(), message)?;
        if !matches!(merged, Merged::Conflicts(_)) {
            self.store.delete_ref(draft, head)?;
        }
        Ok(merged)
    }

    /// Delete `draft`'s ref. Its own objects stay until the next `gc`.
    pub fn discard_draft(&mut self, draft: &str) -> Result<(), Error> {
        let head = self.draft_head(draft)?;
        self.store.delete_ref(draft, head)
    }

    fn draft_head(&self, draft: &str) -> Result<Id, Error> {
        if !draft.starts_with(DRAFTS) {
            return Err(Error::InvalidName(draft.into()));
        }
        self.store
            .get_ref(draft)?
            .ok_or_else(|| Error::InvalidName(draft.into()))
    }
}
