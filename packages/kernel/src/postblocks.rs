//! Postings as blocks of row ids (E251), after Tantivy's postings: rows
//! numbered in key order (doc ids), each word's ids stored as one content
//! of delta-encoded ids bitpacked in blocks of 128, preceded by a skip
//! table (each block's last id and bit width), with the last partial
//! block as varints. An intersection walks the rarest list and seeks the
//! others with the skip table, decoding only the blocks it lands in.
//!
//! Opt-in: used by E247's segments with `SegmentOpts::block_base`.

use std::collections::BTreeMap;

use crate::{Arg, Db, Error, Id, Keys, Node, Repo, Store, Target};

pub const BLOCK: usize = 128;
/// Row keys per doc-map content.
const DOCS_PER_BLOCK: usize = 256;
/// Lists are packed into contents of about this many bytes (Tantivy's doc
/// store blocks are 16 KB); a word's node points at its byte range.
const PACK: usize = 16 << 10;

fn put_varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn varint(b: &[u8], at: &mut usize) -> Option<u32> {
    let (mut v, mut shift) = (0u32, 0);
    loop {
        let x = *b.get(*at)?;
        *at += 1;
        v |= u32::from(x & 0x7f) << shift;
        if x < 0x80 {
            return Some(v);
        }
        shift += 7;
    }
}

/// Sorted, distinct ids as a block list: count, the skip table (last id,
/// width a full block), the packed blocks, then the tail as varint deltas.
pub fn encode_ids(ids: &[u32]) -> Vec<u8> {
    let mut out = Vec::new();
    put_varint(&mut out, ids.len() as u32);
    let full = ids.len() / BLOCK;
    let (mut packed, mut prev) = (Vec::new(), 0u32);
    for b in 0..full {
        let block = &ids[b * BLOCK..(b + 1) * BLOCK];
        let deltas: Vec<u32> = block
            .iter()
            .scan(prev, |p, &id| {
                let d = id - *p;
                *p = id;
                Some(d)
            })
            .collect();
        let width = deltas
            .iter()
            .map(|d| 32 - d.leading_zeros())
            .max()
            .unwrap_or(0) as u8;
        out.extend_from_slice(&block[BLOCK - 1].to_le_bytes());
        out.push(width);
        let mut acc: u64 = 0;
        let mut bits = 0;
        for d in deltas {
            acc |= u64::from(d) << bits;
            bits += u32::from(width);
            while bits >= 8 {
                packed.push(acc as u8);
                acc >>= 8;
                bits -= 8;
            }
        }
        if bits > 0 {
            packed.push(acc as u8);
        }
        prev = block[BLOCK - 1];
    }
    out.extend_from_slice(&packed);
    for &id in &ids[full * BLOCK..] {
        put_varint(&mut out, id - prev);
        prev = id;
    }
    out
}

/// A decoded view of one block list, decoding blocks only when reached.
pub struct Ids<'a> {
    bytes: &'a [u8],
    count: usize,
    /// (last id, width, offset of its packed bytes) a full block.
    skips: Vec<(u32, u8, usize)>,
    tail: usize,
    block: Option<(usize, Vec<u32>)>,
}

impl<'a> Ids<'a> {
    pub fn new(bytes: &'a [u8]) -> Option<Ids<'a>> {
        let mut at = 0;
        let count = varint(bytes, &mut at)? as usize;
        let full = count / BLOCK;
        let mut skips = Vec::with_capacity(full);
        let mut off = at + full * 5;
        for b in 0..full {
            let h = &bytes.get(at + b * 5..at + b * 5 + 5)?;
            let width = h[4];
            skips.push((u32::from_le_bytes(h[..4].try_into().ok()?), width, off));
            off += (usize::from(width) * BLOCK).div_ceil(8);
        }
        Some(Ids {
            bytes,
            count,
            skips,
            tail: off,
            block: None,
        })
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Ids of block `b` (the tail is block `skips.len()`).
    fn block(&mut self, b: usize) -> &[u32] {
        if self.block.as_ref().is_none_or(|x| x.0 != b) {
            let mut prev = if b == 0 { 0 } else { self.skips[b - 1].0 };
            let mut ids = Vec::with_capacity(BLOCK);
            if let Some(&(_, width, off)) = self.skips.get(b) {
                let mask = if width == 0 {
                    0
                } else {
                    u64::MAX >> (64 - u32::from(width))
                };
                let (mut acc, mut bits, mut at) = (0u64, 0u32, off);
                for _ in 0..BLOCK {
                    while bits < u32::from(width) {
                        acc |= u64::from(self.bytes[at]) << bits;
                        at += 1;
                        bits += 8;
                    }
                    prev += (acc & mask) as u32;
                    acc = if width == 0 { acc } else { acc >> width };
                    bits -= u32::from(width);
                    ids.push(prev);
                }
            } else {
                let mut at = self.tail;
                while let Some(d) = varint(self.bytes, &mut at) {
                    prev += d;
                    ids.push(prev);
                }
            }
            self.block = Some((b, ids));
        }
        &self.block.as_ref().unwrap().1
    }

    /// Every id, in order.
    pub fn all(&mut self) -> Vec<u32> {
        (0..=self.skips.len())
            .flat_map(|b| self.block(b).to_vec())
            .collect()
    }

    /// The first id at least `target`: the skip table finds its block.
    pub fn seek(&mut self, target: u32) -> Option<u32> {
        let b = self.skips.partition_point(|s| s.0 < target);
        let ids = self.block(b);
        ids.get(ids.partition_point(|&x| x < target)).copied()
    }
}

/// Rows holding every list's ids: walk the shortest, seek the others.
pub fn intersect(lists: &mut [Ids]) -> Vec<u32> {
    lists.sort_by_key(Ids::len);
    let Some((first, rest)) = lists.split_first_mut() else {
        return Vec::new();
    };
    first
        .all()
        .into_iter()
        .filter(|&id| rest.iter_mut().all(|l| l.seek(id) == Some(id)))
        .collect()
}

/// A word index as block lists: `words` maps each word to its list, and
/// `docs` maps each id to its row key, `DOCS_PER_BLOCK` keys a content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockIndex {
    pub words: Id,
    pub docs: Id,
    pub rows: u32,
    pub postings: usize,
}

impl<S: Store> Repo<S> {
    fn put_plain(&mut self, bytes: &[u8]) -> Result<Id, Error> {
        let id = Id::of_content(bytes);
        if !self.has_content(&id)? {
            let encoded = self.encode_plain(bytes);
            self.materialize(id, &encoded, bytes)?;
        }
        Ok(id)
    }

    /// Index the rows of `db` under `prefix` as block lists.
    pub fn block_index_build(&mut self, db: &Db, prefix: &str) -> Result<BlockIndex, Error> {
        let rows = self.graph_prefix(&db.kv, prefix)?;
        let mut lists: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        let mut docs = Vec::new();
        for (i, n) in rows.iter().enumerate() {
            let text = self.kv_value(n)?;
            for w in crate::textindex::words(&text) {
                lists.entry(w).or_default().push(i as u32);
            }
            if i % DOCS_PER_BLOCK == 0 {
                docs.push(Vec::new());
            }
            let block = docs.last_mut().unwrap();
            block.extend_from_slice(n.label.as_bytes());
            block.push(b'\n');
        }
        let node = |label: String, id: Id| Node {
            label,
            target: Target::Content(Arg::Whole(id)),
            edges: Vec::new(),
        };
        let mut words = Vec::with_capacity(lists.len());
        let postings = lists.values().map(Vec::len).sum();
        let (mut pack, mut spans): (Vec<u8>, Vec<(String, usize, usize)>) =
            (Vec::new(), Vec::new());
        let mut lists = lists.into_iter().peekable();
        while let Some((w, ids)) = lists.next() {
            let bytes = encode_ids(&ids);
            spans.push((w, pack.len(), bytes.len()));
            pack.extend_from_slice(&bytes);
            if pack.len() >= PACK || lists.peek().is_none() {
                let id = self.put_plain(&pack)?;
                for (w, start, len) in spans.drain(..) {
                    let a = Arg::Range {
                        id,
                        start: start as u64,
                        len: len as u64,
                    };
                    words.push(Node {
                        label: w,
                        target: Target::Content(a),
                        edges: Vec::new(),
                    });
                }
                pack.clear();
            }
        }
        let mut doc_nodes = Vec::new();
        for (b, keys) in docs.iter().enumerate() {
            doc_nodes.push(node(format!("{b:010}"), self.put_plain(keys)?));
        }
        Ok(BlockIndex {
            words: self.put_graph_with(words, Keys::Any)?,
            docs: self.put_graph_with(doc_nodes, Keys::Any)?,
            rows: rows.len() as u32,
            postings,
        })
    }

    /// Ids holding every word of `words`, in order.
    pub fn block_index_ids(&self, index: &BlockIndex, words: &[String]) -> Result<Vec<u32>, Error> {
        let mut bytes = Vec::new();
        for w in words {
            match self.graph_node(&index.words, w)? {
                Some(Node {
                    target: Target::Content(Arg::Range { id, start, len }),
                    ..
                }) => {
                    let pack = self.read_content(&id)?;
                    let list = crate::repo::select(&pack, start, len).ok_or(Error::Corrupt(id))?;
                    bytes.push(list.to_vec());
                }
                _ => return Ok(Vec::new()),
            }
        }
        let mut lists: Vec<Ids> = bytes
            .iter()
            .map(|b| Ids::new(b).ok_or(Error::Decode("block list")))
            .collect::<Result<_, _>>()?;
        Ok(intersect(&mut lists))
    }

    /// The row keys of `ids` (sorted), reading only their doc-map blocks.
    pub fn block_index_keys(&self, index: &BlockIndex, ids: &[u32]) -> Result<Vec<String>, Error> {
        let (mut out, mut held): (Vec<String>, Option<(usize, Vec<String>)>) = (Vec::new(), None);
        for &id in ids {
            let b = id as usize / DOCS_PER_BLOCK;
            if held.as_ref().is_none_or(|h| h.0 != b) {
                let Some(Node {
                    target: Target::Content(a),
                    ..
                }) = self.graph_node(&index.docs, &format!("{b:010}"))?
                else {
                    return Err(Error::Decode("block doc map"));
                };
                let text = String::from_utf8(self.read_content(&a.id())?)
                    .map_err(|_| Error::Decode("doc map"))?;
                held = Some((b, text.lines().map(str::to_string).collect()));
            }
            let keys = &held.as_ref().unwrap().1;
            out.push(
                keys.get(id as usize % DOCS_PER_BLOCK)
                    .ok_or(Error::Decode("doc id"))?
                    .clone(),
            );
        }
        Ok(out)
    }
}
