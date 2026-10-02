//! The verified layer over a `Store`.
//!
//! Laws (tested in tests/laws.rs):
//! - `read_content(put_content(x).id) == x`, and the id is `Id::of_content(x)`
//!   no matter how `x` ends up stored.
//! - Every read is checked against its id; corruption is an error, never data.
//! - Any storage plan, from any planner, is verified before it is written; an
//!   invalid or unprofitable plan falls back to a plain blob.

use std::collections::{HashMap, HashSet};

use crate::matcher::{self, Index, Part};
use crate::{Commit, Error, Id, Kind, Object, Slice, Store};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// Already stored; nothing written.
    Existing,
    Blob,
    /// Stored as slices of other blobs.
    Rep,
}

#[derive(Debug)]
pub struct Put {
    pub id: Id,
    pub form: Form,
}

pub struct Repo<S: Store> {
    store: S,
    index: Option<Index>,
    /// Encoded bytes newly written through this handle.
    pub bytes_written: usize,
}

impl<S: Store> Repo<S> {
    pub fn new(store: S) -> Repo<S> {
        Repo {
            store,
            index: None,
            bytes_written: 0,
        }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        if self.store.write(id, encoded)? {
            self.bytes_written += encoded.len();
            if encoded[0] == b'B'
                && let Some(index) = &mut self.index
            {
                index.add(id, &encoded[1..]);
            }
        }
        Ok(())
    }

    /// Store a tree or commit (content goes through `put_content`).
    pub fn put(&mut self, obj: &Object) -> Result<Id, Error> {
        if !matches!(obj, Object::Tree(_) | Object::Commit(_)) {
            return Err(Error::WrongKind(obj.id()));
        }
        let encoded = obj.encode();
        Object::decode(&encoded)?; // only canonical objects are stored
        let id = Id::of(&encoded);
        self.write(id, &encoded)?;
        Ok(id)
    }

    /// Read a tree or commit, verifying its hash.
    pub fn get(&self, id: &Id) -> Result<Object, Error> {
        let encoded = self.store.read(id)?;
        if Id::of(&encoded) != *id {
            return Err(Error::Corrupt(*id));
        }
        match Object::decode(&encoded)? {
            obj @ (Object::Tree(_) | Object::Commit(_)) => Ok(obj),
            _ => Err(Error::WrongKind(*id)),
        }
    }

    pub fn put_content(&mut self, bytes: &[u8]) -> Result<Put, Error> {
        if self.store.has(&Id::of_content(bytes)) {
            return Ok(Put {
                id: Id::of_content(bytes),
                form: Form::Existing,
            });
        }
        self.ensure_index()?;
        let store = &self.store;
        let plan = matcher::plan(bytes, self.index.as_ref().unwrap(), |id| {
            stored_blob(store, id).ok()
        });
        self.put_planned(bytes, &plan)
    }

    /// Store `bytes` following `plan` if the plan is valid, reproduces `bytes`
    /// exactly, and is smaller than a plain blob; otherwise store a blob.
    pub fn put_planned(&mut self, bytes: &[u8], plan: &[Part]) -> Result<Put, Error> {
        let id = Id::of_content(bytes);
        if self.store.has(&id) {
            return Ok(Put {
                id,
                form: Form::Existing,
            });
        }
        if let Some((literal, rep)) = self.build_rep(bytes, plan) {
            let lit_cost = if literal.is_empty() || self.store.has(&Id::of_content(&literal)) {
                0
            } else {
                literal.len() + 1
            };
            let rep_enc = rep.encode();
            if rep_enc.len() + lit_cost < bytes.len() + 1 {
                if !literal.is_empty() {
                    let lit = Object::Blob(literal).encode();
                    self.write(Id::of(&lit), &lit)?;
                }
                self.write(id, &rep_enc)?;
                return Ok(Put {
                    id,
                    form: Form::Rep,
                });
            }
        }
        self.write(id, &Object::Blob(bytes.to_vec()).encode())?;
        Ok(Put {
            id,
            form: Form::Blob,
        })
    }

    /// Turn a plan into (literal bytes, rep), or `None` if it does not
    /// reproduce `bytes` exactly.
    fn build_rep(&self, bytes: &[u8], plan: &[Part]) -> Option<(Vec<u8>, Object)> {
        let mut literal = Vec::new();
        for part in plan {
            if let Part::Input(r) = part {
                literal.extend_from_slice(bytes.get(r.clone())?);
            }
        }
        let lit_id = Id::of_content(&literal);
        let (mut slices, mut lit_pos) = (Vec::<Slice>::new(), 0);
        for part in plan {
            let (src, start, len) = match part {
                Part::Input(r) => (lit_id, lit_pos, r.len()),
                Part::Existing { src, start, len } => (*src, *start, *len),
            };
            if let Part::Input(_) = part {
                lit_pos += len;
            }
            if len == 0 {
                continue;
            }
            match slices.last_mut() {
                Some(s) if s.src == src && s.start + s.len == start as u64 => s.len += len as u64,
                _ => slices.push(Slice {
                    src,
                    start: start as u64,
                    len: len as u64,
                }),
            }
        }
        if slices.iter().all(|s| s.src == lit_id) {
            return None; // nothing reused: a plain blob is strictly smaller
        }
        let rendered = render(&slices, |src| {
            if *src == lit_id {
                Ok(literal.clone())
            } else {
                stored_blob(&self.store, src)
            }
        })
        .ok()?;
        (rendered == bytes).then_some((literal, Object::Rep(slices)))
    }

    pub fn read_content(&self, id: &Id) -> Result<Vec<u8>, Error> {
        let encoded = self.store.read(id)?;
        let bytes = match encoded.first() {
            Some(b'B') => encoded[1..].to_vec(),
            Some(b'R') => match Object::decode(&encoded)? {
                Object::Rep(slices) => render(&slices, |src| stored_blob(&self.store, src))?,
                _ => unreachable!(),
            },
            _ => return Err(Error::WrongKind(*id)),
        };
        if Id::of_content(&bytes) != *id {
            return Err(Error::Corrupt(*id));
        }
        Ok(bytes)
    }

    fn ensure_index(&mut self) -> Result<(), Error> {
        if self.index.is_none() {
            let mut index = Index::default();
            for id in self.store.ids()? {
                if let Ok(bytes) = stored_blob(&self.store, &id) {
                    index.add(id, &bytes);
                }
            }
            self.index = Some(index);
        }
        Ok(())
    }

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

    /// Delete every object not reachable from a ref. Returns objects deleted.
    pub fn gc(&mut self) -> Result<usize, Error> {
        let mut live = HashSet::new();
        let mut stack: Vec<(Id, Option<Kind>)> = self
            .store
            .refs()?
            .into_iter()
            .map(|(_, id)| (id, None))
            .collect();
        while let Some((id, kind)) = stack.pop() {
            if !live.insert(id) {
                continue;
            }
            if kind == Some(Kind::File) {
                let encoded = self.store.read(&id)?;
                if encoded.first() == Some(&b'R')
                    && let Object::Rep(slices) = Object::decode(&encoded)?
                {
                    live.extend(slices.iter().map(|s| s.src));
                }
                continue;
            }
            match self.get(&id)? {
                Object::Commit(c) => {
                    stack.push((c.tree, Some(Kind::Tree)));
                    stack.extend(c.parents.into_iter().map(|p| (p, None)));
                }
                Object::Tree(entries) => {
                    stack.extend(entries.into_iter().map(|e| (e.id, Some(e.kind))))
                }
                _ => unreachable!(),
            }
        }
        let dead: Vec<Id> = self
            .store
            .ids()?
            .into_iter()
            .filter(|id| !live.contains(id))
            .collect();
        for id in &dead {
            self.store.delete(id)?;
        }
        self.index = None;
        Ok(dead.len())
    }
}

/// Bytes of a blob stored in plain form (slice sources must be plain blobs,
/// so reconstruction never chains through reps).
fn stored_blob<S: Store>(store: &S, id: &Id) -> Result<Vec<u8>, Error> {
    let mut encoded = store.read(id)?;
    if encoded.first() != Some(&b'B') {
        return Err(Error::WrongKind(*id));
    }
    encoded.remove(0);
    Ok(encoded)
}

fn render(
    slices: &[Slice],
    load: impl Fn(&Id) -> Result<Vec<u8>, Error>,
) -> Result<Vec<u8>, Error> {
    let mut cache: HashMap<Id, Vec<u8>> = HashMap::new();
    let mut out = Vec::new();
    for s in slices {
        if let std::collections::hash_map::Entry::Vacant(e) = cache.entry(s.src) {
            e.insert(load(&s.src)?);
        }
        let src = &cache[&s.src];
        let end = s.start.checked_add(s.len);
        let range = usize::try_from(s.start)
            .ok()
            .zip(end.and_then(|e| usize::try_from(e).ok()));
        let piece = range
            .and_then(|(a, b)| src.get(a..b))
            .ok_or(Error::Corrupt(s.src))?;
        out.extend_from_slice(piece);
    }
    Ok(out)
}
