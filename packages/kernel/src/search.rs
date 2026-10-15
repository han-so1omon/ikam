//! Content search (E058): which of a set of contents contain a byte
//! pattern. Without an index every content is decoded and scanned. A
//! trigram index (as in Google Code Search) narrows the candidates to the
//! contents holding every trigram of the pattern; each candidate is still
//! decoded and scanned, so results are exact whatever the index says, and
//! contents the index does not cover are always scanned.
//!
//! The index is ordinary content (ref `meta/search-index`), encoded as
//! `count:u32 { id[32] }`, `common:varint { trigram[3] }`, then per other
//! trigram in order `trigram[3] n:varint { delta:varint }` (content
//! positions in the id list). A common trigram (in more than half of the
//! contents, E061) narrows little and costs the most, so it gets no list
//! and constrains nothing; any trigram neither listed nor common is in no
//! content.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::group::{put_varint, varint};
use crate::repo::Cx;
use crate::{Error, Id, Repo, Store};

pub const SEARCH_REF: &str = "meta/search-index";

/// A loaded trigram index: the contents it covers and, per trigram, the
/// positions of the contents holding it.
pub struct SearchIndex {
    files: Vec<Id>,
    common: HashSet<[u8; 3]>,
    postings: HashMap<[u8; 3], Vec<usize>>,
}

impl<S: Store> Repo<S> {
    /// Build and store a trigram index over `files`, named by `SEARCH_REF`.
    pub fn build_search_index(&mut self, files: &[Id]) -> Result<Id, Error> {
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
        let common: Vec<&[u8; 3]> = (postings.iter())
            .filter(|(_, list)| 2 * list.len() > files.len())
            .map(|(t, _)| t)
            .collect();
        put_varint(&mut out, common.len());
        common.iter().for_each(|t| out.extend_from_slice(*t));
        for (t, list) in postings.iter().filter(|(_, l)| 2 * l.len() <= files.len()) {
            out.extend_from_slice(t);
            put_varint(&mut out, list.len());
            let mut prev = 0;
            for &i in list {
                put_varint(&mut out, i - prev);
                prev = i;
            }
        }
        let id = self.put_content(&out)?.id;
        let current = self.store.get_ref(SEARCH_REF)?;
        self.store.set_ref(SEARCH_REF, current, id)?;
        Ok(id)
    }

    /// The stored index, if any (its decoding counts as decode work).
    pub fn search_index(&self) -> Result<Option<SearchIndex>, Error> {
        let Some(id) = self.store.get_ref(SEARCH_REF)? else {
            return Ok(None);
        };
        let bytes = self.read_content(&id)?;
        let bad = || Error::Decode("search index");
        let count =
            u32::from_be_bytes(bytes.get(..4).ok_or_else(bad)?.try_into().unwrap()) as usize;
        let ids = bytes.get(4..4 + 32 * count).ok_or_else(bad)?;
        let files = ids
            .chunks(32)
            .map(|c| Id::from_bytes(c.try_into().unwrap()))
            .collect();
        let mut rest = &bytes[4 + 32 * count..];
        let mut common = HashSet::new();
        for _ in 0..varint(&mut rest).ok_or_else(bad)? {
            let (t, tail) = rest.split_first_chunk::<3>().ok_or_else(bad)?;
            common.insert(*t);
            rest = tail;
        }
        let mut postings = HashMap::new();
        while let Some((t, tail)) = rest.split_first_chunk::<3>() {
            rest = tail;
            let n = varint(&mut rest).ok_or_else(bad)?;
            let mut list = Vec::with_capacity(n);
            for _ in 0..n {
                list.push(list.last().copied().unwrap_or(0) + varint(&mut rest).ok_or_else(bad)?);
            }
            postings.insert(*t, list);
        }
        Ok(Some(SearchIndex {
            files,
            common,
            postings,
        }))
    }

    /// The contents of `files` that contain `pattern`, and the decode work
    /// (bytes produced by functions) the scan took.
    pub fn search(
        &self,
        pattern: &[u8],
        files: &[Id],
        index: Option<&SearchIndex>,
    ) -> Result<(Vec<Id>, usize), Error> {
        let candidates: Vec<Id> = match index.filter(|_| pattern.len() >= 3) {
            Some(index) => {
                let covered: HashSet<&Id> = index.files.iter().collect();
                let mut hits: Option<HashSet<usize>> = None;
                for w in pattern.windows(3) {
                    let t = [w[0], w[1], w[2]];
                    if index.common.contains(&t) {
                        continue;
                    }
                    let list = index.postings.get(&t);
                    let set: HashSet<usize> = list.into_iter().flatten().copied().collect();
                    hits = Some(match hits {
                        Some(h) => h.intersection(&set).copied().collect(),
                        None => set,
                    });
                }
                // Only common trigrams: no constraint, every content is a candidate.
                let hits = hits.unwrap_or_else(|| (0..index.files.len()).collect());
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
        let mut cx = Cx::default();
        let mut out = Vec::new();
        for id in candidates {
            let bytes = self.rebuild(&id, &mut cx)?;
            if pattern.is_empty() || bytes.windows(pattern.len()).any(|w| w == pattern) {
                out.push(id);
            }
            cx.known.clear();
        }
        Ok((out, cx.work))
    }
}

#[cfg(test)]
mod tests {
    use crate::{MemStore, Repo};

    /// Indexed and unindexed searches return the same contents (common
    /// trigrams included), and the index narrows what is decoded.
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
        // "eta" is in every indexed content: a common trigram, no constraint.
        for pattern in [&b"alpha"[..], b"beta", b"zeta", b"eta", b"ta", b"none"] {
            let full = repo.search(pattern, &ids, None).unwrap().0;
            let fast = repo.search(pattern, &ids, Some(&index)).unwrap().0;
            assert_eq!(full, fast, "{}", String::from_utf8_lossy(pattern));
        }
        assert_eq!(
            repo.search(b"alpha", &ids, Some(&index)).unwrap().0,
            vec![ids[0], ids[3]]
        );
    }
}
