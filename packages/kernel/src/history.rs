//! Commits, refs, history, garbage collection and verification.

use std::collections::HashSet;

use crate::dict::{names, needs, object_tag, tree_content};
use crate::repo::Cx;
use crate::{Arg, Commit, Derivation, Error, Graph, Id, Kind, Object, Repo, Store, Target, func};

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
    /// trees/commits/derivations against their hash, and every derivation a
    /// read depends on (its output not stored) by evaluating it (E097).
    /// Returns the failing object ids.
    pub fn fsck(&self) -> Result<Vec<(Id, Error)>, Error> {
        self.check_store(false)
    }

    /// `fsck`, also evaluating every derivation whose output is stored. Run
    /// after upgrading dependencies that reconstruction relies on.
    pub fn fsck_deep(&self) -> Result<Vec<(Id, Error)>, Error> {
        self.check_store(true)
    }

    fn check_store(&self, deep: bool) -> Result<Vec<(Id, Error)>, Error> {
        let mut failures = Vec::new();
        for id in self.store.ids()? {
            let encoded = self.store.read(&id)?;
            let result = match self.decode_plain(&id, &encoded) {
                Ok(bytes) if Id::of_content(&bytes) == id => Ok(()),
                Ok(_) => Err(Error::Corrupt(id)),
                Err(Error::WrongKind(_)) => self.check_object(&id, deep),
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                failures.push((id, e));
            }
        }
        Ok(failures)
    }

    /// A derivation is re-evaluated only if its output is not stored, or
    /// `deep` (E097): reads use a stored output first, and fsck hashes it; a
    /// read through a derivation checks its output anyway. Every record is
    /// still hash-checked.
    fn check_object(&self, id: &Id, deep: bool) -> Result<(), Error> {
        if let Object::Derivation(d) = self.get(id)?
            && (deep || !self.store.has(&d.output))
        {
            let out = self.eval(&d.func, &d.args, &mut Cx::default())?;
            if Id::of_content(&out) != d.output {
                return Err(Error::Corrupt(*id));
            }
        }
        Ok(())
    }

    /// Ids reachable from refs: tree/commit objects, the file content ids
    /// those trees name, and the content trees are stored as.
    pub(crate) fn reachable(&self) -> Result<(HashSet<Id>, Vec<Id>), Error> {
        let refs = self.store.refs()?.into_iter().map(|(_, id)| id).collect();
        self.reachable_from(refs, &HashSet::new())
    }

    /// `reachable` from `roots`, not descending into objects in `skip`
    /// (E268: what changed since an earlier walk).
    pub(crate) fn reachable_from(
        &self,
        mut stack: Vec<Id>,
        skip: &HashSet<Id>,
    ) -> Result<(HashSet<Id>, Vec<Id>), Error> {
        let (mut objects, mut files) = (HashSet::new(), Vec::new());
        while let Some(id) = stack.pop() {
            if skip.contains(&id) || !objects.insert(id) {
                continue;
            }
            // A tree stored as content keeps that content live.
            if let Ok(encoded) = self.store.read(&id)
                && let Some(content) = tree_content(&encoded)
            {
                files.push(content);
            }
            match self.get(&id) {
                Ok(Object::Commit(c)) => stack.extend(std::iter::once(c.tree).chain(c.parents)),
                Ok(Object::Tree(entries)) => {
                    for e in entries {
                        match e.kind {
                            Kind::File => files.push(e.id),
                            Kind::Tree | Kind::Graph => stack.push(e.id),
                        }
                    }
                }
                // A graph keeps its chunks and node targets live; its edges
                // name nodes by key and are never followed, so cycles are moot.
                Ok(Object::Graph(Graph::Children(_, children))) => {
                    stack.extend(children.into_iter().map(|(_, id)| id))
                }
                Ok(Object::Graph(Graph::Nodes(nodes))) => {
                    for n in nodes {
                        match n.target {
                            Target::Content(a) => files.push(a.id()),
                            Target::Object(id) => stack.push(id),
                            Target::None => {}
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
        // Dictionaries, groups and contexts that live stored encodings need:
        // every object their id prefix may name, and what those need in
        // turn (a group may be stored against a context, E268).
        let mut dicts: Vec<Vec<u8>> = Vec::new();
        let mut scan: Vec<Id> = content.iter().chain(&objects).copied().collect();
        let all = self.store.ids()?;
        while !scan.is_empty() {
            let mut found = false;
            for id in scan.drain(..) {
                if let Ok(encoded) = self.store.read(&id) {
                    for prefix in needs(&encoded) {
                        if !dicts.iter().any(|d| d == prefix) {
                            dicts.push(prefix.to_vec());
                            found = true;
                        }
                    }
                }
            }
            if !found {
                break;
            }
            for id in &all {
                if dicts.iter().any(|d| names(d, id))
                    && objects.insert(*id)
                    && !content.contains(id)
                {
                    scan.push(*id);
                }
            }
        }
        let mut dead = Vec::new();
        for id in self.store.ids()? {
            if !objects.contains(&id)
                && !content.contains(&id)
                && !self.claim_is_live(&id, &content)?
            {
                dead.push(id);
            }
        }
        let deleted = dead.len();
        self.delete_readers_first(dead)?;
        self.reset_projections();
        Ok(deleted)
    }

    /// Delete `ids` so that, if deleting stops part way, no stored object
    /// names one already gone (E310): an object goes only once none of
    /// `ids` still stored needs it, to decode (its dictionary, group,
    /// context or pointed content) or, for a derivation, to read its
    /// function and arguments (stored, or produced by another derivation
    /// among `ids`); objects before contents within a round. A cycle would
    /// be deleted in order.
    pub(crate) fn delete_readers_first(&mut self, ids: Vec<Id>) -> Result<(), Error> {
        let mut by_prefix: std::collections::HashMap<&[u8], Vec<usize>> = Default::default();
        for (i, id) in ids.iter().enumerate() {
            for n in [crate::dict::DICT_REF_LEN, crate::group::GROUP_REF_LEN, 32] {
                by_prefix.entry(&id.as_bytes()[..n]).or_default().push(i);
            }
        }
        let (mut readers, mut reads) = (vec![0usize; ids.len()], vec![Vec::new(); ids.len()]);
        let mut object = vec![false; ids.len()];
        // A derivation reads its function and arguments: stored ones, and
        // ones only another derivation among `ids` can produce.
        let mut derivations = vec![None; ids.len()];
        let mut produces: std::collections::HashMap<Id, Vec<usize>> = Default::default();
        for (i, id) in ids.iter().enumerate() {
            if object_tag(&self.store.read_head(id, 2)?) == Some(b'D')
                && let Ok(Object::Derivation(d)) = self.get(id)
            {
                produces.entry(d.output).or_default().push(i);
                derivations[i] = Some(d);
            }
        }
        for (i, id) in ids.iter().enumerate() {
            let head = self.store.read_head(id, 65)?;
            object[i] = object_tag(&head).is_some();
            let read: Vec<Id> = (derivations[i].iter())
                .flat_map(|d| d.args.iter().map(Arg::id).chain([d.func, d.output]))
                .collect();
            let mut named: Vec<usize> = (needs(&head).into_iter())
                .chain(tree_content(&head).as_ref().map(|c| &c.as_bytes()[..]))
                .chain(read.iter().map(|r| &r.as_bytes()[..]))
                .flat_map(|p| by_prefix.get(p).into_iter().flatten().copied())
                .chain(
                    read.iter()
                        .flat_map(|r| produces.get(r).into_iter().flatten().copied()),
                )
                .filter(|&j| j != i)
                .collect();
            named.sort_unstable();
            named.dedup();
            for &j in &named {
                readers[j] += 1;
            }
            reads[i] = named;
        }
        let mut left: Vec<usize> = (0..ids.len()).collect();
        while !left.is_empty() {
            let (mut ready, wait): (Vec<usize>, Vec<usize>) =
                left.into_iter().partition(|&i| readers[i] == 0);
            if ready.is_empty() {
                ready = wait;
                left = Vec::new();
            } else {
                left = wait;
            }
            // Objects first: a derivation names contents it does not decode with.
            ready.sort_by_key(|&i| !object[i]);
            let mut objects = true;
            for i in ready {
                if objects && !object[i] {
                    // A store may hold deletes until `sync` (E300's deferred
                    // FsStore) and then apply them in any order, so each step
                    // is made durable before the next one starts.
                    self.store.sync()?;
                    objects = false;
                }
                self.store.delete(&ids[i])?;
                for &j in &reads[i] {
                    readers[j] -= 1;
                }
            }
            self.store.sync()?;
        }
        Ok(())
    }

    /// Claims annotate content without keeping it alive: one survives while
    /// both its endpoints do.
    fn claim_is_live(&self, id: &Id, content: &HashSet<Id>) -> Result<bool, Error> {
        if object_tag(&self.store.read(id)?) != Some(b'L') {
            return Ok(false);
        }
        Ok(
            matches!(self.get(id), Ok(Object::Claim(c)) if content.contains(&c.subject.id()) && content.contains(&c.object.id())),
        )
    }
}
