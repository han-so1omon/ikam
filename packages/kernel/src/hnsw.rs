//! A graph vector index (E332): Malkov and Yashunin's HNSW (hierarchical
//! navigable small world graphs) over the unit vectors under a prefix of
//! the database, searched by dot product. Each row is a node with up to
//! `m` links a level (`2m` on the bottom level); a search walks down from
//! the top level greedily and searches the bottom level with a beam of
//! `ef` candidates.
//!
//! E331 found the IVF routes queries into a few big lists, so at good
//! recall it scans most rows. A graph scores only the nodes it walks
//! through, a few thousand however the rows cluster.
//!
//! The graph is kept as one content (the links of every node, rows in key
//! order) beside the database it was built from; vectors stay in the
//! database and are read once when the index is loaded. Levels come from
//! a hash of each row's position, so a build is deterministic. Rows can be
//! added to a loaded index; a removed row stays in the graph as a waypoint
//! and is left out of results. A rebuild is the caller's choice.
//!
//! Opt-in: nothing in the kernel builds a graph unless a caller asks.

use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::{BTreeSet, BinaryHeap, HashMap};

use crate::vectors::{dot, dot_lanes, to_f32s};
use crate::{Db, Error, Id, Repo, Store};

/// A stored graph: `links` is the content `HnswIndex::encode` wrote for
/// the `rows` rows under the prefix it was built from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hnsw {
    pub links: Id,
    pub rows: usize,
    pub dim: usize,
    pub m: usize,
}

/// A graph in memory with the rows' vectors.
pub struct HnswIndex {
    dim: usize,
    m: usize,
    ef_build: usize,
    keys: Vec<String>,
    vecs: Vec<f32>,
    /// Per node, per level from 0, the linked nodes.
    links: Vec<Vec<Vec<u32>>>,
    removed: Vec<bool>,
    /// The live node of each key.
    pos: HashMap<String, u32>,
    entry: Option<u32>,
    visit: Visit,
    /// E327: score with 16 independent sums.
    lanes: bool,
    /// Threads `extend` links new rows with (1: one at a time).
    threads: usize,
    /// Vectors scored by the last search, for measurement.
    pub scored: usize,
}

/// Scratch for one search: the search that last saw each node.
#[derive(Default)]
struct Visit {
    seen: Vec<u32>,
    epoch: u32,
}

impl Visit {
    fn start(&mut self, nodes: usize) -> u32 {
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.seen.iter_mut().for_each(|s| *s = 0);
            self.epoch = 1;
        }
        self.seen.resize(nodes, 0);
        self.epoch
    }
}

/// Rows `extend` links at once when it runs on more than one thread.
const BATCH: usize = 1024;

#[derive(Clone, Copy, PartialEq)]
struct Near(f32, u32);
impl Eq for Near {}
impl PartialOrd for Near {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Near {
    /// Higher similarity first; ties by node so the order is total.
    fn cmp(&self, o: &Self) -> Ordering {
        self.0.total_cmp(&o.0).then_with(|| o.1.cmp(&self.1))
    }
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// The top level of node `i`: geometric with ratio 1/m, from a hash.
fn level_of(i: usize, m: usize) -> usize {
    let u = ((splitmix(i as u64) >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
    ((-u.ln()) / (m as f64).ln()).floor().min(16.0) as usize
}

impl HnswIndex {
    /// An empty graph for vectors of `dim`, `m` links a level, and a beam
    /// of `ef_build` while linking.
    pub fn new(dim: usize, m: usize, ef_build: usize) -> Self {
        HnswIndex {
            dim,
            m: m.max(2),
            ef_build: ef_build.max(1),
            keys: Vec::new(),
            vecs: Vec::new(),
            links: Vec::new(),
            removed: Vec::new(),
            pos: HashMap::new(),
            entry: None,
            visit: Visit::default(),
            lanes: false,
            threads: 1,
            scored: 0,
        }
    }

    /// Score with 16 independent sums (E327, off by default).
    pub fn with_lanes(mut self, on: bool) -> Self {
        self.lanes = on;
        self
    }

    /// Link rows added by `extend` on `n` threads, `BATCH` at a time
    /// (off by default: 1). Rows of a batch are linked to the graph as it
    /// was before the batch and to each other by an exact scan, so the
    /// graph differs from one built a row at a time; the build stays
    /// deterministic for a given `n`.
    pub fn with_threads(mut self, n: usize) -> Self {
        self.threads = n.max(1);
        self
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    fn vec(&self, i: u32) -> &[f32] {
        &self.vecs[i as usize * self.dim..][..self.dim]
    }

    fn pair(&self, a: &[f32], b: &[f32]) -> f32 {
        if self.lanes {
            dot_lanes(a, b)
        } else {
            dot(a, b)
        }
    }

    fn sim(&self, q: &[f32], i: u32) -> f32 {
        self.pair(q, self.vec(i))
    }

    fn top(&self) -> usize {
        self.entry.map_or(0, |e| self.links[e as usize].len() - 1)
    }

    /// The `ef` nodes nearest `q` found from `start` on `level`, best
    /// first, and the vectors scored.
    fn search_level(
        &self,
        visit: &mut Visit,
        q: &[f32],
        start: &[Near],
        ef: usize,
        level: usize,
    ) -> (Vec<Near>, usize) {
        let epoch = visit.start(self.keys.len());
        let seen = &mut visit.seen;
        let mut todo: BinaryHeap<Near> = BinaryHeap::new();
        // The results, worst on top.
        let mut best: BinaryHeap<Reverse<Near>> = BinaryHeap::new();
        for &s in start {
            seen[s.1 as usize] = epoch;
            todo.push(s);
            best.push(Reverse(s));
        }
        while best.len() > ef {
            best.pop();
        }
        let mut scored = 0;
        while let Some(c) = todo.pop() {
            let worst = best.peek().map_or(f32::NEG_INFINITY, |w| w.0.0);
            if c.0 < worst && best.len() >= ef {
                break;
            }
            for &n in &self.links[c.1 as usize][level] {
                if seen[n as usize] == epoch {
                    continue;
                }
                seen[n as usize] = epoch;
                let s = self.sim(q, n);
                scored += 1;
                let worst = best.peek().map_or(f32::NEG_INFINITY, |w| w.0.0);
                if best.len() < ef || s > worst {
                    todo.push(Near(s, n));
                    best.push(Reverse(Near(s, n)));
                    if best.len() > ef {
                        best.pop();
                    }
                }
            }
        }
        let mut out: Vec<Near> = best.into_iter().map(|r| r.0).collect();
        out.sort_by(|a, b| b.cmp(a));
        (out, scored)
    }

    /// Of `cands` (best first, similarity to the new node), at most `keep`
    /// that are each nearer the new node than to any already kept, filled
    /// up with the nearest of the rest (Malkov and Yashunin, algorithm 4).
    fn select(&self, cands: &[Near], keep: usize) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::with_capacity(keep);
        let mut pruned = Vec::new();
        for c in cands {
            if out.len() >= keep {
                break;
            }
            let v = self.vec(c.1);
            if out.iter().all(|&r| self.pair(v, self.vec(r)) < c.0) {
                out.push(c.1);
            } else {
                pruned.push(c.1);
            }
        }
        for p in pruned {
            if out.len() >= keep {
                break;
            }
            out.push(p);
        }
        out
    }

    fn max_links(&self, level: usize) -> usize {
        if level == 0 { 2 * self.m } else { self.m }
    }

    /// Push a node for a row, unlinked; its top level.
    fn push(&mut self, key: String, v: &[f32]) -> usize {
        assert_eq!(v.len(), self.dim, "hnsw: vector length");
        self.remove(&key);
        let i = self.keys.len();
        let top = level_of(i, self.m);
        self.pos.insert(key.clone(), i as u32);
        self.keys.push(key);
        self.vecs.extend_from_slice(v);
        self.links.push(vec![Vec::new(); top + 1]);
        self.removed.push(false);
        top
    }

    /// The links node `i` (top level `top`) should get on each of its
    /// levels: the graph searched from the entry, with `extra` candidates
    /// for each level merged in.
    fn plan(
        &self,
        visit: &mut Visit,
        i: u32,
        top: usize,
        extra: &dyn Fn(usize) -> Vec<Near>,
    ) -> Vec<Vec<u32>> {
        let q = self.vec(i);
        let mut out = vec![Vec::new(); top + 1];
        let Some(e) = self.entry else {
            for (level, links) in out.iter_mut().enumerate() {
                let mut c = extra(level);
                c.sort_by(|a, b| b.cmp(a));
                c.truncate(self.ef_build);
                *links = self.select(&c, self.m);
            }
            return out;
        };
        let mut near = vec![Near(self.sim(q, e), e)];
        let graph_top = self.top();
        for level in (top + 1..=graph_top).rev() {
            near = self.search_level(visit, q, &near, 1, level).0;
        }
        for level in (0..=top).rev() {
            let found = if level <= graph_top {
                self.search_level(visit, q, &near, self.ef_build, level).0
            } else {
                Vec::new()
            };
            let mut c = found.clone();
            c.extend(extra(level));
            c.sort_by(|a, b| b.cmp(a));
            c.truncate(self.ef_build);
            out[level] = self.select(&c, self.m);
            if !found.is_empty() {
                near = found;
            }
        }
        out
    }

    /// Give node `i` its planned links and link back to it, pruning lists
    /// past their size.
    fn link(&mut self, i: u32, plan: Vec<Vec<u32>>) {
        for (level, chosen) in plan.into_iter().enumerate() {
            let max = self.max_links(level);
            for &n in &chosen {
                let list = &mut self.links[n as usize][level];
                list.push(i);
                if list.len() > max {
                    let nv = self.vec(n);
                    let mut cands: Vec<Near> = self.links[n as usize][level]
                        .iter()
                        .map(|&o| Near(self.pair(nv, self.vec(o)), o))
                        .collect();
                    cands.sort_by(|a, b| b.cmp(a));
                    self.links[n as usize][level] = self.select(&cands, max);
                }
            }
            // Links to `i` from rows of its batch linked before it stay.
            let before = std::mem::replace(&mut self.links[i as usize][level], chosen);
            for b in before {
                if !self.links[i as usize][level].contains(&b) {
                    self.links[i as usize][level].push(b);
                }
            }
            if self.links[i as usize][level].len() > max {
                let iv = self.vec(i);
                let mut cands: Vec<Near> = self.links[i as usize][level]
                    .iter()
                    .map(|&o| Near(self.pair(iv, self.vec(o)), o))
                    .collect();
                cands.sort_by(|a, b| b.cmp(a));
                self.links[i as usize][level] = self.select(&cands, max);
            }
        }
        let top = self.links[i as usize].len() - 1;
        if self.entry.is_none() || top > self.top() {
            self.entry = Some(i);
        }
    }

    /// Add a row of `dim` components; a key already in the graph is
    /// removed first.
    pub fn insert(&mut self, key: String, v: &[f32]) {
        let top = self.push(key, v);
        let i = (self.keys.len() - 1) as u32;
        let mut visit = std::mem::take(&mut self.visit);
        let plan = self.plan(&mut visit, i, top, &|_| Vec::new());
        self.visit = visit;
        self.link(i, plan);
    }

    /// Add rows in order: one at a time, or on `with_threads` threads a
    /// batch at a time.
    pub fn extend(&mut self, rows: impl IntoIterator<Item = (String, Vec<f32>)>) {
        let mut rows = rows.into_iter().peekable();
        if self.threads <= 1 {
            for (k, v) in rows {
                self.insert(k, &v);
            }
            return;
        }
        // The first rows one at a time, so batches join a graph.
        while self.len() < BATCH {
            match rows.next() {
                Some((k, v)) => self.insert(k, &v),
                None => return,
            }
        }
        while rows.peek().is_some() {
            let first = self.len();
            for (k, v) in rows.by_ref().take(BATCH) {
                self.push(k, &v);
            }
            let ids: Vec<u32> = (first as u32..self.len() as u32).collect();
            let per = ids.len().div_ceil(self.threads);
            let this = &*self;
            let plans: Vec<Vec<Vec<u32>>> = std::thread::scope(|scope| {
                let parts: Vec<_> = ids
                    .chunks(per)
                    .map(|part| {
                        let ids = &ids;
                        scope.spawn(move || {
                            let mut visit = Visit::default();
                            part.iter()
                                .map(|&i| {
                                    let q = this.vec(i);
                                    let extra = |level: usize| -> Vec<Near> {
                                        ids.iter()
                                            .filter(|&&j| {
                                                j != i && this.links[j as usize].len() > level
                                            })
                                            .map(|&j| Near(this.sim(q, j), j))
                                            .collect()
                                    };
                                    let top = this.links[i as usize].len() - 1;
                                    this.plan(&mut visit, i, top, &extra)
                                })
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                parts
                    .into_iter()
                    .flat_map(|h| h.join().expect("hnsw: link thread"))
                    .collect()
            });
            for (i, plan) in ids.into_iter().zip(plans) {
                self.link(i, plan);
            }
        }
    }

    /// Leave `key` out of results; it stays a waypoint. False if absent.
    pub fn remove(&mut self, key: &str) -> bool {
        match self.pos.remove(key) {
            Some(i) => {
                self.removed[i as usize] = true;
                true
            }
            None => false,
        }
    }

    /// The `k` rows nearest `q` by dot product, with a beam of `ef` (at
    /// least `k`) on the bottom level.
    pub fn search(&mut self, q: &[f32], k: usize, ef: usize) -> Vec<(String, f32)> {
        self.scored = 0;
        let Some(e) = self.entry else {
            return Vec::new();
        };
        let mut near = vec![Near(self.sim(q, e), e)];
        let mut visit = std::mem::take(&mut self.visit);
        let mut scored = 1;
        for level in (1..=self.top()).rev() {
            let (n, s) = self.search_level(&mut visit, q, &near, 1, level);
            near = n;
            scored += s;
        }
        let (found, s) = self.search_level(&mut visit, q, &near, ef.max(k), 0);
        self.visit = visit;
        self.scored = scored + s;
        let mut out: Vec<(String, f32)> = found
            .into_iter()
            .filter(|n| !self.removed[n.1 as usize])
            .map(|n| (self.keys[n.1 as usize].clone(), n.0))
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out.truncate(k);
        out
    }

    /// The graph as bytes: m, entry + 1 (0: empty), then per node its
    /// key, its level count shifted left once with the low bit set if it
    /// was removed (then its vector, which the database no longer has),
    /// and per level its link count and links; numbers are varints.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_varint(&mut out, self.m as u64);
        put_varint(&mut out, self.entry.map_or(0, |e| u64::from(e) + 1));
        for (i, node) in self.links.iter().enumerate() {
            let key = self.keys[i].as_bytes();
            put_varint(&mut out, key.len() as u64);
            out.extend_from_slice(key);
            let removed = self.removed[i];
            put_varint(&mut out, (node.len() as u64) << 1 | u64::from(removed));
            if removed {
                out.extend(self.vec(i as u32).iter().flat_map(|x| x.to_le_bytes()));
            }
            for level in node {
                put_varint(&mut out, level.len() as u64);
                for &n in level {
                    put_varint(&mut out, u64::from(n));
                }
            }
        }
        out
    }
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_varint(b: &[u8], at: &mut usize) -> Result<u64, Error> {
    let (mut v, mut shift) = (0u64, 0);
    loop {
        let x = *b.get(*at).ok_or(Error::Decode("hnsw varint"))?;
        *at += 1;
        v |= u64::from(x & 0x7f) << shift;
        if x < 0x80 {
            return Ok(v);
        }
        shift += 7;
        if shift > 63 {
            return Err(Error::Decode("hnsw varint"));
        }
    }
}

impl<S: Store> Repo<S> {
    /// `index` (empty, as `HnswIndex::new` made it, with its options)
    /// filled with the rows under `prefix` in key order, and its stored
    /// form. `m` 16 and `ef_build` 100 are the usual start.
    pub fn hnsw_build(
        &mut self,
        db: &Db,
        prefix: &str,
        mut index: HnswIndex,
    ) -> Result<(Hnsw, HnswIndex), Error> {
        let mut rows = Vec::new();
        for n in self.graph_prefix(&db.kv, prefix)? {
            let v = to_f32s(&self.node_value(&n)?);
            if v.len() != index.dim {
                return Err(Error::Decode("hnsw dim"));
            }
            rows.push((n.label, v));
        }
        index.extend(rows);
        let h = self.hnsw_save(&index)?;
        Ok((h, index))
    }

    /// Store the graph of `index`.
    pub fn hnsw_save(&mut self, index: &HnswIndex) -> Result<Hnsw, Error> {
        Ok(Hnsw {
            links: self.put_derived(&index.encode())?,
            rows: index.len(),
            dim: index.dim,
            m: index.m,
        })
    }

    /// Load `h` with the vectors of its live rows from `db`, the database
    /// it was last brought up to.
    pub fn hnsw_load(&self, h: &Hnsw, db: &Db) -> Result<HnswIndex, Error> {
        let b = self.read_content(&h.links)?;
        let mut at = 0;
        let m = get_varint(&b, &mut at)? as usize;
        let entry = get_varint(&b, &mut at)?;
        let mut index = HnswIndex::new(h.dim, m, 100);
        index.entry = entry.checked_sub(1).map(|e| e as u32);
        let mut want = BTreeSet::new();
        for i in 0..h.rows {
            let n = get_varint(&b, &mut at)? as usize;
            let key = b.get(at..at + n).ok_or(Error::Decode("hnsw key"))?;
            let key = String::from_utf8(key.to_vec()).map_err(|_| Error::Decode("hnsw key"))?;
            at += n;
            let head = get_varint(&b, &mut at)?;
            let removed = head & 1 == 1;
            if removed {
                let v = b
                    .get(at..at + 4 * h.dim)
                    .ok_or(Error::Decode("hnsw vector"))?;
                index.vecs.extend(to_f32s(v));
                at += 4 * h.dim;
            } else {
                index.vecs.extend(std::iter::repeat_n(0.0, h.dim));
                want.insert(key.clone());
                index.pos.insert(key.clone(), i as u32);
            }
            let mut node = Vec::with_capacity((head >> 1) as usize);
            for _ in 0..head >> 1 {
                let n = get_varint(&b, &mut at)? as usize;
                let mut level = Vec::with_capacity(n.min(1024));
                for _ in 0..n {
                    let x = get_varint(&b, &mut at)?;
                    if x >= h.rows as u64 {
                        return Err(Error::Decode("hnsw link"));
                    }
                    level.push(x as u32);
                }
                node.push(level);
            }
            if node.is_empty() {
                return Err(Error::Decode("hnsw node"));
            }
            index.keys.push(key);
            index.links.push(node);
            index.removed.push(removed);
        }
        if index.entry.is_some_and(|e| e as usize >= h.rows) || at != b.len() {
            return Err(Error::Decode("hnsw graph"));
        }
        let nodes = self.graph_nodes_in(&db.kv, &want)?;
        if nodes.len() != want.len() {
            return Err(Error::Decode("hnsw rows"));
        }
        for n in &nodes {
            let v = to_f32s(&self.node_value(n)?);
            if v.len() != h.dim {
                return Err(Error::Decode("hnsw dim"));
            }
            let i = index.pos[&n.label] as usize;
            index.vecs[i * h.dim..][..h.dim].copy_from_slice(&v);
        }
        Ok(index)
    }

    /// Bring a loaded `index`, kept at `old`, up to `new`: rows in the
    /// diff under `prefix` leave results and their new values are added.
    pub fn hnsw_update(
        &self,
        index: &mut HnswIndex,
        old: &Db,
        new: &Db,
        prefix: &str,
    ) -> Result<(), Error> {
        for (row, _, after) in self.graph_diff(&old.kv, &new.kv)? {
            if !row.starts_with(prefix) {
                continue;
            }
            match after {
                Some(n) => {
                    let v = to_f32s(&self.node_value(&n)?);
                    if v.len() != index.dim {
                        return Err(Error::Decode("hnsw dim"));
                    }
                    index.insert(row, &v);
                }
                None => {
                    index.remove(&row);
                }
            }
        }
        Ok(())
    }
}
