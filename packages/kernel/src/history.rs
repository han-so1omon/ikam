//! Commits, refs, history, garbage collection and verification.

use std::collections::HashSet;

use crate::dict::dict_of;
use crate::repo::Cx;
use crate::{Arg, Commit, Derivation, Error, Id, Kind, Object, Repo, Store, func};

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

    /// Re-verify every stored object: stored content against its id,
    /// trees/commits/derivations against their hash, and every derivation by
    /// evaluating it. Run after upgrading dependencies that reconstruction
    /// relies on. Returns the failing object ids.
    pub fn fsck(&self) -> Result<Vec<(Id, Error)>, Error> {
        let mut failures = Vec::new();
        for id in self.store.ids()? {
            let encoded = self.store.read(&id)?;
            let result = match self.decode_plain(&id, &encoded) {
                Ok(bytes) if Id::of_content(&bytes) == id => Ok(()),
                Ok(_) => Err(Error::Corrupt(id)),
                Err(Error::WrongKind(_)) => self.check_object(&id),
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                failures.push((id, e));
            }
        }
        Ok(failures)
    }

    fn check_object(&self, id: &Id) -> Result<(), Error> {
        if let Object::Derivation(d) = self.get(id)? {
            let out = self.eval(&d.func, &d.args, &mut Cx::default())?;
            if Id::of_content(&out) != d.output {
                return Err(Error::Corrupt(*id));
            }
        }
        Ok(())
    }

    /// Ids reachable from refs: tree/commit objects, and the file content
    /// ids those trees name.
    pub(crate) fn reachable(&self) -> Result<(HashSet<Id>, Vec<Id>), Error> {
        let (mut objects, mut files) = (HashSet::new(), Vec::new());
        let mut stack: Vec<Id> = self.store.refs()?.into_iter().map(|(_, id)| id).collect();
        while let Some(id) = stack.pop() {
            if !objects.insert(id) {
                continue;
            }
            match self.get(&id) {
                Ok(Object::Commit(c)) => stack.extend(std::iter::once(c.tree).chain(c.parents)),
                Ok(Object::Tree(entries)) => {
                    for e in entries {
                        if e.kind == Kind::Tree {
                            stack.push(e.id)
                        } else {
                            files.push(e.id)
                        }
                    }
                }
                _ => {}
            }
        }
        Ok((objects, files))
    }

    /// Content ids `roots` depend on through derivations accepted by
    /// `follow`, with the ids of those derivation records.
    pub(crate) fn closure(
        &self,
        mut stack: Vec<Id>,
        follow: impl Fn(&Derivation) -> bool,
    ) -> Result<(HashSet<Id>, HashSet<Id>), Error> {
        let (mut content, mut records) = (HashSet::new(), HashSet::new());
        while let Some(c) = stack.pop() {
            if !content.insert(c) {
                continue;
            }
            for d in self.derivations(&c)?.into_iter().filter(|d| follow(d)) {
                if !func::is_builtin(&d.func) {
                    stack.push(d.func);
                }
                stack.extend(d.args.iter().map(Arg::id));
                records.insert(Object::Derivation(d).id());
            }
        }
        Ok((content, records))
    }

    /// Delete every object that no ref needs. Every derivation of live
    /// content is kept along with its inputs, so live content stays
    /// reconstructible and keeps its provenance; claims are kept while both
    /// endpoints are live. Returns the number of objects deleted.
    pub fn gc(&mut self) -> Result<usize, Error> {
        let (mut objects, files) = self.reachable()?;
        let (content, records) = self.closure(files, |_| true)?;
        objects.extend(records);
        // Dictionaries that live stored bytes are encoded against.
        for id in &content {
            if let Ok(encoded) = self.store.read(id)
                && let Some(dict) = dict_of(&encoded)
            {
                objects.insert(dict);
            }
        }
        let mut deleted = 0;
        for id in self.store.ids()? {
            if !objects.contains(&id)
                && !content.contains(&id)
                && !self.claim_is_live(&id, &content)?
            {
                self.store.delete(&id)?;
                deleted += 1;
            }
        }
        self.reset_projections();
        Ok(deleted)
    }

    /// Claims annotate content without keeping it alive: one survives while
    /// both its endpoints do.
    fn claim_is_live(&self, id: &Id, content: &HashSet<Id>) -> Result<bool, Error> {
        if self.store.read(id)?.first() != Some(&b'L') {
            return Ok(false);
        }
        Ok(
            matches!(self.get(id), Ok(Object::Claim(c)) if content.contains(&c.subject.id()) && content.contains(&c.object.id())),
        )
    }
}
