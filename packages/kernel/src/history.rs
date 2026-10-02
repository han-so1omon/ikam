//! Commits, refs, history and garbage collection.

use std::collections::HashSet;

use crate::{Commit, Error, Id, Object, Repo, Store};

impl<S: Store> Repo<S> {
    /// Advance `name` to a new commit of `tree` whose parent is the ref's
    /// current value. Fails if the ref moved concurrently.
    pub fn commit(&mut self, name: &str, tree: Id, message: &str) -> Result<Id, Error> {
        let parent = self.store.get_ref(name)?;
        let commit = Commit {
            tree,
            parents: parent.into_iter().collect(),
            message: message.into(),
        };
        let id = self.put(&Object::Commit(commit))?;
        self.store.set_ref(name, parent, id)?;
        Ok(id)
    }

    /// Resolve a ref name or a full hex id.
    pub fn resolve(&self, rev: &str) -> Result<Id, Error> {
        match rev.parse() {
            Ok(id) => Ok(id),
            Err(_) => self
                .store
                .get_ref(rev)?
                .ok_or_else(|| Error::InvalidName(rev.into())),
        }
    }

    /// First-parent history starting at `id`.
    pub fn log(&self, mut id: Id) -> Result<Vec<(Id, Commit)>, Error> {
        let mut out = Vec::new();
        loop {
            let Object::Commit(c) = self.get(&id)? else {
                return Err(Error::WrongKind(id));
            };
            let parent = c.parents.first().copied();
            out.push((id, c));
            match parent {
                Some(p) => id = p,
                None => return Ok(out),
            }
        }
    }

    /// Re-verify every stored object: content is fully reconstructed and
    /// hash-checked, other objects are hash-checked. Run this after upgrading
    /// dependencies that reconstruction relies on. Returns the failures.
    pub fn fsck(&self) -> Result<Vec<(Id, Error)>, Error> {
        let mut failures = Vec::new();
        for id in self.store.ids()? {
            let content = matches!(
                self.store.read(&id)?.first(),
                Some(b'B' | b'Z' | b'R' | b'A')
            );
            let result = if content {
                self.read_content(&id).map(drop)
            } else {
                self.get(&id).map(drop)
            };
            if let Err(e) = result {
                failures.push((id, e));
            }
        }
        Ok(failures)
    }

    /// Delete every object that no ref reaches. Run records are kept while
    /// their output is kept, so live content retains its provenance.
    /// Returns the number of objects deleted.
    pub fn gc(&mut self) -> Result<usize, Error> {
        let all = self.store.ids()?;
        let runs: Vec<(Id, Id)> = all
            .iter()
            .filter_map(|id| match self.get(id) {
                Ok(Object::Run(run)) => Some((*id, run.output)),
                _ => None,
            })
            .collect();
        let mut live = HashSet::new();
        let mut stack: Vec<Id> = self.store.refs()?.into_iter().map(|(_, id)| id).collect();
        loop {
            while let Some(id) = stack.pop() {
                if self.store.has(&id) && live.insert(id) {
                    stack.extend(self.links(&id)?.into_iter().map(|(_, target)| target));
                }
            }
            stack.extend(
                runs.iter()
                    .filter(|(run, out)| live.contains(out) && !live.contains(run))
                    .map(|(run, _)| *run),
            );
            if stack.is_empty() {
                break;
            }
        }
        let dead: Vec<&Id> = all.iter().filter(|id| !live.contains(*id)).collect();
        for id in &dead {
            self.store.delete(id)?;
        }
        self.index = None;
        self.runs = None;
        Ok(dead.len())
    }
}
