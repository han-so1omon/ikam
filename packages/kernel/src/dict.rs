//! Plain storage encodings, including a shared zstd dictionary.
//!
//! Stored bytes are encoded as the smallest of:
//!   "B" bytes | "Z" zstd(bytes) | "Y" dict[4] zstd_with_dict(bytes)
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

use std::io::Read;

use crate::{Error, Id, Repo, Store};

pub(crate) const DICT_REF: &str = "meta/zstd-dict";
/// Mode-byte flag: a tree body with abbreviated ids (see `abbrev.rs`).
const ABBREVIATED: u8 = 0x80;
/// Mode: an uncompressed body (only worth it abbreviated).
const RAW: u8 = 3;
/// Bytes of a dictionary's id stored in each encoding that uses it.
pub(crate) const DICT_REF_LEN: usize = 4;
type DictRef = [u8; DICT_REF_LEN];

/// True for any plain encoding of content bytes.
pub(crate) fn is_plain(encoded: &[u8]) -> bool {
    matches!(encoded.first(), Some(b'B' | b'Z' | b'Y'))
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
        [b'Y', ..] => 1,
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

/// True if `id` is one of the ids `dict` may name.
pub(crate) fn names(dict: &DictRef, id: &Id) -> bool {
    id.as_bytes().starts_with(dict)
}

fn decode_basic(id: &Id, encoded: &[u8]) -> Result<Vec<u8>, Error> {
    match encoded.split_first() {
        Some((b'B', bytes)) => Ok(bytes.to_vec()),
        Some((b'Z', z)) => zstd::stream::decode_all(z).map_err(|_| Error::Corrupt(*id)),
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

    /// Store `dict` as content and make it the current dictionary.
    pub(crate) fn set_dictionary(&mut self, dict: Vec<u8>) -> Result<Id, Error> {
        let id = Id::of_content(&dict);
        let z = zstd::bulk::compress(&dict, 3)
            .ok()
            .filter(|z| z.len() < dict.len());
        let encoded = z.map_or_else(|| [&b"B"[..], &dict].concat(), |z| [&b"Z"[..], &z].concat());
        self.write(id, &encoded)?;
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
        if let Ok(z) = zstd::bulk::compress(bytes, 3)
            && z.len() < bytes.len()
        {
            best = Some((None, z));
        }
        if let Some((id, dict)) = self.dictionary()
            && let Ok(z) = zstd::bulk::Compressor::with_dictionary(3, dict).and_then(|mut c| {
                c.set_parameter(zstd::zstd_safe::CParameter::DictIdFlag(false))?;
                c.compress(bytes)
            })
            && z.len() + DICT_REF_LEN < best.as_ref().map_or(bytes.len(), |b| b.1.len())
        {
            best = Some((Some(*id), z));
        }
        best
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
            0 if t.is_ascii_lowercase() => {
                finish(zstd::stream::decode_all(z).map_err(|_| Error::Corrupt(*id))?)
            }
            1 if t.is_ascii_lowercase() => {
                let rest = self.unzstd_with_dict(id, &encoded, 2, &|rest| {
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

    /// Decode any plain encoding. Unverified (callers check the result
    /// against its id), except that a dictionary is accepted only if its
    /// output does.
    pub(crate) fn decode_plain(&self, id: &Id, encoded: &[u8]) -> Result<Vec<u8>, Error> {
        match encoded.first() {
            Some(b'Y') => self.unzstd_with_dict(id, encoded, 1, &|b| Id::of_content(b) == *id),
            _ => decode_basic(id, encoded),
        }
    }

    /// Decompress the frame after the dictionary reference at `at`, with
    /// the first dictionary it names whose output passes `check`: the
    /// current dictionary first, then any stored content the prefix names.
    fn unzstd_with_dict(
        &self,
        id: &Id,
        encoded: &[u8],
        at: usize,
        check: &dyn Fn(&[u8]) -> bool,
    ) -> Result<Vec<u8>, Error> {
        let dict = dict_of(encoded).ok_or(Error::Corrupt(*id))?;
        let frame = &encoded[at + DICT_REF_LEN..];
        let attempt = |bytes: &[u8]| {
            let mut out = Vec::new();
            zstd::stream::Decoder::with_dictionary(frame, bytes)
                .and_then(|mut d| d.read_to_end(&mut out))
                .ok()
                .filter(|_| check(&out))
                .map(|_| out)
        };
        if let Some((current, bytes)) = self.dictionary()
            && names(&dict, current)
            && let Some(out) = attempt(bytes)
        {
            return Ok(out);
        }
        for candidate in &self.store.ids_with_prefix(&dict)? {
            if let Ok(bytes) = decode_basic(candidate, &self.store.read(candidate)?)
                && Id::of_content(&bytes) == *candidate
                && let Some(out) = attempt(&bytes)
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
    use crate::MemStore;

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
}
