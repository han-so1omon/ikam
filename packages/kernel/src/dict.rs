//! Plain storage encodings, including a shared zstd dictionary.
//!
//! Stored bytes are encoded as the smallest of:
//!   "B" bytes | "Z" zstd(bytes) | "Y" dict[32] zstd_with_dict(bytes)
//! Identity is always over the uncompressed bytes, so the choice never
//! changes an id. The dictionary is ordinary content, named by the ref
//! `meta/zstd-dict`. Exact reuse (slices, templates) and this statistical
//! reuse are complementary: many small files otherwise each pay for zstd
//! learning their shared vocabulary from scratch. A dictionary itself is
//! always stored as "B" or "Z", so decoding never chains.
//!
//! Objects (trees, commits, derivations, claims) are stored the same way,
//! keeping their kind visible in the first byte:
//!   canonical | lower(tag) 0 zstd(rest) | lower(tag) 1 dict[32] zstd_with_dict(rest)
//! where rest is the canonical encoding after its tag. Their ids stay
//! `BLAKE3(canonical)`. Records repeat the same content ids across objects,
//! which only a shared dictionary can exploit.

use std::io::Read;

use crate::{Error, Id, Repo, Store};

pub(crate) const DICT_REF: &str = "meta/zstd-dict";

/// True for any plain encoding of content bytes.
pub(crate) fn is_plain(encoded: &[u8]) -> bool {
    matches!(encoded.first(), Some(b'B' | b'Z' | b'Y'))
}

/// The kind of a stored object ("T", "C", "D" or "L"), however it is
/// encoded; `None` for content.
pub(crate) fn object_tag(encoded: &[u8]) -> Option<u8> {
    let tag = encoded.first()?.to_ascii_uppercase();
    (!is_plain(encoded) && b"TCDL".contains(&tag)).then_some(tag)
}

/// The dictionary a stored encoding needs, if any.
pub(crate) fn dict_of(encoded: &[u8]) -> Option<Id> {
    let at = match encoded {
        [b'Y', ..] => 1,
        [t, 1, ..] if t.is_ascii_lowercase() => 2,
        _ => return None,
    };
    Some(Id::from_bytes(encoded.get(at..at + 32)?.try_into().ok()?))
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
            && let Ok(z) =
                zstd::bulk::Compressor::with_dictionary(3, dict).and_then(|mut c| c.compress(bytes))
            && z.len() + 32 < best.as_ref().map_or(bytes.len(), |b| b.1.len())
        {
            best = Some((Some(*id), z));
        }
        best
    }

    /// The smallest plain encoding of `bytes`.
    pub(crate) fn encode_plain(&self, bytes: &[u8]) -> Vec<u8> {
        match self.compress(bytes) {
            Some((None, z)) => [&b"Z"[..], &z].concat(),
            Some((Some(id), z)) => [&b"Y"[..], id.as_bytes(), &z].concat(),
            None => [&b"B"[..], bytes].concat(),
        }
    }

    /// The smallest stored encoding of a canonical object encoding.
    pub(crate) fn encode_object(&self, canonical: &[u8]) -> Vec<u8> {
        let Some((&tag, rest)) = canonical.split_first() else {
            return canonical.to_vec();
        };
        let tag = tag.to_ascii_lowercase();
        let encoded = match self.compress(rest) {
            Some((None, z)) => [&[tag, 0][..], &z].concat(),
            Some((Some(id), z)) => [&[tag, 1][..], id.as_bytes(), &z].concat(),
            None => return canonical.to_vec(),
        };
        if encoded.len() < canonical.len() {
            encoded
        } else {
            canonical.to_vec()
        }
    }

    /// The canonical encoding of a stored object, or any other stored bytes
    /// unchanged. Unverified: callers check the result against its id.
    pub(crate) fn decode_object(&self, id: &Id, encoded: Vec<u8>) -> Result<Vec<u8>, Error> {
        match encoded.as_slice() {
            [t, 0, z @ ..] if object_tag(&encoded).is_some() && t.is_ascii_lowercase() => {
                let rest = zstd::stream::decode_all(z).map_err(|_| Error::Corrupt(*id))?;
                Ok([&[t.to_ascii_uppercase()][..], &rest].concat())
            }
            [t, 1, ..] if object_tag(&encoded).is_some() && t.is_ascii_lowercase() => {
                let rest = self.unzstd_with_dict(id, &encoded, 34)?;
                Ok([&[t.to_ascii_uppercase()][..], &rest].concat())
            }
            _ => Ok(encoded),
        }
    }

    /// Read a stored object's canonical encoding (see `decode_object`).
    pub(crate) fn read_object(&self, id: &Id) -> Result<Vec<u8>, Error> {
        self.decode_object(id, self.store.read(id)?)
    }

    /// Decode any plain encoding. Unverified: callers check the result
    /// against its id.
    pub(crate) fn decode_plain(&self, id: &Id, encoded: &[u8]) -> Result<Vec<u8>, Error> {
        match encoded.first() {
            Some(b'Y') => self.unzstd_with_dict(id, encoded, 33),
            _ => decode_basic(id, encoded),
        }
    }

    /// Decompress `encoded[frame..]` with the dictionary `encoded` names.
    fn unzstd_with_dict(&self, id: &Id, encoded: &[u8], frame: usize) -> Result<Vec<u8>, Error> {
        let dict_id = dict_of(encoded).ok_or(Error::Corrupt(*id))?;
        let dict = match self.dictionary() {
            Some((current, bytes)) if *current == dict_id => bytes.clone(),
            _ => decode_basic(&dict_id, &self.store.read(&dict_id)?)?,
        };
        let mut out = Vec::new();
        zstd::stream::Decoder::with_dictionary(&encoded[frame..], &dict)
            .and_then(|mut d| d.read_to_end(&mut out))
            .map_err(|_| Error::Corrupt(*id))?;
        Ok(out)
    }
}
