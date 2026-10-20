//! Plain storage encodings, including a shared zstd dictionary.
//!
//! Stored bytes are encoded as the smallest of:
//!   "B" bytes | "Z" zstd(bytes) | "Y" dict[4] zstd_with_dict(bytes)
//!   | "W" crc32[4] bytes   (stored as is with its own check, only from repack)
//!   | "S" group[8] start:varint len:varint   (bytes of a group, see `group.rs`)
//!   | "X" .lzma(bytes) | "R" brotli(bytes) | "P" ppmd(bytes)   (only from repack, see `codec.rs`)
//!   | "K" dict[4] block count { frame_len } frames   (a group in blocks, only from repack)
//!   | "Q" ref[32] zstd_with(ref's bytes)   (related content as the frame's
//!     context, only from repack; the ref is never "Q" or "S", so decoding
//!     never chains beyond one step)
//! Identity is always over the uncompressed bytes, so the choice never
//! changes an id. The dictionary is ordinary content, named by the ref
//! `meta/zstd-dict`. Exact reuse (slices, templates) and this statistical
//! reuse are complementary: many small files otherwise each pay for zstd
//! learning their shared vocabulary from scratch. A dictionary itself is
//! always stored as "B" or "Z", so decoding never chains.
//!
//! A dictionary is named by the first `DICT_REF_LEN` bytes of its id, not
//! all 32: most stored encodings are small, so a full id per encoding is a
//! large share of the store. A prefix can name several stored objects;
//! decoding tries each and accepts only output that hashes to the expected
//! id, so exactness never depends on the prefix being unique. Frames omit
//! zstd's own dictionary id for the same reason.
//!
//! Objects (trees, commits, derivations, claims) are stored the same way,
//! keeping their kind visible in the first byte:
//!   canonical | lower(tag) 0 zstd(rest) | lower(tag) 1 dict[4] zstd_with_dict(rest)
//!   | lower(tag) 3 rest
//! (a tree's rest may have abbreviated ids: mode | 0x80, see `abbrev.rs`)
//! where rest is the canonical encoding after its tag. Their ids stay
//! `BLAKE3(canonical)`. Records repeat the same content ids across objects,
//! which only a shared dictionary can exploit.
//!
//! A tree or graph chunk may also be stored as `t 2 content[32]` (`g 2`):
//! its canonical encoding is that content, stored like any other (sliced
//! against earlier versions, templated, re-planned by repack), so successive
//! versions share bytes. Only these: derivation records must stay readable
//! without the ledger they make up.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use zstd::zstd_safe::DDict;

use crate::group::{put_varint, varint};
use crate::{Error, Id, Repo, Store, codec};

pub(crate) const DICT_REF: &str = "meta/zstd-dict";
/// Ref to a content listing further dictionaries' ids (32 B each; E079):
/// contents of different kinds compress better with their own.
pub(crate) const DICTS_REF: &str = "meta/zstd-dicts";
/// Mode-byte flag: a tree body with abbreviated ids (see `abbrev.rs`).
const ABBREVIATED: u8 = 0x80;
/// Mode: an uncompressed body (only worth it abbreviated).
const RAW: u8 = 3;
/// Bytes of a dictionary's id stored in each encoding that uses it.
pub(crate) const DICT_REF_LEN: usize = 4;
type DictRef = [u8; DICT_REF_LEN];

/// True for any plain encoding of content bytes.
pub(crate) fn is_plain(encoded: &[u8]) -> bool {
    matches!(
        encoded.first(),
        Some(b'B' | b'W' | b'Z' | b'Y' | b'X' | b'R' | b'P' | b'K' | b'S' | b'Q')
    )
}

/// `bytes` stored as is with a CRC-32: "W" crc32_le[4] bytes (E074). A read
/// checks the CRC instead of hashing the bytes against their id: it catches
/// damaged storage for about the cost of a copy, where BLAKE3 costs 10-20x
/// more. It does not catch bytes swapped on purpose (fsck still hashes them
/// all); what binds the bytes to their id is that repack verified them
/// before writing the encoding.
pub(crate) fn checked_raw(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + 5);
    out.push(b'W');
    out.extend_from_slice(&crc32fast::hash(bytes).to_le_bytes());
    out.extend_from_slice(bytes);
    out
}

/// The bytes of a "W" body, if its CRC matches.
fn unchecked_raw(body: &[u8]) -> Option<&[u8]> {
    let (crc, bytes) = body.split_at_checked(4)?;
    (crc32fast::hash(bytes).to_le_bytes() == crc).then_some(bytes)
}

/// True if `decode_plain` already verified the output of `encoded`, so a
/// read need not hash it again (E075: reads of these hashed it twice). "W"
/// checks its CRC (E074); the others check the bytes against the id while
/// decoding (to pick among dictionaries or bases, or a group member's own
/// id). Only "B" and "Z" come back unverified.
pub(crate) fn self_verified(encoded: &[u8]) -> bool {
    matches!(
        encoded.first(),
        Some(b'W' | b'Y' | b'X' | b'R' | b'P' | b'Q' | b'K' | b'S')
    )
}

/// The id prefix of the stored content an encoding cannot be decoded
/// without (its dictionary, or its group), if any. gc keeps every object
/// the prefix names.
pub(crate) fn needs(encoded: &[u8]) -> Option<&[u8]> {
    match encoded {
        [b'S', group @ ..] => group.get(..crate::group::GROUP_REF_LEN),
        [b'Q', base @ ..] => base.get(..32),
        _ => dict_of(encoded).map(|_| {
            let at = if matches!(encoded[0], b'Y' | b'K') {
                1
            } else {
                2
            };
            &encoded[at..at + DICT_REF_LEN]
        }),
    }
}

/// The kind of a stored object ("T", "C", "D" or "L"), however it is
/// encoded; `None` for content.
pub(crate) fn object_tag(encoded: &[u8]) -> Option<u8> {
    let tag = encoded.first()?.to_ascii_uppercase();
    (!is_plain(encoded) && b"TCDLG".contains(&tag)).then_some(tag)
}

/// The dictionary reference (an id prefix) a stored encoding needs, if any.
pub(crate) fn dict_of(encoded: &[u8]) -> Option<DictRef> {
    let at = match encoded {
        [b'Y' | b'K', ..] => 1,
        [t, m, ..] if t.is_ascii_lowercase() && m & !ABBREVIATED == 1 => 2,
        _ => return None,
    };
    encoded.get(at..at + DICT_REF_LEN)?.try_into().ok()
}

/// The content holding a tree's or graph chunk's canonical encoding, if it
/// is stored so.
pub(crate) fn tree_content(encoded: &[u8]) -> Option<Id> {
    match encoded {
        [b't' | b'g', 2, id @ ..] => Some(Id::from_bytes(id.try_into().ok()?)),
        _ => None,
    }
}

/// True if `id` is one of the ids an id prefix may name.
pub(crate) fn names(prefix: &[u8], id: &Id) -> bool {
    id.as_bytes().starts_with(prefix)
}

/// zstd at `level` in the kernel's frame format: no magic number, content size,
/// checksum or dictionary id. The encoding around a frame already says what
/// it is, and decoded bytes are always checked against their id, so those
/// fields would only repeat what is known (up to ~10 B per stored object).
/// `dict` empty means no dictionary.
pub(crate) fn zstd_frame(bytes: &[u8], dict: &[u8], level: i32) -> Option<Vec<u8>> {
    framer_at(dict, level)?.compress(bytes).ok()
}

/// zstd level for estimates (growing groups, scoring dictionaries, comparing
/// trial plans) and for what ingest writes: fast.
pub(crate) const MEASURE_LEVEL: i32 = 3;
/// zstd level of what repack finally keeps (`Repo::recompress`). Decoding
/// speed barely depends on the level; only encoding is slower.
pub(crate) const STORE_LEVEL: i32 = 19;

/// A compressor for `zstd_frame`'s format at `MEASURE_LEVEL`, reusable
/// across many inputs.
pub(crate) fn framer(dict: &[u8]) -> Option<zstd::bulk::Compressor<'static>> {
    framer_at(dict, MEASURE_LEVEL)
}

fn framer_at(dict: &[u8], level: i32) -> Option<zstd::bulk::Compressor<'static>> {
    use zstd::zstd_safe::{CParameter, FrameFormat};
    let mut c = match dict.is_empty() {
        true => zstd::bulk::Compressor::new(level).ok()?,
        false => zstd::bulk::Compressor::with_dictionary(level, dict).ok()?,
    };
    for p in [
        CParameter::Format(FrameFormat::Magicless),
        CParameter::ContentSizeFlag(false),
        CParameter::ChecksumFlag(false),
        CParameter::DictIdFlag(false),
    ] {
        c.set_parameter(p).ok()?;
    }
    Some(c)
}

/// Decode a frame written by `zstd_frame` with the same `dict`.
pub(crate) fn unzstd_frame(frame: &[u8], dict: &[u8]) -> Option<Vec<u8>> {
    unzstd_prefix(frame, dict, usize::MAX)
}

thread_local! {
    static DECOMPRESSED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Decode cost of reads on this thread so far, in zstd-equivalent bytes:
/// bytes decompressed from stored frames, each weighted by its codec's
/// decode time relative to zstd (`codec::*_READ`), and since E069 bytes
/// read as stored at 1/8 (copying and verifying them: BLAKE3 measured at
/// 0.25 ns/B against zstd's 2.2). The part of a read the evaluator's work
/// (function output) leaves out. A measurement for benchmarks; the
/// difference between two calls is what one read cost.
pub fn decode_cost() -> usize {
    DECOMPRESSED.with(|d| d.get())
}

/// Counting units per zstd-equivalent byte, so cheaper-than-zstd reads
/// count in whole units.
pub(crate) const DECODE_UNITS: usize = 64;
/// Units per byte stored as is: copied and hashed ("B", 1/8 of zstd) or
/// copied and CRC-checked ("W", 1/64; see `codec::read_weight`).
pub(crate) const RAW_UNITS: usize = DECODE_UNITS / 8;
pub(crate) const CHECKED_UNITS: usize = 1;

/// Count `n` bytes read at `units` (`DECODE_UNITS` per zstd-equivalent
/// byte), rounded per call, so a read counts the same whatever came before.
pub(crate) fn count_decode(n: usize, units: usize) {
    let cost = (n * units + DECODE_UNITS / 2) / DECODE_UNITS;
    DECOMPRESSED.with(|d| d.set(d.get() + cost));
}

/// A dictionary to decode with: raw bytes (loaded per frame), or prepared
/// once per process (E068: loading rebuilds zstd's tables, which costs more
/// than decoding a small frame).
#[derive(Clone, Copy)]
pub(crate) enum Dict<'a> {
    Raw(&'a [u8]),
    Prepared(&'a DDict<'static>),
}

/// `dict` (content `id`) prepared for decoding, built once per process.
pub(crate) fn prepared(id: &Id, dict: &[u8]) -> Arc<DDict<'static>> {
    type Cache = Mutex<HashMap<Id, Arc<DDict<'static>>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Cache::default).lock().unwrap();
    cache
        .entry(*id)
        .or_insert_with(|| Arc::new(DDict::create(dict)))
        .clone()
}

/// Decode at most the first `limit` bytes of a frame: zstd decodes
/// sequentially, so a prefix costs only its own length.
pub(crate) fn unzstd_prefix(frame: &[u8], dict: &[u8], limit: usize) -> Option<Vec<u8>> {
    unzstd_prefix_with(frame, Dict::Raw(dict), limit)
}

pub(crate) fn unzstd_prefix_with(frame: &[u8], dict: Dict, limit: usize) -> Option<Vec<u8>> {
    use zstd::zstd_safe::{DCtx, DParameter, FrameFormat, InBuffer, OutBuffer};
    let mut ctx = DCtx::create();
    ctx.set_parameter(DParameter::Format(FrameFormat::Magicless))
        .ok()?;
    match dict {
        Dict::Raw([]) => {}
        Dict::Raw(raw) => drop(ctx.load_dictionary(raw).ok()?),
        Dict::Prepared(ddict) => drop(ctx.ref_ddict(ddict).ok()?),
    }
    // Stream until the frame ends or `limit` bytes are out.
    let (mut out, mut input, mut chunk) = (Vec::new(), InBuffer::around(frame), [0u8; 1 << 16]);
    while out.len() < limit {
        let mut buf = OutBuffer::around(&mut chunk[..]);
        let left = ctx.decompress_stream(&mut buf, &mut input).ok()?;
        let n = buf.pos();
        out.extend_from_slice(&chunk[..n]);
        if left == 0 {
            break; // the frame is complete
        }
        if n == 0 && input.pos() == frame.len() {
            return None; // truncated
        }
    }
    out.truncate(limit);
    count_decode(out.len(), DECODE_UNITS);
    Some(out)
}

/// A blocked group encoding's body: `block count { frame_len } frames`,
/// each block of `block` bytes (the last may be shorter) its own zstd frame
/// with `dict`, so a member read decodes only the blocks it overlaps (as in
/// zstd's seekable format). Stored as "K" dict[4] body.
pub(crate) fn block_frames(bytes: &[u8], dict: &[u8], block: usize) -> Option<Vec<u8>> {
    let frames: Vec<Vec<u8>> = bytes
        .chunks(block)
        .map(|c| zstd_frame(c, dict, STORE_LEVEL))
        .collect::<Option<_>>()?;
    let mut out = Vec::new();
    put_varint(&mut out, block);
    put_varint(&mut out, frames.len());
    frames.iter().for_each(|f| put_varint(&mut out, f.len()));
    frames.iter().for_each(|f| out.extend_from_slice(f));
    Some(out)
}

/// The blocks of a `block_frames` body overlapping `start..end`, decoded:
/// their first byte's offset in the content, and their bytes.
fn unblock(mut body: &[u8], dict: Dict, start: usize, end: usize) -> Option<(usize, Vec<u8>)> {
    let (block, count) = (varint(&mut body).filter(|&b| b > 0)?, varint(&mut body)?);
    let lens: Vec<usize> = (0..count)
        .map(|_| varint(&mut body))
        .collect::<Option<_>>()?;
    let (first, last) = ((start / block).min(count), end.div_ceil(block).min(count));
    let mut at = lens[..first].iter().sum::<usize>();
    let mut out = Vec::new();
    for &n in lens.get(first..last)? {
        out.extend(unzstd_prefix_with(body.get(at..at + n)?, dict, usize::MAX)?);
        at += n;
    }
    Some((first * block, out))
}

fn decode_basic(id: &Id, encoded: &[u8]) -> Result<Vec<u8>, Error> {
    match encoded.split_first() {
        Some((b'B', bytes)) => {
            count_decode(bytes.len(), RAW_UNITS);
            Ok(bytes.to_vec())
        }
        Some((b'W', body)) => {
            let bytes = unchecked_raw(body).ok_or(Error::Corrupt(*id))?;
            count_decode(bytes.len(), CHECKED_UNITS);
            Ok(bytes.to_vec())
        }
        Some((b'Z', z)) => unzstd_frame(z, &[]).ok_or(Error::Corrupt(*id)),
        _ => Err(Error::WrongKind(*id)),
    }
}

impl<S: Store> Repo<S> {
    /// The current dictionary, if the ref names one that can be read.
    pub(crate) fn dictionary(&self) -> Option<&(Id, Vec<u8>)> {
        self.dict
            .get_or_init(|| {
                let id = self.store.get_ref(DICT_REF).ok()??;
                let bytes = decode_basic(&id, &self.store.read(&id).ok()?).ok()?;
                (Id::of_content(&bytes) == id).then_some((id, bytes))
            })
            .as_ref()
    }

    /// Dictionaries beyond the current one (E079), each readable and
    /// verified; one that cannot be read is left out.
    pub(crate) fn extra_dictionaries(&self) -> &[(Id, Vec<u8>)] {
        self.extra_dicts.get_or_init(|| {
            let read = |id: &Id| {
                let bytes = decode_basic(id, &self.store.read(id).ok()?).ok()?;
                (Id::of_content(&bytes) == *id).then_some(bytes)
            };
            let Some(list) = self
                .store
                .get_ref(DICTS_REF)
                .ok()
                .flatten()
                .and_then(|l| read(&l))
            else {
                return Vec::new();
            };
            (list.chunks_exact(32))
                .map(|c| Id::from_bytes(c.try_into().unwrap()))
                .filter_map(|id| Some((id, read(&id)?)))
                .collect()
        })
    }

    /// True if `id` is the current or a further dictionary, or their list:
    /// stored as "B" or "Z", so they decode without a dictionary.
    pub(crate) fn is_dictionary(&self, id: &Id) -> bool {
        self.dictionary().is_some_and(|(d, _)| d == id)
            || self.extra_dictionaries().iter().any(|(d, _)| d == id)
            || self.store.get_ref(DICTS_REF).ok().flatten() == Some(*id)
    }

    /// Store `dicts` as content and make them the further dictionaries.
    pub(crate) fn set_extra_dictionaries(&mut self, dicts: Vec<Vec<u8>>) -> Result<(), Error> {
        let mut list = Vec::new();
        let mut kept = Vec::new();
        for dict in dicts {
            let id = self.store_dictionary(&dict)?;
            list.extend_from_slice(id.as_bytes());
            kept.push((id, dict));
        }
        let list_id = self.store_dictionary(&list)?;
        let current = self.store.get_ref(DICTS_REF)?;
        self.store.set_ref(DICTS_REF, current, list_id)?;
        self.extra_dicts = kept.into();
        Ok(())
    }

    /// Store a dictionary (or the list of them) as "B" or "Z", which
    /// decode without a dictionary.
    fn store_dictionary(&mut self, dict: &[u8]) -> Result<Id, Error> {
        let id = Id::of_content(dict);
        let z = zstd_frame(dict, &[], self.level).filter(|z| z.len() < dict.len());
        let encoded = z.map_or_else(|| [&b"B"[..], dict].concat(), |z| [&b"Z"[..], &z].concat());
        self.write(id, &encoded)?;
        Ok(id)
    }

    /// Store `dict` as content and make it the current dictionary.
    pub(crate) fn set_dictionary(&mut self, dict: Vec<u8>) -> Result<Id, Error> {
        let id = self.store_dictionary(&dict)?;
        let current = self.store.get_ref(DICT_REF)?;
        self.store.set_ref(DICT_REF, current, id)?;
        self.dict = Some((id, dict)).into();
        Ok(id)
    }

    /// The smaller of zstd alone and zstd with the current dictionary
    /// (counting the dictionary id it must name), if either is smaller than
    /// `bytes`.
    fn compress(&self, bytes: &[u8]) -> Option<(Option<Id>, Vec<u8>)> {
        let mut best: Option<(Option<Id>, Vec<u8>)> = None;
        if let Some(z) = zstd_frame(bytes, &[], self.level)
            && z.len() < bytes.len()
        {
            best = Some((None, z));
        }
        let dicts = self
            .dictionary()
            .into_iter()
            .chain(self.extra_dictionaries());
        for (id, dict) in dicts {
            if let Some(z) = zstd_frame(bytes, dict, self.level)
                && z.len() + DICT_REF_LEN < best.as_ref().map_or(bytes.len(), |b| b.1.len())
            {
                best = Some((Some(*id), z));
            }
        }
        best
    }

    /// The cheapest plain encoding of `bytes` among raw, zstd (at the repo's
    /// level) and the active `codec::CODECS`, for what repack keeps: stored bytes +
    /// `read_weight` x the codec's decode time x `reads` (bytes decoded to
    /// read everything once, see `Repo::recompress`).
    pub(crate) fn encode_cheapest(&self, bytes: &[u8], reads: usize) -> Vec<u8> {
        let mut best = self.encode_plain(bytes);
        // Stored as is: more bytes, nothing to decode (E069); with a CRC,
        // 4 bytes more and a cheaper check on read (E074).
        for raw in [[&b"B"[..], bytes].concat(), checked_raw(bytes)] {
            if self.cost(&raw, reads) < self.cost(&best, reads) {
                best = raw;
            }
        }
        for (tag, encode, _) in codec::CODECS.iter().filter(|c| c.2) {
            if let Some(c) = encode(bytes) {
                let c = [&[*tag][..], &c].concat();
                if self.cost(&c, reads) < self.cost(&best, reads) {
                    best = c;
                }
            }
        }
        best
    }

    /// Stored bytes + `read_weight` x decode time of a plain encoding that
    /// is decoded for `reads` bytes.
    pub(crate) fn cost(&self, encoded: &[u8], reads: usize) -> f64 {
        let weight = encoded.first().map_or(1.0, |t| codec::read_weight(*t));
        encoded.len() as f64 + self.read_weight * weight * reads as f64
    }

    /// The smallest plain encoding of `bytes`.
    pub(crate) fn encode_plain(&self, bytes: &[u8]) -> Vec<u8> {
        match self.compress(bytes) {
            Some((None, z)) => [&b"Z"[..], &z].concat(),
            Some((Some(id), z)) => [&b"Y"[..], &id.as_bytes()[..DICT_REF_LEN], &z].concat(),
            None => [&b"B"[..], bytes].concat(),
        }
    }

    /// The smallest stored encoding of a canonical object encoding. A tree
    /// may abbreviate its entry ids (see `abbrev.rs`), flagged by
    /// `ABBREVIATED` in the mode byte.
    pub(crate) fn encode_object(&self, canonical: &[u8]) -> Vec<u8> {
        let Some((&tag, rest)) = canonical.split_first() else {
            return canonical.to_vec();
        };
        let short = (tag == b'T')
            .then(|| self.abbreviate(rest).ok().flatten())
            .flatten();
        let tag = tag.to_ascii_lowercase();
        let mut best = canonical.to_vec();
        let bodies = [(0, Some(rest)), (ABBREVIATED, short.as_deref())];
        for (flag, body) in bodies.into_iter().filter_map(|(f, b)| Some((f, b?))) {
            let encoded = match self.compress(body) {
                Some((None, z)) => [&[tag, flag][..], &z].concat(),
                Some((Some(id), z)) => {
                    [&[tag, flag | 1][..], &id.as_bytes()[..DICT_REF_LEN], &z].concat()
                }
                // Too small to compress: abbreviation alone may still pay.
                None => [&[tag, flag | RAW][..], body].concat(),
            };
            if encoded.len() < best.len() {
                best = encoded;
            }
        }
        best
    }

    /// The canonical encoding of a stored object, or any other stored bytes
    /// unchanged. Unverified (callers check the result against its id),
    /// except that a dictionary is accepted only if its output does.
    pub(crate) fn decode_object(&self, id: &Id, encoded: Vec<u8>) -> Result<Vec<u8>, Error> {
        let (Some(tag), [t, mode, z @ ..]) = (object_tag(&encoded), encoded.as_slice()) else {
            return Ok(encoded);
        };
        // Canonical encoding from a decompressed body.
        let finish = |rest: Vec<u8>| -> Result<Vec<u8>, Error> {
            let rest = match mode & ABBREVIATED {
                0 => rest,
                _ => self.expand(id, &rest)?,
            };
            Ok([&[tag][..], &rest].concat())
        };
        match mode & !ABBREVIATED {
            0 if t.is_ascii_lowercase() => finish(unzstd_frame(z, &[]).ok_or(Error::Corrupt(*id))?),
            1 if t.is_ascii_lowercase() => {
                let rest = self.unzstd_with_dict(id, &encoded, 2, usize::MAX, &|rest| {
                    finish(rest.to_vec()).is_ok_and(|c| Id::of(&c) == *id)
                })?;
                finish(rest)
            }
            RAW if t.is_ascii_lowercase() => finish(z.to_vec()),
            _ => match tree_content(&encoded) {
                Some(content) => self.read_content(&content),
                None => Ok(encoded),
            },
        }
    }

    /// Read a stored object's canonical encoding (see `decode_object`).
    pub(crate) fn read_object(&self, id: &Id) -> Result<Vec<u8>, Error> {
        self.decode_object(id, self.store.read(id)?)
    }

    /// Decode any plain encoding. Verified against `id` for the encodings
    /// `self_verified` names; callers check the others ("B", "Z").
    pub(crate) fn decode_plain(&self, id: &Id, encoded: &[u8]) -> Result<Vec<u8>, Error> {
        match encoded.first() {
            Some(b'Y' | b'X' | b'R' | b'P' | b'Q') => {
                self.decode_prefix(id, encoded, usize::MAX, &|b| Id::of_content(b) == *id)
            }
            Some(b'K') => {
                let whole = |_: usize, b: &[u8]| Id::of_content(b) == *id;
                Ok(self.decode_blocks(id, encoded, 0, usize::MAX, &whole)?.1)
            }
            Some(b'S') => self.read_member(id, encoded),
            _ => decode_basic(id, encoded),
        }
    }

    /// The first `limit` bytes of a plain encoding ("B", "W", "Z", "Y", "X", "R", "P" or "Q"), if
    /// they pass `check`. Nothing here verifies the whole content's id, so
    /// `check` must verify what the caller uses (a group member's own id).
    pub(crate) fn decode_prefix(
        &self,
        id: &Id,
        encoded: &[u8],
        limit: usize,
        check: &dyn Fn(&[u8]) -> bool,
    ) -> Result<Vec<u8>, Error> {
        let out = match encoded.split_first() {
            Some((b'B', bytes)) => {
                let out = bytes[..limit.min(bytes.len())].to_vec();
                count_decode(out.len(), RAW_UNITS);
                Some(out)
            }
            // The CRC covers the whole body, so the whole body is checked.
            Some((b'W', body)) => unchecked_raw(body).map(|bytes| {
                count_decode(bytes.len(), CHECKED_UNITS);
                bytes[..limit.min(bytes.len())].to_vec()
            }),
            Some((b'Z', z)) => unzstd_prefix(z, &[], limit),
            Some((b'X', x)) => codec::unxz_prefix(x, limit),
            Some((b'R', r)) => codec::unbrotli_prefix(r, limit),
            Some((b'P', p)) => codec::unppmd_prefix(p, limit),
            Some((b'Y', _)) => return self.unzstd_with_dict(id, encoded, 1, limit, check),
            Some((b'Q', rest)) => {
                let (base, frame) = rest.split_at_checked(32).ok_or(Error::Corrupt(*id))?;
                let base = self.related_base(&Id::from_bytes(base.try_into().unwrap()))?;
                unzstd_prefix(frame, &base, limit)
            }
            _ => return Err(Error::WrongKind(*id)),
        };
        out.filter(|b| check(b)).ok_or(Error::Corrupt(*id))
    }

    /// The bytes of `base`, the context of a "Q" encoding: stored in any
    /// plain encoding but "Q" or "S" (one step, no chains), and verified.
    pub(crate) fn related_base(&self, base: &Id) -> Result<Vec<u8>, Error> {
        let stored = self.store.read(base)?;
        if matches!(stored.first(), Some(b'Q' | b'S')) {
            return Err(Error::Corrupt(*base));
        }
        let bytes = self.decode_plain(base, &stored)?;
        (Id::of_content(&bytes) == *base)
            .then_some(bytes)
            .ok_or(Error::Corrupt(*base))
    }

    /// True if some stored "Q" encoding uses `id` as its context.
    pub(crate) fn is_related_base(&self, id: &Id) -> Result<bool, Error> {
        for other in self.store.ids()? {
            let stored = self.store.read(&other)?;
            if stored.first() == Some(&b'Q') && stored.get(1..33) == Some(&id.as_bytes()[..]) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The blocks of a "K" encoding overlapping `start..end`, if they pass
    /// `check(offset, bytes)`: their first byte's offset and their bytes.
    /// As with `decode_prefix`, `check` must verify what the caller uses.
    pub(crate) fn decode_blocks(
        &self,
        id: &Id,
        encoded: &[u8],
        start: usize,
        end: usize,
        check: &dyn Fn(usize, &[u8]) -> bool,
    ) -> Result<(usize, Vec<u8>), Error> {
        let body = encoded.get(1 + DICT_REF_LEN..).ok_or(Error::Corrupt(*id))?;
        let decode = |d: Dict| unblock(body, d, start, end).filter(|(o, b)| check(*o, b));
        self.with_dict(id, encoded, &decode)
    }

    /// Decompress the frame after the dictionary reference at `at`, with
    /// the first dictionary it names whose output passes `check`.
    fn unzstd_with_dict(
        &self,
        id: &Id,
        encoded: &[u8],
        at: usize,
        limit: usize,
        check: &dyn Fn(&[u8]) -> bool,
    ) -> Result<Vec<u8>, Error> {
        let frame = &encoded[at + DICT_REF_LEN..];
        self.with_dict(id, encoded, &|d| {
            unzstd_prefix_with(frame, d, limit).filter(|out| check(out))
        })
    }

    /// Decode with the first dictionary the encoding names whose output
    /// `attempt` accepts: the current dictionary first, then any stored
    /// content the prefix names.
    fn with_dict<T>(
        &self,
        id: &Id,
        encoded: &[u8],
        attempt: &dyn Fn(Dict) -> Option<T>,
    ) -> Result<T, Error> {
        let dict = dict_of(encoded).ok_or(Error::Corrupt(*id))?;
        let known = self
            .dictionary()
            .into_iter()
            .chain(self.extra_dictionaries());
        for (current, bytes) in known {
            if names(&dict, current)
                && let Some(out) = attempt(Dict::Prepared(&prepared(current, bytes)))
            {
                return Ok(out);
            }
        }
        for candidate in &self.store.ids_with_prefix(&dict)? {
            if let Ok(bytes) = decode_basic(candidate, &self.store.read(candidate)?)
                && Id::of_content(&bytes) == *candidate
                && let Some(out) = attempt(Dict::Prepared(&prepared(candidate, &bytes)))
            {
                return Ok(out);
            }
        }
        Err(Error::Corrupt(*id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every codec in `codec::CODECS`, active or archived, still decodes
    /// through `decode_plain`: archiving stops trying a codec, never
    /// reading it.
    #[test]
    fn every_codec_in_the_table_still_decodes() {
        let repo = Repo::new(crate::MemStore::default());
        let text: Vec<u8> = (0..2000)
            .flat_map(|i| format!("line {i}\n").into_bytes())
            .collect();
        for (tag, encode, _) in codec::CODECS {
            let encoded = [&[tag][..], &encode(&text).unwrap()].concat();
            assert_eq!(
                repo.decode_plain(&Id::of_content(&text), &encoded).unwrap(),
                text
            );
        }
    }

    /// Codecs are chosen by stored bytes + read weight x decode time: a
    /// frame read very often keeps zstd, a frame never read takes the
    /// smallest codec.
    #[test]
    fn codec_choice_prices_reads() {
        let repo = Repo::new(crate::MemStore::default());
        let text: Vec<u8> = (0..3000)
            .flat_map(|i| format!("row {i}: some words {}\n", i % 17).into_bytes())
            .collect();
        let hot = repo.encode_cheapest(&text, 1 << 40);
        assert!(
            matches!(hot[0], b'Z' | b'B' | b'W'),
            "a hot frame keeps zstd"
        );
        let cold = repo.encode_cheapest(&text, 0);
        assert!(cold.len() <= hot.len());
        assert_eq!(
            repo.decode_plain(&Id::of_content(&text), &cold).unwrap(),
            text
        );
    }
    use crate::MemStore;

    /// Content of another kind compresses with a further dictionary (E079),
    /// which decodes like the current one and is never itself encoded
    /// against a dictionary.
    #[test]
    fn further_dictionaries_encode_and_decode() {
        let mut repo = Repo::new(MemStore::default());
        let prose = |i: usize| format!("Paragraph {i}: the quarterly review covers revenue.\n");
        let rows = |i: usize| format!("{{\"id\": {i}, \"sku\": \"A-{}\", \"qty\": 3}}\n", i * 7);
        let sample = |f: &dyn Fn(usize) -> String| (0..400).map(f).collect::<String>().into_bytes();
        repo.set_dictionary(sample(&prose)).unwrap();
        repo.set_extra_dictionaries(vec![sample(&rows)]).unwrap();
        let extra = repo.extra_dictionaries()[0].0;
        assert!(repo.is_dictionary(&extra));
        let text: Vec<u8> = (900..960).map(rows).collect::<String>().into_bytes();
        let id = repo.put_content(&text).unwrap().id;
        let stored = repo.store.read(&id).unwrap();
        assert_eq!(
            dict_of(&stored),
            Some(extra.as_bytes()[..DICT_REF_LEN].try_into().unwrap())
        );
        repo.reset_projections();
        assert_eq!(repo.read_content(&id).unwrap(), text);
        assert!(repo.fsck().unwrap().is_empty());
    }

    /// Encodings that `self_verified` names are read without a second hash
    /// (E075), so their decoding must reject well-formed bytes of another
    /// content.
    #[test]
    fn self_verified_encodings_reject_other_content() {
        let mut repo = Repo::new(MemStore::default());
        let text: Vec<u8> = (0..500)
            .flat_map(|i| format!("a {i}\n").into_bytes())
            .collect();
        let other: Vec<u8> = (0..500)
            .flat_map(|i| format!("b {i}\n").into_bytes())
            .collect();
        let id = repo.put_content(&text).unwrap().id;
        for (tag, encode, _) in codec::CODECS {
            let wrong = [&[tag][..], &encode(&other).unwrap()].concat();
            assert!(self_verified(&wrong));
            repo.store.replace(id, &wrong).unwrap();
            assert!(matches!(repo.read_content(&id), Err(Error::Corrupt(_))));
        }
    }

    /// A "W" read checks its CRC, not the id (E074): damage is an error,
    /// and only fsck, which hashes every content, sees bytes swapped with a
    /// matching CRC.
    #[test]
    fn checked_raw_reads_check_their_crc() {
        let mut repo = Repo::new(MemStore::default());
        let text = b"some stored text".to_vec();
        let id = repo.put_content(&text).unwrap().id;
        repo.store.replace(id, &checked_raw(&text)).unwrap();
        assert_eq!(repo.read_content(&id).unwrap(), text);
        let mut damaged = checked_raw(&text);
        damaged[7] ^= 1;
        repo.store.replace(id, &damaged).unwrap();
        assert!(matches!(repo.read_content(&id), Err(Error::Corrupt(_))));
        repo.store
            .replace(id, &checked_raw(b"other bytes"))
            .unwrap();
        assert!(!repo.fsck().unwrap().is_empty());
    }

    fn sample(n: usize) -> Vec<u8> {
        (0..n)
            .flat_map(|i| format!("<row id=\"{i}\">shared vocabulary {}</row>", i % 7).into_bytes())
            .collect()
    }

    /// Encodings name their dictionary by an id prefix. They stay readable
    /// after the current dictionary changes, and a decoy stored under an id
    /// with the same prefix is never used.
    #[test]
    fn short_dictionary_references_decode_exactly() {
        let mut repo = Repo::new(MemStore::default());
        let old = repo.set_dictionary(sample(200)).unwrap();
        let bytes = sample(40);
        let encoded = repo.encode_plain(&bytes);
        assert_eq!(encoded[0], b'Y');
        assert_eq!(dict_of(&encoded).unwrap(), old.as_bytes()[..DICT_REF_LEN]);

        let mut decoy = *old.as_bytes();
        decoy[31] ^= 1;
        let decoy = Id::from_bytes(decoy);
        repo.store
            .write(decoy, &[&b"B"[..], &sample(90)].concat())
            .unwrap();
        repo.set_dictionary(b"another dictionary entirely".repeat(50))
            .unwrap();

        let id = Id::of_content(&bytes);
        assert_eq!(repo.decode_plain(&id, &encoded).unwrap(), bytes);
        repo.store.delete(&old).unwrap();
        assert!(
            repo.decode_plain(&id, &encoded).is_err(),
            "only the decoy is left"
        );
    }
    /// A "Q" encoding decodes against its stored context, gc keeps that
    /// context, and a context that is itself "Q" is refused (no chains).
    #[test]
    fn related_context_decodes_one_step_only() {
        let mut repo = Repo::new(MemStore::default());
        let base: Vec<u8> = (0..4000u32)
            .flat_map(|i| format!("row {} {}\n", i % 97, i * 7 % 13).into_bytes())
            .collect();
        let other = [&base[..], b"tail"].concat();
        let b = repo.put_content(&base).unwrap().id;
        let o = Id::of_content(&other);
        let z = zstd_frame(&other, &base, STORE_LEVEL).unwrap();
        repo.store
            .write(o, &[&b"Q"[..], b.as_bytes(), &z].concat())
            .unwrap();
        assert_eq!(repo.read_content(&o).unwrap(), other);
        assert_eq!(
            needs(&repo.store.read(&o).unwrap()),
            Some(&b.as_bytes()[..])
        );
        assert!(repo.is_related_base(&b).unwrap());
        let third = [&other[..], b"more"].concat();
        let t = Id::of_content(&third);
        let z = zstd_frame(&third, &other, STORE_LEVEL).unwrap();
        repo.store
            .write(t, &[&b"Q"[..], o.as_bytes(), &z].concat())
            .unwrap();
        assert!(repo.read_content(&t).is_err());
    }
}
