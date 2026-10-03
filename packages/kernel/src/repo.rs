//! The verified layer over a `Store`: reading content through the ledger.
//!
//! Laws (tested in tests/):
//! - `read_content(put_content(x).id) == x`, and the id is `Id::of_content(x)`
//!   however `x` is stored: as bytes, or only through derivations.
//! - Every result is checked against its id. A corrupt stored copy falls
//!   through to the id's derivations; if none reproduces the bytes, the read
//!   fails. Corruption is an error, never data.
//! - Derivations enter the ledger only after they reproduce their output.

use std::cell::OnceCell;
use std::collections::HashMap;

use crate::matcher::Index;
use crate::{Arg, Derivation, Error, Id, Object, Store, func};

/// Bound on nested derivations followed by one read.
const MAX_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// Already stored or derivable; nothing new written for the bytes.
    Existing,
    /// Stored as bytes (zstd-compressed when smaller).
    Blob,
    /// Not stored as bytes; reconstructed from a derivation.
    Derived,
}

#[derive(Debug)]
pub struct Put {
    pub id: Id,
    pub form: Form,
}

/// Ledger projection: rebuilt from derivation objects on demand.
#[derive(Default)]
pub(crate) struct Ledger {
    pub(crate) by_output: HashMap<Id, Vec<Derivation>>,
    pub(crate) by_key: HashMap<Id, Id>,
}

impl Ledger {
    fn insert(&mut self, d: Derivation) {
        self.by_key
            .insert(Derivation::key(&d.func, &d.args), d.output);
        let ds = self.by_output.entry(d.output).or_default();
        if !ds.contains(&d) {
            ds.push(d);
        }
    }
}

/// State of one reconstruction: the derivation stack (cycle guard and depth
/// bound) and bytes already resolved, including not-yet-stored candidates.
#[derive(Default)]
pub(crate) struct Cx {
    stack: Vec<Id>,
    pub(crate) known: HashMap<Id, Vec<u8>>,
}

impl Cx {
    /// Treat `id` as being reconstructed, so nothing below may read it.
    pub(crate) fn stack_guard(&mut self, id: Id) {
        self.stack.push(id);
    }
}

pub struct Repo<S: Store> {
    pub(crate) store: S,
    pub(crate) index: Option<Index>,
    ledger: OnceCell<Ledger>,
    /// Encoded bytes newly written through this handle.
    pub bytes_written: usize,
}

impl<S: Store> Repo<S> {
    pub fn new(store: S) -> Repo<S> {
        Repo {
            store,
            index: None,
            ledger: OnceCell::new(),
            bytes_written: 0,
        }
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub(crate) fn ledger(&self) -> Result<&Ledger, Error> {
        if let Some(ledger) = self.ledger.get() {
            return Ok(ledger);
        }
        let mut ledger = Ledger::default();
        for id in self.store.ids()? {
            let encoded = self.store.read(&id)?;
            if encoded.first() == Some(&b'D')
                && Id::of(&encoded) == id
                && let Object::Derivation(d) = Object::decode(&encoded)?
            {
                ledger.insert(d);
            }
        }
        Ok(self.ledger.get_or_init(|| ledger))
    }

    pub(crate) fn reset_projections(&mut self) {
        self.index = None;
        self.ledger = OnceCell::new();
    }

    /// Every recorded derivation of `id`.
    pub fn derivations(&self, id: &Id) -> Result<Vec<Derivation>, Error> {
        Ok(self
            .ledger()?
            .by_output
            .get(id)
            .cloned()
            .unwrap_or_default())
    }

    /// True if `id` (a content id) is stored as bytes or has a derivation.
    pub fn has_content(&self, id: &Id) -> Result<bool, Error> {
        Ok(self.store.has(id) || self.ledger()?.by_output.contains_key(id))
    }

    pub(crate) fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        let fresh = self.store.write(id, encoded)?;
        if fresh {
            self.bytes_written += encoded.len();
        }
        Ok(fresh)
    }

    /// Store content bytes under their id.
    pub(crate) fn materialize(
        &mut self,
        id: Id,
        encoded: &[u8],
        bytes: &[u8],
    ) -> Result<(), Error> {
        if self.write(id, encoded)?
            && let Some(index) = &mut self.index
        {
            index.add(id, bytes);
        }
        Ok(())
    }

    /// Append a (verified) derivation to the ledger. Returns its object id.
    pub(crate) fn record(&mut self, d: Derivation) -> Result<Id, Error> {
        let encoded = Object::Derivation(d.clone()).encode();
        let id = Id::of(&encoded);
        if self.write(id, &encoded)?
            && let Some(ledger) = self.ledger.get_mut()
        {
            ledger.insert(d);
        }
        Ok(id)
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

    /// Read a tree, commit or derivation record, verifying its hash.
    pub fn get(&self, id: &Id) -> Result<Object, Error> {
        let encoded = self.store.read(id)?;
        if Id::of(&encoded) != *id {
            return Err(Error::Corrupt(*id));
        }
        match Object::decode(&encoded)? {
            Object::Blob(_) => Err(Error::WrongKind(*id)),
            obj => Ok(obj),
        }
    }

    pub fn read_content(&self, id: &Id) -> Result<Vec<u8>, Error> {
        self.rebuild(id, &mut Cx::default())
    }

    pub(crate) fn rebuild(&self, id: &Id, cx: &mut Cx) -> Result<Vec<u8>, Error> {
        if let Some(bytes) = cx.known.get(id) {
            return Ok(bytes.clone());
        }
        if cx.stack.len() >= MAX_DEPTH || cx.stack.contains(id) {
            return Err(Error::Exec(format!(
                "{id}: derivation cycle or nesting too deep"
            )));
        }
        let mut err = Error::NotFound(*id);
        match self.store.read(id) {
            Ok(encoded) => match decode_plain(id, &encoded) {
                Ok(bytes) if Id::of_content(&bytes) == *id => return self.remember(id, bytes, cx),
                Ok(_) | Err(Error::Corrupt(_)) => err = Error::Corrupt(*id),
                Err(e) => return Err(e),
            },
            Err(Error::NotFound(_)) => {}
            Err(e) => return Err(e),
        }
        let derivations = self.derivations(id)?;
        cx.stack.push(*id);
        for d in derivations {
            match self.eval(&d.func, &d.args, cx) {
                Ok(bytes) if Id::of_content(&bytes) == *id => {
                    cx.stack.pop();
                    return self.remember(id, bytes, cx);
                }
                Ok(_) => err = Error::Corrupt(*id),
                Err(e) if matches!(err, Error::NotFound(_)) => err = e,
                Err(_) => {}
            }
        }
        cx.stack.pop();
        Err(err)
    }

    fn remember(&self, id: &Id, bytes: Vec<u8>, cx: &mut Cx) -> Result<Vec<u8>, Error> {
        cx.known.insert(*id, bytes.clone());
        Ok(bytes)
    }

    /// Evaluate `func(args)` over stored content.
    pub(crate) fn eval(&self, func: &Id, args: &[Arg], cx: &mut Cx) -> Result<Vec<u8>, Error> {
        let mut inputs = Vec::with_capacity(args.len());
        for arg in args {
            let bytes = self.rebuild(&arg.id(), cx)?;
            inputs.push(match *arg {
                Arg::Whole(_) => bytes,
                Arg::Range { id, start, len } => select(&bytes, start, len)
                    .ok_or(Error::Corrupt(id))?
                    .to_vec(),
            });
        }
        let module = if func::is_builtin(func) {
            None
        } else {
            Some(self.rebuild(func, cx)?)
        };
        func::run(func, module.as_deref(), &inputs)
    }
}

fn select(bytes: &[u8], start: u64, len: u64) -> Option<&[u8]> {
    let start = usize::try_from(start).ok()?;
    bytes.get(start..start.checked_add(usize::try_from(len).ok()?)?)
}

/// `"B" bytes`, or `"Z" zstd(bytes)` when smaller. Identity is always over
/// the uncompressed bytes, so this choice never changes an id.
pub(crate) fn plain_encoding(bytes: &[u8]) -> Vec<u8> {
    if let Ok(z) = zstd::bulk::compress(bytes, 3)
        && z.len() < bytes.len()
    {
        return [&b"Z"[..], &z].concat();
    }
    [&b"B"[..], bytes].concat()
}

pub(crate) fn decode_plain(id: &Id, encoded: &[u8]) -> Result<Vec<u8>, Error> {
    match encoded.split_first() {
        Some((b'B', bytes)) => Ok(bytes.to_vec()),
        Some((b'Z', z)) => zstd::stream::decode_all(z).map_err(|_| Error::Corrupt(*id)),
        _ => Err(Error::WrongKind(*id)),
    }
}
