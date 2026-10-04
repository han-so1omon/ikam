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
const SEED: usize = 32;
/// An anchor follows ~1 in 64 positions (top 6 gear-hash bits zero).
const ANCHOR_SHIFT: u32 = 58;
/// Shortest match worth a slice: one slice entry (48 B) plus the extra entry
/// needed to resume the literal run.
pub const MIN_MATCH: usize = 96;
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

const GEAR: [u64; 256] = {
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
fn anchors(bytes: &[u8]) -> impl Iterator<Item = usize> + '_ {
    let mut h = 0u64;
    bytes.iter().enumerate().filter_map(move |(i, &b)| {
        h = (h << 1).wrapping_add(GEAR[b as usize]);
        (h >> ANCHOR_SHIFT == 0 && i + 1 + SEED <= bytes.len()).then_some(i + 1)
    })
}

fn seed_key(window: &[u8]) -> u64 {
    u64::from_le_bytes(blake3::hash(window).as_bytes()[..8].try_into().unwrap())
}

/// Seed index over stored blobs. A projection: rebuildable from the blobs.
#[derive(Default)]
pub struct Index {
    seeds: HashMap<u64, Vec<(Id, usize)>>,
}

impl Index {
    pub fn add(&mut self, id: Id, bytes: &[u8]) {
        for p in anchors(bytes) {
            let slot = self.seeds.entry(seed_key(&bytes[p..p + SEED])).or_default();
            if slot.len() < MAX_CANDIDATES && !slot.iter().any(|(i, _)| *i == id) {
                slot.push((id, p));
            }
        }
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
        for (id, _) in index
            .seeds
            .get(&seed_key(&input[q..q + SEED]))
            .into_iter()
            .flatten()
        {
            *hits.entry(*id).or_default() += 1;
        }
    }
    let mut ranked: Vec<(Id, usize)> = hits.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.truncate(k);
    (ranked, seeds)
}

/// Worth an alignment: a neighbour sharing at least 3 seeds and at least a
/// quarter of the input's seeds. Aligning dissimilar contents costs time
/// and cannot yield a useful template.
pub fn similar(shared: usize, seeds: usize) -> bool {
    shared >= 3 && shared * 4 >= seeds
}

/// Propose a plan for `input`. `load` returns a stored blob's bytes.
pub fn plan(input: &[u8], index: &Index, load: impl Fn(&Id) -> Option<Vec<u8>>) -> Vec<Part> {
    let mut cache: HashMap<Id, Option<Vec<u8>>> = HashMap::new();
    let (mut parts, mut cursor) = (Vec::new(), 0);
    for q in anchors(input) {
        if q < cursor {
            continue;
        }
        let Some(candidates) = index.seeds.get(&seed_key(&input[q..q + SEED])) else {
            continue;
        };
        let mut best: Option<(usize, Id, usize, usize)> = None;
        for &(src, p) in candidates {
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

fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
}
