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

use std::io::Read;

use crate::{Error, Id, Repo, Store};

pub(crate) const DICT_REF: &str = "meta/zstd-dict";

/// True for any plain encoding of content bytes.
pub(crate) fn is_plain(encoded: &[u8]) -> bool {
    matches!(encoded.first(), Some(b'B' | b'Z' | b'Y'))
}

/// The dictionary a "Y" encoding needs, if any.
pub(crate) fn dict_of(encoded: &[u8]) -> Option<Id> {
    if encoded.first() != Some(&b'Y') {
        return None;
    }
    Some(Id::from_bytes(encoded.get(1..33)?.try_into().ok()?))
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

    /// The smallest plain encoding of `bytes`.
    pub(crate) fn encode_plain(&self, bytes: &[u8]) -> Vec<u8> {
        let mut best = [&b"B"[..], bytes].concat();
        if let Ok(z) = zstd::bulk::compress(bytes, 3)
            && z.len() + 1 < best.len()
        {
            best = [&b"Z"[..], &z].concat();
        }
        if let Some((id, dict)) = self.dictionary()
            && let Ok(z) =
                zstd::bulk::Compressor::with_dictionary(3, dict).and_then(|mut c| c.compress(bytes))
            && z.len() + 33 < best.len()
        {
            best = [&b"Y"[..], id.as_bytes(), &z].concat();
        }
        best
    }

    /// Decode any plain encoding. Unverified: callers check the result
    /// against its id.
    pub(crate) fn decode_plain(&self, id: &Id, encoded: &[u8]) -> Result<Vec<u8>, Error> {
        let Some(dict_id) = dict_of(encoded) else {
            return decode_basic(id, encoded);
        };
        let dict = match self.dictionary() {
            Some((current, bytes)) if *current == dict_id => bytes.clone(),
            _ => decode_basic(&dict_id, &self.store.read(&dict_id)?)?,
        };
        let mut out = Vec::new();
        zstd::stream::Decoder::with_dictionary(&encoded[33..], &dict)
            .and_then(|mut d| d.read_to_end(&mut out))
            .map_err(|_| Error::Corrupt(*id))?;
        Ok(out)
    }
}
