//! The verified layer over a `Store`: reading content through the ledger.
//!
//! Laws (tested in tests/):
//! - `read_content(put_content(x).id) == x`, and the id is `Id::of_content(x)`
//!   however `x` is stored: as bytes, or only through derivations.
//! - Every result is checked against its id, or, for bytes stored as is
//!   with a CRC ("W", E074), against that CRC: repack verified them against
//!   their id before writing it, and fsck hashes them. A corrupt stored copy
//!   falls through to the id's derivations; if none reproduces the bytes,
//!   the read fails. Corruption is an error, never data.
//! - Derivations enter the ledger only after they reproduce their output.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use crate::dict::object_tag;
use crate::matcher::Index;
use crate::{Arg, Derivation, Error, Id, Object, Store, func};

/// Stack-safety guard for malformed stores (reconstruction recurses). It is
/// not a planning policy: the evaluator (`read_weight`) prices read work, so
/// plans this deep are never chosen in practice.
pub(crate) const STACK_GUARD: usize = 256;

/// Default weight of decode work against storage in plan choice: 1 KiB of
/// bytes produced while rebuilding content costs as much as 1 stored byte.
pub const DEFAULT_READ_WEIGHT: f64 = 0.001;

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
    pub(crate) by_output: BTreeMap<Id, Vec<Derivation>>,
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
    pub(crate) stack: Vec<Id>,
    pub(crate) known: HashMap<Id, Vec<u8>>,
    /// Bytes produced by function evaluations so far: the decode work.
    pub(crate) work: usize,
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
    ledger: OnceLock<Ledger>,
    /// Current shared dictionary (see `dict.rs`), loaded on first use.
    pub(crate) dict: OnceLock<Option<(Id, Vec<u8>)>>,
    /// Encoded bytes newly written through this handle.
    pub bytes_written: usize,
    /// Evaluator weight of decode work against storage when choosing plans:
    /// a plan scores `bytes written + read_weight * decode work`.
    pub read_weight: f64,
    /// An external proposal of content order for grouping (e.g. from an
    /// embedding model or an LLM; see `group.rs`). Only ever a proposal:
    /// it is used when it saves bytes, and never affects what is stored
    /// otherwise or how anything is rebuilt.
    pub proposed_order: Vec<Id>,
    /// The search workload, as the caller describes it (E065): searches per
    /// read pass, and sample queries. Repack stores a search index only if it
    /// pays for this workload; with a rate of 0 (the default) it never does.
    pub search_rate: f64,
    pub search_sample: Vec<Vec<u8>>,
    /// Store derivation records with short ids (E104): ids in a record are
    /// cut to 8-byte prefixes and resolved against the store and ledger on
    /// read, exactly as trees are. Off by default. On, it saves about a
    /// third of derivation-record bytes (north score 0.3035 to 0.2793 in
    /// E104), at the price of a two-pass ledger build and a prefix lookup per
    /// id. Reading always accepts both forms, so it can be switched at will.
    pub short_records: bool,
    /// Split the words of a zip's markup members (docx, xlsx, pptx XML)
    /// from their tags at ingest (E112): one skeleton member and one text
    /// member per zip, so the words compress with the same text in other
    /// formats. Off by default. Reading accepts both plans at any setting.
    pub markup_layers: bool,
    /// Contents larger than this skip the slow codecs (brotli) when repack
    /// picks their encoding (E109). Off (`usize::MAX`) by default. Brotli
    /// cannot use more than one core, so on a large content it bounds the
    /// whole recompress: on four pandas source releases it took 44 s of a
    /// 129 s repack for one 23.6 MB tar, and lost to zstd there (3% smaller,
    /// but four times the decode work). Set, a large content can be stored
    /// a few percent larger where brotli would have won.
    pub slow_codec_max: usize,
    /// Keep the set of ids stored encodings name (groups, contexts,
    /// dictionaries) in memory, so ingest can drop a superseded stored copy
    /// without reading every stored object first (E129). Off by default.
    /// On, it assumes this handle is the store's only writer while it is
    /// open (as `ikam serve` is): an encoding another process writes later
    /// is not seen, and the copy it needs could be dropped. Off, every drop
    /// at ingest reads the whole store.
    pub needed_index: bool,
    pub(crate) needed: std::sync::Mutex<Option<crate::dict::Needed>>,
    /// Where new content looks for repeats (E131). `None` (the default):
    /// every stored content, so the first write on a handle reads the whole
    /// store to build the matcher's seed index. `Some(roots)`: only those
    /// contents and what they are derived from, for example the files of
    /// the tree being written, or of one tenant's refs. Trees and graph
    /// chunks are stored as content too, so name the trees and view chunks
    /// a write will replace if their new versions should repeat them (E133:
    /// a view update stored 404 KB without its chunks, 227 KB with). A
    /// smaller scope
    /// reads less and never builds new content out of bytes outside it, at
    /// the price of missing repeats of content outside it. Content written
    /// through the handle is added to the index either way. Set it before
    /// the handle's first write, or with `set_seed_scope` later.
    pub seed_scope: Option<Vec<Id>>,
    /// zstd level new encodings use (`dict::MEASURE_LEVEL` unless
    /// `recompress` is running).
    pub(crate) level: i32,
    /// Large members' deflates, reused while a repack runs (E109); `None`
    /// otherwise, so every other read and fsck does the full work.
    pub(crate) deflate_memo: Option<std::sync::Arc<crate::container::DeflateMemo>>,
}

impl<S: Store> Repo<S> {
    pub fn new(store: S) -> Repo<S> {
        Repo {
            store,
            index: None,
            ledger: OnceLock::new(),
            dict: OnceLock::new(),
            bytes_written: 0,
            read_weight: DEFAULT_READ_WEIGHT,
            proposed_order: Vec::new(),
            search_rate: 0.0,
            search_sample: Vec::new(),
            short_records: false,
            markup_layers: false,
            slow_codec_max: usize::MAX,
            needed_index: false,
            needed: Default::default(),
            seed_scope: None,
            level: crate::dict::MEASURE_LEVEL,
            deflate_memo: None,
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
        // Records' outputs first: short records resolve their arguments
        // against stored ids and these (E104, `abbrev.rs`).
        let mut records = Vec::new();
        let mut outputs = std::collections::BTreeSet::new();
        // The needed index (E129) is another projection of every stored
        // encoding: build it from this same pass if it will be wanted
        // (E134), so a fresh handle reads the store once, not twice.
        let mut needed = (self.needed_index && self.needed.lock().unwrap().is_none())
            .then(crate::dict::Needed::default);
        for id in self.store.ids()? {
            // Both projections need only an encoding's header, except for
            // derivation records, which are read whole (E135).
            let head = self.store.read_head(&id, crate::dict::HEAD_LEN)?;
            if let Some(set) = needed.as_mut() {
                set.add(&head);
            }
            if object_tag(&head) != Some(b'D') {
                continue;
            }
            let encoded = if head.len() < crate::dict::HEAD_LEN {
                head
            } else {
                self.store.read(&id)?
            };
            outputs.extend(self.derivation_output(&id, &encoded));
            records.push((id, encoded));
        }
        let resolve = |prefix: &[u8]| -> Result<Vec<Id>, Error> {
            let mut out = self.store.ids_with_prefix(prefix)?;
            let start =
                Id::from_bytes(std::array::from_fn(|i| prefix.get(i).copied().unwrap_or(0)));
            for id in outputs
                .range(start..)
                .take_while(|id| id.as_bytes().starts_with(prefix))
            {
                if !out.contains(id) {
                    out.push(*id);
                }
            }
            Ok(out)
        };
        for (id, stored) in records {
            let encoded = self.decode_object_with(&id, stored, &resolve)?;
            if Id::of(&encoded) == id
                && let Object::Derivation(d) = Object::decode(&encoded)?
            {
                ledger.insert(d);
            }
        }
        if let Some(set) = needed {
            let mut slot = self.needed.lock().unwrap();
            if slot.is_none() {
                *slot = Some(set);
            }
        }
        Ok(self.ledger.get_or_init(|| ledger))
    }

    /// Change `seed_scope`, dropping the seed index so the next write builds
    /// it for the new scope (E133: one handle serving goals in different
    /// folders).
    pub fn set_seed_scope(&mut self, scope: Option<Vec<Id>>) {
        self.seed_scope = scope;
        self.index = None;
    }

    pub(crate) fn reset_projections(&mut self) {
        self.index = None;
        self.ledger = OnceLock::new();
        self.dict = OnceLock::new();
        *self.needed.get_mut().unwrap() = None;
    }

    /// Keep the needed set current after storing `encoded` (E129).
    pub(crate) fn note_needs(&mut self, encoded: &[u8]) {
        if let Some(set) = self.needed.get_mut().unwrap().as_mut() {
            set.add(encoded);
        }
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

    /// Stored or derivable ids starting with `prefix`, without duplicates.
    pub(crate) fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        let mut out = self.store.ids_with_prefix(prefix)?;
        for (id, _) in self
            .ledger()?
            .by_output
            .range(Id::from_bytes(std::array::from_fn(|i| prefix.get(i).copied().unwrap_or(0)))..)
        {
            if !id.as_bytes().starts_with(prefix) {
                break;
            }
            if !out.contains(id) {
                out.push(*id);
            }
        }
        Ok(out)
    }

    /// True if `id` (a content id) is stored as bytes or has a derivation.
    pub fn has_content(&self, id: &Id) -> Result<bool, Error> {
        Ok(self.store.has(id) || self.ledger()?.by_output.contains_key(id))
    }

    /// Write content in a plain encoding, or an object in its canonical
    /// encoding (stored in its smallest encoding, see `dict.rs`).
    pub(crate) fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        if self.store.has(&id) {
            return Ok(false);
        }
        let stored = match object_tag(encoded) {
            // Trees and graph chunks are stored as content, so a new version
            // is derived from earlier ones (slices); see `dict.rs`.
            Some(tag @ (b'T' | b'G')) => {
                let content = self.put_content(encoded)?.id;
                [&[tag.to_ascii_lowercase(), 2][..], content.as_bytes()].concat()
            }
            Some(_) => self.encode_object(encoded),
            None => encoded.to_vec(),
        };
        let fresh = self.store.write(id, &stored)?;
        self.note_needs(&stored);
        if fresh {
            self.bytes_written += stored.len();
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

    /// Store a tree, commit or graph chunk (content goes through `put_content`).
    pub fn put(&mut self, obj: &Object) -> Result<Id, Error> {
        if !matches!(obj, Object::Tree(_) | Object::Commit(_) | Object::Graph(_)) {
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
        let encoded = self.read_object(id)?;
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
        if cx.stack.len() >= STACK_GUARD || cx.stack.contains(id) {
            return Err(Error::Exec(format!(
                "{id}: derivation cycle or nesting too deep"
            )));
        }
        let mut err = Error::NotFound(*id);
        match self.store.read(id) {
            Ok(encoded) => match self.decode_plain(id, &encoded) {
                Ok(bytes)
                    if crate::dict::self_verified(&encoded) || Id::of_content(&bytes) == *id =>
                {
                    return self.remember(id, bytes, cx);
                }
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
            inputs.push(match *arg {
                Arg::Whole(id) => self.rebuild(&id, cx)?,
                // A range is cut from the rebuilt content in place: a slice
                // plan names its source once per range, and copying the whole
                // source for each was most of reading a release's archive
                // spliced from the one before (E109).
                Arg::Range { id, start, len } => {
                    if !cx.known.contains_key(&id) {
                        self.rebuild(&id, cx)?;
                    }
                    let bytes = cx.known.get(&id).ok_or(Error::NotFound(id))?;
                    select(bytes, start, len)
                        .ok_or(Error::Corrupt(id))?
                        .to_vec()
                }
            });
        }
        let module = if func::is_builtin(func) {
            None
        } else {
            Some(self.rebuild(func, cx)?)
        };
        let (out, work) = func::run(
            func,
            module.as_deref(),
            &inputs,
            self.deflate_memo.as_deref(),
        )?;
        cx.work += work;
        Ok(out)
    }
}

pub(crate) fn select(bytes: &[u8], start: u64, len: u64) -> Option<&[u8]> {
    let start = usize::try_from(start).ok()?;
    bytes.get(start..start.checked_add(usize::try_from(len).ok()?)?)
}
