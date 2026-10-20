//! Content search (E058): which of a set of contents contain a byte
//! pattern. Without an index every content is decoded and scanned. A
//! trigram index (as in Google Code Search) narrows the candidates to the
//! contents holding every trigram of the pattern; each candidate is still
//! decoded and scanned, so results are exact whatever the index says, and
//! contents the index does not cover are always scanned.
//!
//! The index is ordinary content (ref `meta/search-index`), encoded as
//! `count:u32 { id[32] }` then, per trigram in order, `trigram[3]
//! n:varint { delta:varint }` (content positions in the id list).
//!
//! A chunk index (E089) instead searches what is shared once: contents are
//! cut into content-defined chunks; each distinct chunk is scanned once
//! (read from one content holding it), as is each distinct pair of
//! neighbouring chunks across their seam. A content contains the pattern
//! iff one of its chunks or seams does, since every chunk but a content's
//! last is at least `MIN_CHUNK` bytes and patterns are no longer. Encoded
//! as `CHUNKS count:u32 { id[32] } n:varint { owner offset len }` then per
//! content `n:varint { chunk }` (varints).

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::group::{put_varint, varint};
use crate::repo::Cx;
use crate::{Error, Id, Repo, Store};

pub const SEARCH_REF: &str = "meta/search-index";

/// A loaded index: the contents it covers and either, per trigram, the
/// positions of the contents holding it, or their chunks.
pub struct SearchIndex {
    files: Vec<Id>,
    postings: HashMap<[u8; 3], Vec<usize>>,
    chunks: Option<Chunks>,
}

/// Distinct chunks as `(owner, offset, len)` (owner: a position in
/// `files`), and each content's chunks in order.
/// Derived when loaded: the distinct seams (neighbouring chunk pairs), and
/// which contents hold each chunk and each seam, so a query scans each
/// once and marks the contents behind each hit.
struct Chunks {
    /// `(owner, offset, len)`; owner `files.len()` means `inline[offset..]`.
    table: Vec<(usize, usize, usize)>,
    inline: Vec<u8>,
    seams: Vec<(usize, usize)>,
    chunk_files: Vec<Vec<usize>>,
    seam_files: Vec<Vec<usize>>,
}

impl Chunks {
    fn new(
        table: Vec<(usize, usize, usize)>,
        inline: Vec<u8>,
        contents: Vec<Vec<usize>>,
    ) -> Chunks {
        let mut chunk_files = vec![Vec::new(); table.len()];
        let (mut seam_at, mut seams, mut seam_files) =
            (HashMap::new(), Vec::new(), Vec::<Vec<usize>>::new());
        for (f, list) in contents.iter().enumerate() {
            for &c in list {
                if chunk_files[c].last() != Some(&f) {
                    chunk_files[c].push(f);
                }
            }
            for w in list.windows(2) {
                let s = *seam_at.entry((w[0], w[1])).or_insert_with(|| {
                    seams.push((w[0], w[1]));
                    seam_files.push(Vec::new());
                    seams.len() - 1
                });
                if seam_files[s].last() != Some(&f) {
                    seam_files[s].push(f);
                }
            }
        }
        Chunks {
            table,
            inline,
            seams,
            chunk_files,
            seam_files,
        }
    }
}

/// First byte of a chunk index (a trigram index starts with a big-endian
/// count, never this large).
const CHUNKS: u8 = 0xc1;
/// Chunk sizes: at least `MIN_CHUNK` (and patterns up to it are answered
/// from chunks), about 1 KiB on average, at most `MAX_CHUNK`.
const MIN_CHUNK: usize = 256;
const MAX_CHUNK: usize = 8192;
const CHUNK_MASK: u64 = (1 << 10) - 1;
/// Decode units per byte scanned for a pattern (`dict::DECODE_UNITS` per
/// zstd byte): memmem measured at about 0.08 ns/B on the bench's
/// synthetic-history (0.05 ms per query over 650 KB, less the read), 1/32
/// of zstd's 2.2. Unpriced before E089, so a scan of stored-as-is content
/// cost the planner almost nothing.
const SCAN_UNITS: usize = 2;
/// A content whose own chunks are under 1/`INLINE_SHARE` of it has them
/// kept in the index instead of being read to search them.
const INLINE_SHARE: usize = 4;
/// A chunk index is chosen only below this share of the best other cost.
const CHUNK_MARGIN: f64 = 0.75;

/// Content-defined chunk ends of `bytes` (a gear hash, as FastCDC).
fn chunk_ends(bytes: &[u8]) -> Vec<usize> {
    let (mut ends, mut start, mut h) = (Vec::new(), 0, 0u64);
    for (i, &b) in bytes.iter().enumerate() {
        h = (h << 1).wrapping_add(crate::matcher::GEAR[b as usize]);
        let len = i + 1 - start;
        if len >= MAX_CHUNK || (len >= MIN_CHUNK && h & CHUNK_MASK == 0) {
            ends.push(i + 1);
            (start, h) = (i + 1, 0);
        }
    }
    if start < bytes.len() {
        ends.push(bytes.len());
    }
    ends
}

impl<S: Store> Repo<S> {
    /// Build and store a trigram index over `files`, named by `SEARCH_REF`.
    pub fn build_search_index(&mut self, files: &[Id]) -> Result<Id, Error> {
        let bytes = self.encode_index(files)?;
        let id = self.put_content(&bytes)?.id;
        let current = self.store.get_ref(SEARCH_REF)?;
        self.store.set_ref(SEARCH_REF, current, id)?;
        Ok(id)
    }

    /// The stored index, if any (its decoding counts as decode work).
    pub fn search_index(&self) -> Result<Option<SearchIndex>, Error> {
        match self.store.get_ref(SEARCH_REF)? {
            Some(id) => Ok(Some(decode_index(&self.read_content(&id)?)?)),
            None => Ok(None),
        }
    }

    /// Store an index over `files` only if it pays for the workload the
    /// caller described (`search_rate` searches per read pass, `search_sample`
    /// queries; E065): its stored bytes against `read_weight` x the decode
    /// work it saves the sample queries, measured. A scan's work does not
    /// depend on the query, so one query measures it.
    pub(crate) fn plan_search_index(&mut self, files: &[Id]) -> Result<(), Error> {
        if self.search_rate <= 0.0 || self.search_sample.is_empty() || files.is_empty() {
            return Ok(());
        }
        let work = |index: Option<&SearchIndex>, queries: &[Vec<u8>]| -> Result<f64, Error> {
            let before = crate::dict::decode_cost();
            let mut work = 0;
            for q in queries {
                work += self.search(q, files, index)?.1;
            }
            Ok((crate::dict::decode_cost() - before + work) as f64 / queries.len() as f64)
        };
        let scan = work(None, &self.search_sample[..1])?;
        let mut best: (f64, Option<Vec<u8>>) = (self.read_weight * self.search_rate * scan, None);
        // A trigram index, or a chunk index (E089): whichever pays most. A
        // chunk index must also beat the others by `CHUNK_MARGIN`: the work
        // units mispriced it by about 20% on transcripts (estimated 5%
        // cheaper than trigrams, measured 16% slower), where on structured
        // and synthetic-history it is estimated and measured 75-90% cheaper.
        for bytes in [self.encode_index(files)?, self.encode_chunks(files)?] {
            let index = decode_index(&bytes)?;
            let margin = if index.chunks.is_some() {
                CHUNK_MARGIN
            } else {
                1.0
            };
            let cost = self.encode_plain(&bytes).len() as f64
                + self.read_weight * self.search_rate * work(Some(&index), &self.search_sample)?;
            if cost < best.0 * margin {
                best = (cost, Some(bytes));
            }
        }
        if let Some(bytes) = best.1 {
            let id = self.put_content(&bytes)?.id;
            let current = self.store.get_ref(SEARCH_REF)?;
            self.store.set_ref(SEARCH_REF, current, id)?;
        }
        Ok(())
    }

    /// The chunk index of `files` (see the module documentation). A chunk is
    /// owned by the first content holding it, largest contents first, so
    /// few contents are read to search them all.
    fn encode_chunks(&self, files: &[Id]) -> Result<Vec<u8>, Error> {
        let texts: Vec<Vec<u8>> = files
            .iter()
            .map(|id| self.read_content(id))
            .collect::<Result<_, _>>()?;
        let mut order: Vec<usize> = (0..files.len()).collect();
        order.sort_by_key(|&i| (std::cmp::Reverse(texts[i].len()), i));
        let mut seen: HashMap<&[u8], usize> = HashMap::new();
        let mut table: Vec<(usize, usize, usize)> = Vec::new();
        let mut contents = vec![Vec::new(); files.len()];
        for i in order {
            let mut start = 0;
            for end in chunk_ends(&texts[i]) {
                let next = table.len();
                let piece = &texts[i][start..end];
                let c = *seen.entry(piece).or_insert(next);
                if c == next {
                    table.push((i, start, end - start));
                }
                contents[i].push(c);
                start = end;
            }
        }
        // A content owning little of its own (a version's few new chunks)
        // is not read at search time: its chunks are kept in the index.
        let mut owned = vec![0; files.len()];
        table
            .iter()
            .for_each(|&(owner, _, len)| owned[owner] += len);
        let inline = |owner: usize| owned[owner] * INLINE_SHARE < texts[owner].len();
        let mut out = vec![CHUNKS];
        out.extend_from_slice(&(files.len() as u32).to_be_bytes());
        files
            .iter()
            .for_each(|id| out.extend_from_slice(id.as_bytes()));
        put_varint(&mut out, table.len());
        for &(owner, offset, len) in &table {
            match inline(owner) {
                true => {
                    put_varint(&mut out, files.len());
                    put_varint(&mut out, len);
                    out.extend_from_slice(&texts[owner][offset..offset + len]);
                }
                false => [owner, offset, len]
                    .into_iter()
                    .for_each(|n| put_varint(&mut out, n)),
            }
        }
        for list in &contents {
            put_varint(&mut out, list.len());
            list.iter().for_each(|&c| put_varint(&mut out, c));
        }
        Ok(out)
    }

    /// The trigram index of `files` (see the module documentation).
    fn encode_index(&self, files: &[Id]) -> Result<Vec<u8>, Error> {
        let mut postings: BTreeMap<[u8; 3], Vec<usize>> = BTreeMap::new();
        for (i, id) in files.iter().enumerate() {
            let trigrams: HashSet<[u8; 3]> = self
                .read_content(id)?
                .windows(3)
                .map(|w| [w[0], w[1], w[2]])
                .collect();
            trigrams
                .into_iter()
                .for_each(|t| postings.entry(t).or_default().push(i));
        }
        let mut out = (files.len() as u32).to_be_bytes().to_vec();
        files
            .iter()
            .for_each(|id| out.extend_from_slice(id.as_bytes()));
        for (t, list) in &postings {
            out.extend_from_slice(t);
            put_varint(&mut out, list.len());
            let mut prev = 0;
            for &i in list {
                put_varint(&mut out, i - prev);
                prev = i;
            }
        }
        Ok(out)
    }

    /// The contents of `files` that contain `pattern`, and the decode work
    /// (bytes produced by functions) the scan took.
    pub fn search(
        &self,
        pattern: &[u8],
        files: &[Id],
        index: Option<&SearchIndex>,
    ) -> Result<(Vec<Id>, usize), Error> {
        if let Some(index) = index
            && let Some(chunks) = &index.chunks
            && !pattern.is_empty()
            && pattern.len() <= MIN_CHUNK
        {
            return self.search_chunks(pattern, files, &index.files, chunks);
        }
        let candidates: Vec<Id> = match index.filter(|i| pattern.len() >= 3 && i.chunks.is_none()) {
            Some(index) => {
                let covered: HashSet<&Id> = index.files.iter().collect();
                let mut hits: Option<HashSet<usize>> = None;
                for w in pattern.windows(3) {
                    let list = index.postings.get(&[w[0], w[1], w[2]]);
                    let set: HashSet<usize> = list.into_iter().flatten().copied().collect();
                    hits = Some(match hits {
                        Some(h) => h.intersection(&set).copied().collect(),
                        None => set,
                    });
                }
                let hits = hits.unwrap_or_default();
                let indexed = files.iter().filter(|id| covered.contains(id));
                let unindexed = files.iter().filter(|id| !covered.contains(id));
                let positions: HashMap<&Id, usize> = index
                    .files
                    .iter()
                    .enumerate()
                    .map(|(i, id)| (id, i))
                    .collect();
                let keep: HashSet<&Id> = indexed
                    .filter(|id| hits.contains(&positions[id]))
                    .chain(unindexed)
                    .collect();
                files
                    .iter()
                    .filter(|id| keep.contains(id))
                    .copied()
                    .collect()
            }
            None => files.to_vec(),
        };
        // A substring search built once per query (E072).
        let finder = memchr::memmem::Finder::new(pattern);
        let mut cx = Cx::default();
        let mut out = Vec::new();
        for id in candidates {
            let bytes = self.rebuild(&id, &mut cx)?;
            crate::dict::count_decode(bytes.len(), SCAN_UNITS);
            if finder.find(&bytes).is_some() {
                out.push(id);
            }
            cx.known.clear();
        }
        Ok((out, cx.work))
    }

    /// `search` through a chunk index: each distinct chunk and seam once.
    fn search_chunks(
        &self,
        pattern: &[u8],
        files: &[Id],
        covered: &[Id],
        chunks: &Chunks,
    ) -> Result<(Vec<Id>, usize), Error> {
        let finder = memchr::memmem::Finder::new(pattern);
        let position: HashMap<&Id, usize> =
            covered.iter().enumerate().map(|(i, id)| (id, i)).collect();
        let mut cx = Cx::default();
        let mut owners: HashMap<usize, Vec<u8>> = HashMap::new();
        for &(owner, _, _) in &chunks.table {
            if owner == covered.len() {
                continue;
            }
            if let std::collections::hash_map::Entry::Vacant(e) = owners.entry(owner) {
                e.insert(self.rebuild(&covered[owner], &mut cx)?);
                cx.known.clear();
            }
        }
        let bytes = |c: usize| {
            let (owner, offset, len) = chunks.table[c];
            let source = match owner == covered.len() {
                true => &chunks.inline,
                false => &owners[&owner],
            };
            source.get(offset..offset + len).unwrap_or_default()
        };
        let mut found = vec![false; covered.len()];
        for c in 0..chunks.table.len() {
            crate::dict::count_decode(bytes(c).len(), SCAN_UNITS);
            if finder.find(bytes(c)).is_some() {
                chunks.chunk_files[c].iter().for_each(|&f| found[f] = true);
            }
        }
        let (keep, mut seam) = (pattern.len() - 1, Vec::new());
        for (s, &(a, b)) in chunks.seams.iter().enumerate() {
            if chunks.seam_files[s].iter().all(|&f| found[f]) {
                continue;
            }
            let (a, b) = (bytes(a), bytes(b));
            seam.clear();
            seam.extend_from_slice(&a[a.len().saturating_sub(keep)..]);
            seam.extend_from_slice(&b[..b.len().min(keep)]);
            crate::dict::count_decode(seam.len(), SCAN_UNITS);
            if finder.find(&seam).is_some() {
                chunks.seam_files[s].iter().for_each(|&f| found[f] = true);
            }
        }
        let mut out = Vec::new();
        for id in files {
            let found = match position.get(id) {
                Some(&i) => found[i],
                None => {
                    let bytes = self.rebuild(id, &mut cx)?;
                    crate::dict::count_decode(bytes.len(), SCAN_UNITS);
                    let found = finder.find(&bytes).is_some();
                    cx.known.clear();
                    found
                }
            };
            if found {
                out.push(*id);
            }
        }
        Ok((out, cx.work))
    }
}

fn decode_index(bytes: &[u8]) -> Result<SearchIndex, Error> {
    let bad = || Error::Decode("search index");
    if bytes.first() == Some(&CHUNKS) {
        return decode_chunks(&bytes[1..]).ok_or_else(bad);
    }
    let count = u32::from_be_bytes(bytes.get(..4).ok_or_else(bad)?.try_into().unwrap()) as usize;
    let ids = bytes.get(4..4 + 32 * count).ok_or_else(bad)?;
    let files = ids
        .chunks(32)
        .map(|c| Id::from_bytes(c.try_into().unwrap()))
        .collect();
    let (mut rest, mut postings) = (&bytes[4 + 32 * count..], HashMap::new());
    while let Some((t, tail)) = rest.split_first_chunk::<3>() {
        rest = tail;
        let n = varint(&mut rest).ok_or_else(bad)?;
        let mut list = Vec::with_capacity(n);
        for _ in 0..n {
            list.push(list.last().copied().unwrap_or(0) + varint(&mut rest).ok_or_else(bad)?);
        }
        postings.insert(*t, list);
    }
    Ok(SearchIndex {
        files,
        postings,
        chunks: None,
    })
}

fn decode_chunks(bytes: &[u8]) -> Option<SearchIndex> {
    let count = u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?) as usize;
    let ids = bytes.get(4..4 + 32 * count)?;
    let files: Vec<Id> = ids
        .chunks(32)
        .map(|c| Id::from_bytes(c.try_into().unwrap()))
        .collect();
    let mut rest = &bytes[4 + 32 * count..];
    let n = varint(&mut rest)?;
    let (mut table, mut inline) = (Vec::with_capacity(n), Vec::new());
    for _ in 0..n {
        let owner = varint(&mut rest)?;
        if owner == count {
            let len = varint(&mut rest)?;
            let (bytes, tail) = rest.split_at_checked(len)?;
            table.push((owner, inline.len(), len));
            inline.extend_from_slice(bytes);
            rest = tail;
        } else {
            table.push((owner, varint(&mut rest)?, varint(&mut rest)?));
        }
    }
    let contents = (0..count)
        .map(|_| {
            let k = varint(&mut rest)?;
            (0..k)
                .map(|_| varint(&mut rest).filter(|&c| c < n))
                .collect()
        })
        .collect::<Option<Vec<Vec<usize>>>>()?;
    let owners_ok = table.iter().all(|&(owner, _, _)| owner <= count);
    (rest.is_empty() && owners_ok).then_some(SearchIndex {
        files,
        postings: HashMap::new(),
        chunks: Some(Chunks::new(table, inline, contents)),
    })
}

#[cfg(test)]
mod tests {
    use crate::{MemStore, Repo};

    /// Indexed and unindexed searches return the same contents, and the
    /// index narrows what is decoded.
    #[test]
    fn indexed_search_matches_a_full_scan() {
        let mut repo = Repo::new(MemStore::default());
        let texts = [
            &b"alpha beta gamma"[..],
            b"beta delta",
            b"epsilon zeta",
            b"zeta alpha",
        ];
        let ids: Vec<_> = texts
            .iter()
            .map(|t| repo.put_content(t).unwrap().id)
            .collect();
        repo.build_search_index(&ids[..3]).unwrap(); // the last is not indexed
        let index = repo.search_index().unwrap().unwrap();
        for pattern in [&b"alpha"[..], b"beta", b"zeta", b"ta", b"none"] {
            let full = repo.search(pattern, &ids, None).unwrap().0;
            let fast = repo.search(pattern, &ids, Some(&index)).unwrap().0;
            assert_eq!(full, fast, "{}", String::from_utf8_lossy(pattern));
        }
        assert_eq!(
            repo.search(b"alpha", &ids, Some(&index)).unwrap().0,
            vec![ids[0], ids[3]]
        );
    }

    /// A chunk index answers as a full scan does, for patterns inside
    /// chunks, across seams, longer than a chunk, and in contents it does
    /// not cover (E089).
    #[test]
    fn chunk_search_matches_a_full_scan() {
        let mut repo = Repo::new(MemStore::default());
        let mut text = String::new();
        let mut versions = Vec::new();
        for i in 0..12u64 {
            for j in 0..60u64 {
                let v = (i * 7919 + j * 104_729) % 100_003;
                text.push_str(&format!("line {i}.{j}: value {v} of {}\n", j * j));
            }
            versions.push(text.clone().into_bytes());
        }
        let ids: Vec<_> = versions
            .iter()
            .map(|t| repo.put_content(t).unwrap().id)
            .collect();
        let index = super::decode_index(&repo.encode_chunks(&ids[..10]).unwrap()).unwrap();
        let chunks = index.chunks.as_ref().unwrap();
        assert!(
            chunks.chunk_files.iter().any(|f| f.len() > 1),
            "versions share chunks"
        );
        let long = versions[11][100..400].to_vec();
        for pattern in [
            &b"line 3.7:"[..],
            b"line 11.",
            b"\nline 5",
            b"nothing here",
            &long,
            b"9",
        ] {
            let full = repo.search(pattern, &ids, None).unwrap().0;
            let fast = repo.search(pattern, &ids, Some(&index)).unwrap().0;
            assert_eq!(full, fast, "{}", String::from_utf8_lossy(pattern));
        }
        // Every seam: a pattern straddling each chunk end of the last version.
        let last = &versions[9];
        for end in super::chunk_ends(last) {
            if end > 8 && end + 8 <= last.len() {
                let pattern = &last[end - 8..end + 8];
                assert_eq!(
                    repo.search(pattern, &ids, None).unwrap().0,
                    repo.search(pattern, &ids, Some(&index)).unwrap().0
                );
            }
        }
    }

    /// An index is stored only when the described workload makes it pay.
    #[test]
    fn index_is_kept_only_when_it_pays() {
        let mut repo = Repo::new(MemStore::default());
        let texts: Vec<Vec<u8>> = (0..40u32)
            .map(|i| {
                format!(
                    "file {i}: {}\n",
                    format!("word{} ", i * 7919 % 1000).repeat(200)
                )
                .into_bytes()
            })
            .collect();
        let ids: Vec<_> = texts
            .iter()
            .map(|t| repo.put_content(t).unwrap().id)
            .collect();
        repo.search_sample = vec![b"word123".to_vec(), b"word456".to_vec()];
        repo.plan_search_index(&ids).unwrap(); // rate 0: never
        assert!(repo.search_index().unwrap().is_none());
        repo.search_rate = 1e6;
        repo.plan_search_index(&ids).unwrap();
        assert!(repo.search_index().unwrap().is_some());
    }
}
