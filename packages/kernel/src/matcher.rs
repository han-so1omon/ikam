//! Data-driven dedup: find where new content repeats stored content.
//!
//! Nothing here fixes chunk boundaries. Seeds (short windows at
//! content-defined anchor positions) are only a lookup key for finding
//! *candidate* matches. A match is then extended byte-by-byte in both
//! directions, so slice boundaries land exactly where the content stops
//! agreeing. A match is used only when it is long enough to pay for its slice
//! entry; that is a cost rule, not a boundary rule.
//!
//! The output is a proposal (`Vec<Part>`). `Repo::put_planned` verifies any
//! proposal, so other planners (including AI ones) can replace this one.

use std::collections::HashMap;
use std::ops::Range;

use crate::Id;

/// Seed window length in bytes.
pub(crate) const SEED: usize = 32;
/// An anchor follows ~1 in 64 positions (top 6 gear-hash bits zero).
const ANCHOR_SHIFT: u32 = 58;
/// Shortest match kept as a slice: the seed window. E347: below the 96 bytes
/// one slice entry and the entry resuming the literal run would take raw,
/// a slice still pays on the north corpora (1.2% smaller; the other
/// corpora's stored bytes are unchanged).
pub const MIN_MATCH: usize = SEED;
/// Candidates kept per seed; bounds work on highly repetitive input.
const MAX_CANDIDATES: usize = 4;

/// One piece of a proposed storage plan, in output order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// Bytes of the input that will be stored as new literal data.
    Input(Range<usize>),
    /// Bytes copied from an existing blob.
    Existing { src: Id, start: usize, len: usize },
}

pub(crate) const GEAR: [u64; 256] = {
    let mut table = [0u64; 256];
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut i = 0;
    while i < 256 {
        // splitmix64: fixed, so anchors are stable across builds and machines.
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        table[i] = z ^ (z >> 31);
        i += 1;
    }
    table
};

/// Positions where a seed may start. Depends only on the preceding 64 bytes,
/// so the same content yields the same anchors wherever it appears.
pub(crate) fn anchors(bytes: &[u8]) -> impl Iterator<Item = usize> + '_ {
    let mut h = 0u64;
    bytes.iter().enumerate().filter_map(move |(i, &b)| {
        h = (h << 1).wrapping_add(GEAR[b as usize]);
        (h >> ANCHOR_SHIFT == 0 && i + 1 + SEED <= bytes.len()).then_some(i + 1)
    })
}

pub(crate) fn seed_key(window: &[u8]) -> u64 {
    u64::from_le_bytes(blake3::hash(window).as_bytes()[..8].try_into().unwrap())
}

/// Seed index over stored blobs. A projection: rebuildable from the blobs.
///
/// E390: packed. Each seed's candidates are a chain through one arena of
/// 16-byte entries (blob number, position, next), in the order they were
/// added, and each blob's id is held once. A map from seed to its own
/// small vector of (id, position) took about 210 bytes an entry, mostly
/// the vector's allocation, and was nearly all of a commit's memory (about
/// 3x the input on a wiki dump); this takes about a quarter of that and
/// finds the same candidates in the same order.
#[derive(Default)]
pub struct Index {
    /// Seed key to (first, last) entry of its chain.
    seeds: HashMap<u64, (u32, u32)>,
    arena: Vec<Entry>,
    ids: Vec<Id>,
    numbers: HashMap<Id, u32>,
    entries: usize,
}

#[derive(Clone, Copy)]
struct Entry {
    blob: u32,
    next: u32,
    pos: u64,
}

/// No next entry.
const END: u32 = u32::MAX;

impl Index {
    pub fn add(&mut self, id: Id, bytes: &[u8]) {
        let mut blob = None;
        for p in anchors(bytes) {
            let key = seed_key(&bytes[p..p + SEED]);
            if let Some(&(first, _)) = self.seeds.get(&key) {
                let (mut n, mut at, mut same) = (0, first, false);
                while at != END {
                    let e = self.arena[at as usize];
                    same |= self.ids[e.blob as usize] == id;
                    n += 1;
                    at = e.next;
                }
                if n >= MAX_CANDIDATES || same {
                    continue;
                }
            }
            if self.arena.len() >= END as usize {
                return; // past 4G entries: index no more (same as a full slot)
            }
            let b = *blob.get_or_insert_with(|| {
                *self.numbers.entry(id).or_insert_with(|| {
                    self.ids.push(id);
                    (self.ids.len() - 1) as u32
                })
            });
            let at = self.arena.len() as u32;
            self.arena.push(Entry {
                blob: b,
                next: END,
                pos: p as u64,
            });
            match self.seeds.get_mut(&key) {
                Some(chain) => {
                    self.arena[chain.1 as usize].next = at;
                    chain.1 = at;
                }
                None => {
                    self.seeds.insert(key, (at, at));
                }
            }
            self.entries += 1;
        }
    }

    /// Seed entries held (each about 16 bytes, plus its seed's share of
    /// the map).
    pub fn entries(&self) -> usize {
        self.entries
    }

    /// The blobs and positions indexed under seed `key`, oldest first.
    fn candidates(&self, key: u64) -> impl Iterator<Item = (Id, usize)> + '_ {
        let mut at = self.seeds.get(&key).map_or(END, |c| c.0);
        std::iter::from_fn(move || {
            (at != END).then(|| {
                let e = self.arena[at as usize];
                at = e.next;
                (self.ids[e.blob as usize], e.pos as usize)
            })
        })
    }
}

/// Up to `k` indexed blobs sharing the most seeds with `input`, most
/// first, with their shared-seed counts, plus how many seeds `input` has.
/// Neighbours are found without any minimum match length, so they suit
/// alignment (templates). Entries for blobs no longer stored as bytes may be
/// returned; the caller decides what to do with them.
pub fn neighbors(input: &[u8], index: &Index, k: usize) -> (Vec<(Id, usize)>, usize) {
    let (mut hits, mut seeds) = (HashMap::<Id, usize>::new(), 0);
    for q in anchors(input) {
        seeds += 1;
        for (id, _) in index.candidates(seed_key(&input[q..q + SEED])) {
            *hits.entry(id).or_default() += 1;
        }
    }
    let mut ranked: Vec<(Id, usize)> = hits.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.truncate(k);
    (ranked, seeds)
}

/// Propose a plan for `input`. `load` returns a stored blob's bytes.
pub fn plan(input: &[u8], index: &Index, load: impl Fn(&Id) -> Option<Vec<u8>>) -> Vec<Part> {
    let mut cache: HashMap<Id, Option<Vec<u8>>> = HashMap::new();
    let (mut parts, mut cursor) = (Vec::new(), 0);
    for q in anchors(input) {
        if q < cursor {
            continue;
        }
        let mut best: Option<(usize, Id, usize, usize)> = None;
        for (src, p) in index.candidates(seed_key(&input[q..q + SEED])) {
            let Some(s) = cache.entry(src).or_insert_with(|| load(&src)) else {
                continue;
            };
            if s.get(p..p + SEED) != Some(&input[q..q + SEED]) {
                continue;
            }
            let back = common_suffix(&input[cursor..q], &s[..p]);
            let fwd = common_prefix(&input[q..], &s[p..]);
            if best.is_none_or(|b| back + fwd > b.3) {
                best = Some((q - back, src, p - back, back + fwd));
            }
        }
        if let Some((at, src, start, len)) = best.filter(|b| b.3 >= MIN_MATCH) {
            if at > cursor {
                parts.push(Part::Input(cursor..at));
            }
            parts.push(Part::Existing { src, start, len });
            cursor = at + len;
        }
    }
    if cursor < input.len() || input.is_empty() {
        parts.push(Part::Input(cursor..input.len()));
    }
    parts
}

pub(crate) fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

pub(crate) fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E390: the packed index keeps, per seed, the same candidates in the
    /// same order as a plain map of vectors: up to `MAX_CANDIDATES`, one
    /// per blob, oldest first.
    #[test]
    fn the_packed_index_keeps_the_candidates_a_plain_map_would() {
        let mut state = 7u64;
        let mut byte = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 56) as u8
        };
        // Overlapping windows of one base, plus bytes of their own, so most
        // seeds recur in many blobs and fill their slots.
        let base: Vec<u8> = (0..20_000).map(|_| byte()).collect();
        let blobs: Vec<Vec<u8>> = (0..40)
            .map(|i| {
                let at = (i * 311) % 12_000;
                let mut b = base[at..at + 6000].to_vec();
                b.extend((0..500).map(|_| byte()));
                b
            })
            .collect();
        let (mut packed, mut plain) = (Index::default(), HashMap::<u64, Vec<(Id, usize)>>::new());
        let mut entries = 0;
        for b in blobs.iter().chain(&blobs[..5]) {
            let id = Id::of_content(b);
            packed.add(id, b);
            for p in anchors(b) {
                let slot = plain.entry(seed_key(&b[p..p + SEED])).or_default();
                if slot.len() < MAX_CANDIDATES && !slot.iter().any(|(i, _)| *i == id) {
                    slot.push((id, p));
                    entries += 1;
                }
            }
        }
        assert_eq!(packed.entries(), entries);
        assert!(plain.values().any(|s| s.len() == MAX_CANDIDATES));
        for (key, slot) in &plain {
            assert_eq!(&packed.candidates(*key).collect::<Vec<_>>(), slot);
        }
        assert_eq!(packed.candidates(0x1234).count(), 0);
    }
}
