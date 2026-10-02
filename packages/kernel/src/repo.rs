//! The verified layer over a `Store`.
//!
//! Laws (tested in tests/laws.rs):
//! - `read_content(put_content(x).id) == x`, and the id is `Id::of_content(x)`
//!   no matter how `x` ends up stored (blob, compressed blob, slices, or a
//!   function application).
//! - Every read is checked against its id; corruption is an error, never data.
//! - Any proposed storage form, from any planner, is verified by actually
//!   reconstructing the bytes before it is written. Invalid proposals are
//!   rejected; a failed or unprofitable slice plan falls back to a blob.

use std::collections::HashMap;

use crate::matcher::{self, Index, Part};
use crate::{Apply, Error, Id, Object, Slice, Store, container, func};

/// Bound on nested storage forms (an apply whose args are applies ...).
const MAX_DEPTH: usize = 16;
/// Bound on nested container expansion at ingest (a zip inside a zip ...).
const MAX_UNPACK_DEPTH: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// Already stored; nothing written.
    Existing,
    /// Plain bytes, zstd-compressed when that is smaller.
    Blob,
    /// Slices of other blobs.
    Rep,
    /// A pure function of other stored content.
    Apply,
}

#[derive(Debug)]
pub struct Put {
    pub id: Id,
    pub form: Form,
}

pub struct Repo<S: Store> {
    pub(crate) store: S,
    pub(crate) index: Option<Index>,
    /// Memo projection: apply key -> output id, rebuilt from `Run` objects.
    pub(crate) runs: Option<HashMap<Id, Id>>,
    /// Encoded bytes newly written through this handle.
    pub bytes_written: usize,
}

impl<S: Store> Repo<S> {
    pub fn new(store: S) -> Repo<S> {
        Repo {
            store,
            index: None,
            runs: None,
            bytes_written: 0,
        }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub(crate) fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        let fresh = self.store.write(id, encoded)?;
        if fresh {
            self.bytes_written += encoded.len();
        }
        Ok(fresh)
    }

    /// Store a tree, commit or run record (content goes through `put_content`).
    pub fn put(&mut self, obj: &Object) -> Result<Id, Error> {
        if !matches!(obj, Object::Tree(_) | Object::Commit(_) | Object::Run(_)) {
            return Err(Error::WrongKind(obj.id()));
        }
        let encoded = obj.encode();
        Object::decode(&encoded)?; // only canonical objects are stored
        let id = Id::of(&encoded);
        self.write(id, &encoded)?;
        Ok(id)
    }

    /// Read a tree, commit or run record, verifying its hash.
    pub fn get(&self, id: &Id) -> Result<Object, Error> {
        let encoded = self.store.read(id)?;
        if Id::of(&encoded) != *id {
            return Err(Error::Corrupt(*id));
        }
        match Object::decode(&encoded)? {
            obj @ (Object::Tree(_) | Object::Commit(_) | Object::Run(_)) => Ok(obj),
            _ => Err(Error::WrongKind(*id)),
        }
    }

    pub fn put_content(&mut self, bytes: &[u8]) -> Result<Put, Error> {
        self.put_content_at(bytes, 0)
    }

    fn put_content_at(&mut self, bytes: &[u8], depth: usize) -> Result<Put, Error> {
        let id = Id::of_content(bytes);
        if self.store.has(&id) {
            return Ok(Put {
                id,
                form: Form::Existing,
            });
        }
        if depth < MAX_UNPACK_DEPTH
            && let Some((manifest, members)) = container::plan_zip(bytes)
        {
            let mut args = vec![self.put_content_at(&manifest, depth + 1)?.id];
            for m in &members {
                args.push(self.put_content_at(m, depth + 1)?.id);
            }
            if let Some(put) = self.put_apply(
                bytes,
                Apply {
                    func: func::deflate_pack(),
                    args,
                },
            )? {
                return Ok(put);
            }
        }
        self.ensure_index()?;
        let store = &self.store;
        let plan = matcher::plan(bytes, self.index.as_ref().unwrap(), |id| {
            read_plain(store, id).ok()
        });
        self.put_planned(bytes, &plan)
    }

    /// Store `bytes` as `apply` if evaluating `apply` reproduces `bytes`
    /// exactly. This is the entry point for semantic dedup: any planner,
    /// including an AI, may propose "these bytes are f(args)". Returns `None`
    /// (and writes nothing) if the proposal does not reproduce the bytes.
    pub fn put_apply(&mut self, bytes: &[u8], apply: Apply) -> Result<Option<Put>, Error> {
        let id = Id::of_content(bytes);
        if self.store.has(&id) {
            return Ok(Some(Put {
                id,
                form: Form::Existing,
            }));
        }
        if self.eval(&apply, 0).ok().as_deref() != Some(bytes) {
            return Ok(None);
        }
        self.write(id, &Object::Apply(apply).encode())?;
        Ok(Some(Put {
            id,
            form: Form::Apply,
        }))
    }

    /// Store `bytes` following a slice plan if it is valid, reproduces
    /// `bytes` exactly, and is smaller than a plain blob; otherwise as a blob.
    pub fn put_planned(&mut self, bytes: &[u8], plan: &[Part]) -> Result<Put, Error> {
        let id = Id::of_content(bytes);
        if self.store.has(&id) {
            return Ok(Put {
                id,
                form: Form::Existing,
            });
        }
        let plain = plain_encoding(bytes);
        if let Some((literal, rep)) = self.build_rep(bytes, plan) {
            let lit_id = Id::of_content(&literal);
            let lit_plain =
                (!literal.is_empty() && !self.store.has(&lit_id)).then(|| plain_encoding(&literal));
            let rep_enc = rep.encode();
            if rep_enc.len() + lit_plain.as_ref().map_or(0, Vec::len) < plain.len() {
                if let Some(enc) = lit_plain {
                    self.write_plain(lit_id, &enc, &literal)?;
                }
                self.write(id, &rep_enc)?;
                return Ok(Put {
                    id,
                    form: Form::Rep,
                });
            }
        }
        self.write_plain(id, &plain, bytes)?;
        Ok(Put {
            id,
            form: Form::Blob,
        })
    }

    fn write_plain(&mut self, id: Id, encoded: &[u8], bytes: &[u8]) -> Result<(), Error> {
        if self.write(id, encoded)?
            && let Some(index) = &mut self.index
        {
            index.add(id, bytes);
        }
        Ok(())
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
        let load = |src: &Id| {
            if *src == lit_id {
                Ok(literal.clone())
            } else {
                read_plain(&self.store, src)
            }
        };
        let rendered = render(&slices, load).ok()?;
        (rendered == bytes).then_some((literal, Object::Rep(slices)))
    }

    pub fn read_content(&self, id: &Id) -> Result<Vec<u8>, Error> {
        self.read_at(id, 0)
    }

    fn read_at(&self, id: &Id, depth: usize) -> Result<Vec<u8>, Error> {
        if depth > MAX_DEPTH {
            return Err(Error::Exec(format!(
                "{id}: storage forms nested too deeply"
            )));
        }
        let encoded = self.store.read(id)?;
        let bytes = match encoded.first() {
            Some(b'B' | b'Z') => decode_plain(id, &encoded)?,
            Some(b'R' | b'A') => match Object::decode(&encoded)? {
                Object::Rep(slices) => render(&slices, |src| read_plain(&self.store, src))?,
                Object::Apply(apply) => self.eval(&apply, depth + 1)?,
                _ => unreachable!(),
            },
            _ => return Err(Error::WrongKind(*id)),
        };
        if Id::of_content(&bytes) != *id {
            return Err(Error::Corrupt(*id));
        }
        Ok(bytes)
    }

    /// Evaluate a function application over stored, verified content.
    pub(crate) fn eval(&self, apply: &Apply, depth: usize) -> Result<Vec<u8>, Error> {
        let args = apply
            .args
            .iter()
            .map(|a| self.read_at(a, depth + 1))
            .collect::<Result<Vec<_>, _>>()?;
        func::run(&apply.func, &args, |f| self.read_at(f, depth + 1))
    }

    pub(crate) fn ensure_index(&mut self) -> Result<(), Error> {
        if self.index.is_none() {
            let mut index = Index::default();
            for id in self.store.ids()? {
                if let Ok(bytes) = read_plain(&self.store, &id) {
                    index.add(id, &bytes);
                }
            }
            self.index = Some(index);
        }
        Ok(())
    }
}

/// `"B" bytes`, or `"Z" zstd(bytes)` when smaller. Identity is always over
/// the uncompressed bytes, so this choice never changes an id.
fn plain_encoding(bytes: &[u8]) -> Vec<u8> {
    if let Ok(z) = zstd::bulk::compress(bytes, 3)
        && z.len() < bytes.len()
    {
        return [&b"Z"[..], &z].concat();
    }
    [&b"B"[..], bytes].concat()
}

fn decode_plain(id: &Id, encoded: &[u8]) -> Result<Vec<u8>, Error> {
    match encoded.split_first() {
        Some((b'B', bytes)) => Ok(bytes.to_vec()),
        Some((b'Z', z)) => zstd::stream::decode_all(z).map_err(|_| Error::Corrupt(*id)),
        _ => Err(Error::WrongKind(*id)),
    }
}

/// Bytes of content stored in plain form. Slice sources must be plain, so
/// reconstruction never chains through reps. Unverified: callers verify the
/// final result against its own id.
pub(crate) fn read_plain<S: Store>(store: &S, id: &Id) -> Result<Vec<u8>, Error> {
    decode_plain(id, &store.read(id)?)
}

fn render(
    slices: &[Slice],
    load: impl Fn(&Id) -> Result<Vec<u8>, Error>,
) -> Result<Vec<u8>, Error> {
    let mut cache: HashMap<Id, Vec<u8>> = HashMap::new();
    let mut out = Vec::new();
    for s in slices {
        let src = match cache.entry(s.src) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::hash_map::Entry::Vacant(e) => e.insert(load(&s.src)?),
        };
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
