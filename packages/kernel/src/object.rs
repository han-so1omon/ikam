//! L0 objects and their canonical encoding.
//!
//! There are exactly two object kinds. A `Blob` is opaque bytes. A `Node` is an
//! ordered list of labelled references whose rendering is the concatenation of
//! its children. Identity is `BLAKE3(encode(object))`; the encoding is strict,
//! so every byte string has at most one decoding and `encode(decode(b)) == b`.
//!
//! Encoding (all lengths are u32 big-endian):
//!   blob = b"B" bytes
//!   node = b"N" count { label_len label kind(b"B"|b"N") id[32] }

use crate::{Error, Id};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Blob,
    Node,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    pub kind: Kind,
    pub id: Id,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Object {
    Blob(Vec<u8>),
    Node(Vec<Entry>),
}

impl Kind {
    fn tag(self) -> u8 {
        match self {
            Kind::Blob => b'B',
            Kind::Node => b'N',
        }
    }

    fn from_tag(tag: u8) -> Result<Kind, Error> {
        match tag {
            b'B' => Ok(Kind::Blob),
            b'N' => Ok(Kind::Node),
            _ => Err(Error::Decode("unknown kind tag")),
        }
    }
}

impl Object {
    pub fn kind(&self) -> Kind {
        match self {
            Object::Blob(_) => Kind::Blob,
            Object::Node(_) => Kind::Node,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        match self {
            Object::Blob(bytes) => [&b"B"[..], bytes].concat(),
            Object::Node(entries) => {
                let mut out = vec![b'N'];
                put_len(&mut out, entries.len());
                for e in entries {
                    put_len(&mut out, e.label.len());
                    out.extend_from_slice(e.label.as_bytes());
                    out.push(e.kind.tag());
                    out.extend_from_slice(e.id.as_bytes());
                }
                out
            }
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Object, Error> {
        let (&tag, rest) = bytes.split_first().ok_or(Error::Decode("empty object"))?;
        match tag {
            b'B' => Ok(Object::Blob(rest.to_vec())),
            b'N' => decode_node(rest),
            _ => Err(Error::Decode("unknown object tag")),
        }
    }

    pub fn id(&self) -> Id {
        Id::of(&self.encode())
    }
}

fn put_len(out: &mut Vec<u8>, len: usize) {
    let len = u32::try_from(len).expect("length exceeds u32");
    out.extend_from_slice(&len.to_be_bytes());
}

fn decode_node(mut rest: &[u8]) -> Result<Object, Error> {
    let count = take_len(&mut rest)?;
    let mut entries = Vec::with_capacity(count.min(rest.len() / 37));
    for _ in 0..count {
        let label_len = take_len(&mut rest)?;
        let label = take(&mut rest, label_len)?;
        let label =
            String::from_utf8(label.to_vec()).map_err(|_| Error::Decode("label not utf-8"))?;
        let kind = Kind::from_tag(take(&mut rest, 1)?[0])?;
        let id = Id::from_bytes(take(&mut rest, 32)?.try_into().unwrap());
        entries.push(Entry { label, kind, id });
    }
    if !rest.is_empty() {
        return Err(Error::Decode("trailing bytes"));
    }
    Ok(Object::Node(entries))
}

fn take<'a>(rest: &mut &'a [u8], n: usize) -> Result<&'a [u8], Error> {
    if rest.len() < n {
        return Err(Error::Decode("truncated"));
    }
    let (head, tail) = rest.split_at(n);
    *rest = tail;
    Ok(head)
}

fn take_len(rest: &mut &[u8]) -> Result<usize, Error> {
    let raw = take(rest, 4)?;
    Ok(u32::from_be_bytes(raw.try_into().unwrap()) as usize)
}
