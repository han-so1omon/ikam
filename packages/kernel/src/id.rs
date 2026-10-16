use std::fmt;
use std::str::FromStr;

use crate::Error;

/// Content identity: BLAKE3 of an object's canonical encoding. No fallbacks.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id([u8; 32]);

impl Id {
    pub fn of(encoded: &[u8]) -> Id {
        Id(*blake3::hash(encoded).as_bytes())
    }

    /// Identity of file content: the id of its blob encoding (`"B" ++ bytes`).
    /// Every verified read computes it. Up to `CONTIGUOUS` bytes it hashes one
    /// copied buffer: fed in two parts, BLAKE3 takes its first 1 KiB chunk
    /// byte-wise and hashes small contents about 2x slower (E071, measured
    /// at 5–16 KiB); beyond that the copy costs more than it saves.
    pub fn of_content(bytes: &[u8]) -> Id {
        const CONTIGUOUS: usize = 256 << 10;
        if bytes.len() <= CONTIGUOUS {
            let mut buf = Vec::with_capacity(bytes.len() + 1);
            buf.push(b'B');
            buf.extend_from_slice(bytes);
            return Id(*blake3::hash(&buf).as_bytes());
        }
        let mut h = blake3::Hasher::new();
        h.update(b"B");
        h.update(bytes);
        Id(*h.finalize().as_bytes())
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Id {
        Id(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", &self.to_hex()[..12])
    }
}

impl FromStr for Id {
    type Err = Error;

    fn from_str(s: &str) -> Result<Id, Error> {
        let bad = || Error::Decode("id must be 64 lowercase hex chars");
        if s.len() != 64
            || !s
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(bad());
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| bad())?;
        }
        Ok(Id(out))
    }
}

#[cfg(test)]
mod tests {
    use super::Id;

    /// Both hashing paths give the id of the blob encoding.
    #[test]
    fn content_id_is_the_blob_id_at_every_size() {
        for n in [0, 1, 1023, 1024, 5000, 256 << 10, (256 << 10) + 1, 300_000] {
            let bytes: Vec<u8> = (0..n).map(|i| (i * 31 % 251) as u8).collect();
            assert_eq!(
                Id::of_content(&bytes),
                Id::of(&[&b"B"[..], &bytes].concat()),
                "{n}"
            );
        }
    }
}
