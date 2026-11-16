//! Vector indexes kept as views of the database (E253): rows whose values
//! are little-endian f32 vectors (unit length), indexed as an IVF (Jégou et
//! al.'s inverted file: vectors assigned to the nearest of `nlist` k-means
//! centroids), each vector kept as an int8 code (scalar quantization). A
//! search probes the `nprobe` nearest lists, scores their codes, and
//! rescores the best `rescore` with the exact vectors read from `kv`.
//! With `nlist` 1 it is a flat int8 scan.
//!
//! The index is a view: lists are a graph of `list US rowkey` nodes, each
//! pointing at its code in a packed content; an update edits only the rows
//! in the `kv` diff (an insert joins its nearest list, a delete leaves its
//! list). Centroids stay as trained: data that drifts from them needs a
//! rebuild, which the caller decides.
//!
//! Opt-in: nothing in the kernel indexes vectors unless a caller asks.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Arg, Db, Error, Id, Keys, Node, Repo, Store, Target};

const US: char = '\u{1f}';

/// Candidates scored by code, and the code packs read so far.
type Scored = (Vec<(f32, String)>, BTreeMap<Id, Vec<u8>>);

type CodeDot = fn(&[f32], &[u8]) -> f32;
type Dot = fn(&[f32], &[f32]) -> f32;

/// An IVF loaded once (E325): centroids, each list's rows and their codes
/// in memory, so a query reads only the rows it rescores. With `vectors`
/// (`ivf_load_with(.., true)`) the rows' exact vectors are kept too and a
/// query reads nothing.
pub struct IvfIndex {
    dim: usize,
    cents: Vec<f32>,
    /// Per list: row keys, and their codes back to back.
    lists: Vec<(Vec<String>, Vec<u8>)>,
    vectors: Option<BTreeMap<String, Vec<f32>>>,
    /// E327: score codes and centroids with 16 independent sums.
    lanes: bool,
}

/// An IVF view: `centroids` is a content of `nlist * dim` f32s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ivf {
    pub centroids: Id,
    pub lists: Id,
    pub nlist: usize,
    pub dim: usize,
}

pub fn to_f32s(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

pub fn from_f32s(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// The int8 code of a unit vector: each component times 127, rounded.
pub fn quantize(v: &[f32]) -> Vec<u8> {
    v.iter()
        .map(|x| ((x * 127.0).round().clamp(-127.0, 127.0) as i8) as u8)
        .collect()
}

fn code_dot(q: &[f32], code: &[u8]) -> f32 {
    q.iter()
        .zip(code)
        .map(|(x, &c)| x * f32::from(c as i8))
        .sum()
}

/// `code_dot` with 16 independent sums added at the end (E327). The
/// compiler may vectorize it, since the order of the additions is written
/// out; the result can differ from `code_dot` in the last bits.
///
/// E339: on x86-64 with AVX2 the same loop runs compiled for AVX2 (wider
/// registers, no fused multiply-add), so the result is the same bits.
pub fn code_dot_lanes(q: &[f32], code: &[u8]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        // Safety: AVX2 is present, checked just above.
        return unsafe { avx2::code_dot_lanes(q, code) };
    }
    code_dot_lanes_body(q, code)
}

#[inline(always)]
fn code_dot_lanes_body(q: &[f32], code: &[u8]) -> f32 {
    let mut acc = [0f32; 16];
    let (qs, cs) = (q.chunks_exact(16), code.chunks_exact(16));
    let tail: f32 = qs
        .remainder()
        .iter()
        .zip(cs.remainder())
        .map(|(x, &c)| x * f32::from(c as i8))
        .sum();
    for (q, c) in qs.zip(cs) {
        for l in 0..16 {
            acc[l] += q[l] * f32::from(c[l] as i8);
        }
    }
    acc.iter().sum::<f32>() + tail
}

/// `dot` with 16 independent sums (E327); with AVX2 as `code_dot_lanes`
/// (E339), the same bits.
pub fn dot_lanes(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        // Safety: AVX2 is present, checked just above.
        return unsafe { avx2::dot_lanes(a, b) };
    }
    dot_lanes_body(a, b)
}

#[inline(always)]
fn dot_lanes_body(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 16];
    let (xs, ys) = (a.chunks_exact(16), b.chunks_exact(16));
    let tail: f32 = xs
        .remainder()
        .iter()
        .zip(ys.remainder())
        .map(|(x, y)| x * y)
        .sum();
    for (x, y) in xs.zip(ys) {
        for l in 0..16 {
            acc[l] += x[l] * y[l];
        }
    }
    acc.iter().sum::<f32>() + tail
}

/// The lanes loops compiled for AVX2 only (not FMA: Rust never fuses a
/// multiply and an add on its own, so each lane rounds as the portable loop).
#[cfg(target_arch = "x86_64")]
mod avx2 {
    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn code_dot_lanes(q: &[f32], code: &[u8]) -> f32 {
        super::code_dot_lanes_body(q, code)
    }

    #[target_feature(enable = "avx2")]
    pub(super) unsafe fn dot_lanes(a: &[f32], b: &[f32]) -> f32 {
        super::dot_lanes_body(a, b)
    }
}

fn nearest(cents: &[f32], dim: usize, v: &[f32]) -> usize {
    (0..cents.len() / dim)
        .max_by(|&a, &b| {
            dot(&cents[a * dim..][..dim], v).total_cmp(&dot(&cents[b * dim..][..dim], v))
        })
        .unwrap_or(0)
}

/// `k` centroids from `sample` by spherical k-means (`iters` rounds), the
/// first `k` sample vectors as the seeds.
pub fn kmeans(sample: &[Vec<f32>], k: usize, iters: usize) -> Vec<f32> {
    let dim = sample.first().map_or(0, Vec::len);
    let mut cents: Vec<f32> = sample.iter().take(k).flatten().copied().collect();
    for _ in 0..iters {
        let mut sums = vec![0f32; cents.len()];
        for v in sample {
            let c = nearest(&cents, dim, v);
            for (s, x) in sums[c * dim..][..dim].iter_mut().zip(v) {
                *s += x;
            }
        }
        for c in 0..cents.len() / dim {
            let s = &mut sums[c * dim..][..dim];
            let n = dot(s, s).sqrt();
            if n > 0.0 {
                s.iter_mut().for_each(|x| *x /= n);
                cents[c * dim..][..dim].copy_from_slice(s);
            }
        }
    }
    cents
}

fn list_label(c: usize, row: &str) -> String {
    format!("{c:06}{US}{row}")
}

impl<S: Store> Repo<S> {
    /// Nodes for `rows` (key, vector) in their nearest lists, their codes
    /// packed into one content.
    fn ivf_nodes(
        &mut self,
        cents: &[f32],
        dim: usize,
        rows: &[(String, Vec<f32>)],
    ) -> Result<Vec<Node>, Error> {
        let mut pack = Vec::with_capacity(rows.len() * dim);
        let lists: Vec<usize> = rows.iter().map(|(_, v)| nearest(cents, dim, v)).collect();
        for (_, v) in rows {
            pack.extend(quantize(v));
        }
        let id = self.put_derived(&pack)?;
        Ok(rows
            .iter()
            .zip(lists)
            .enumerate()
            .map(|(i, ((k, _), c))| Node {
                label: list_label(c, k),
                target: Target::Content(Arg::Range {
                    id,
                    start: (i * dim) as u64,
                    len: dim as u64,
                }),
                edges: Vec::new(),
            })
            .collect())
    }

    fn vector_rows(&self, nodes: &[Node]) -> Result<Vec<(String, Vec<f32>)>, Error> {
        nodes
            .iter()
            .map(|n| Ok((n.label.clone(), to_f32s(&self.node_value(n)?))))
            .collect()
    }

    /// An IVF of the rows under `prefix`, its centroids trained on `sample`.
    pub fn ivf_build(
        &mut self,
        db: &Db,
        prefix: &str,
        cents: &[f32],
        dim: usize,
    ) -> Result<Ivf, Error> {
        let rows = self.vector_rows(&self.graph_prefix(&db.kv, prefix)?)?;
        let nodes = self.ivf_nodes(cents, dim, &rows)?;
        Ok(Ivf {
            centroids: self.put_derived(&from_f32s(cents))?,
            lists: self.put_graph_with(nodes, Keys::Any)?,
            nlist: cents.len() / dim,
            dim,
        })
    }

    /// Bring `ivf`, kept at `old`, up to `new`: rows in the diff leave the
    /// list they were in and join their nearest list.
    pub fn ivf_update(
        &mut self,
        ivf: &Ivf,
        old: &Db,
        new: &Db,
        prefix: &str,
    ) -> Result<Ivf, Error> {
        let cents = to_f32s(&self.read_content(&ivf.centroids)?);
        let (mut edits, mut fresh) = (BTreeMap::new(), Vec::new());
        for (row, before, after) in self.graph_diff(&old.kv, &new.kv)? {
            if !row.starts_with(prefix) {
                continue;
            }
            if let Some(n) = before {
                let c = nearest(&cents, ivf.dim, &to_f32s(&self.node_value(&n)?));
                edits.insert(list_label(c, &row), None);
            }
            if let Some(n) = after {
                fresh.push((row, to_f32s(&self.node_value(&n)?)));
            }
        }
        for n in self.ivf_nodes(&cents, ivf.dim, &fresh)? {
            edits.insert(n.label.clone(), Some(n));
        }
        Ok(Ivf {
            lists: self.graph_edit_with(&ivf.lists, edits, Keys::Any)?,
            ..*ivf
        })
    }

    /// The `k` rows nearest `q` (by dot product): `nprobe` lists scored by
    /// code, the best `rescore` rescored with their vectors in `db`.
    pub fn ivf_search(
        &self,
        ivf: &Ivf,
        db: &Db,
        q: &[f32],
        k: usize,
        nprobe: usize,
        rescore: usize,
    ) -> Result<Vec<(String, f32)>, Error> {
        let cents = to_f32s(&self.read_content(&ivf.centroids)?);
        let mut order: Vec<usize> = (0..ivf.nlist).collect();
        order.sort_by(|&a, &b| {
            dot(&cents[b * ivf.dim..][..ivf.dim], q)
                .total_cmp(&dot(&cents[a * ivf.dim..][..ivf.dim], q))
        });
        let (mut scored, mut packs): Scored = (Vec::new(), BTreeMap::new());
        for &c in order.iter().take(nprobe) {
            for n in self.graph_prefix(&ivf.lists, &format!("{c:06}{US}"))? {
                let Target::Content(Arg::Range { id, start, len }) = n.target else {
                    return Err(Error::Decode("ivf code"));
                };
                if let std::collections::btree_map::Entry::Vacant(e) = packs.entry(id) {
                    e.insert(self.read_content(&id)?);
                }
                let code = &packs[&id][start as usize..(start + len) as usize];
                let row = n
                    .label
                    .split_once(US)
                    .ok_or(Error::Decode("ivf label"))?
                    .1
                    .to_string();
                scored.push((code_dot(q, code), row));
            }
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        let keep: BTreeSet<String> = scored
            .into_iter()
            .take(rescore.max(k))
            .map(|s| s.1)
            .collect();
        let mut exact: Vec<(String, f32)> = self
            .graph_nodes_in(&db.kv, &keep)?
            .iter()
            .map(|n| Ok((n.label.clone(), dot(q, &to_f32s(&self.node_value(n)?)))))
            .collect::<Result<_, Error>>()?;
        exact.sort_by(|a, b| b.1.total_cmp(&a.1));
        exact.truncate(k);
        Ok(exact)
    }
}

impl IvfIndex {
    /// Score with 16 independent sums (E327, off by default): the same
    /// rows unless two scores tie within float rounding.
    pub fn with_lanes(mut self, on: bool) -> Self {
        self.lanes = on;
        self
    }

    fn probe(&self, q: &[f32], nprobe: usize, rescore: usize) -> Vec<String> {
        let dim = self.dim;
        let (cdot, vdot): (CodeDot, Dot) = if self.lanes {
            (code_dot_lanes, dot_lanes)
        } else {
            (code_dot, dot)
        };
        let cent: Vec<f32> = (0..self.lists.len())
            .map(|c| vdot(&self.cents[c * dim..][..dim], q))
            .collect();
        let mut order: Vec<usize> = (0..self.lists.len()).collect();
        order.sort_by(|&a, &b| cent[b].total_cmp(&cent[a]));
        let mut scored: Vec<(f32, &String)> = Vec::new();
        for &c in order.iter().take(nprobe) {
            let (rows, codes) = &self.lists[c];
            for (row, code) in rows.iter().zip(codes.chunks_exact(dim)) {
                scored.push((cdot(q, code), row));
            }
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored
            .into_iter()
            .take(rescore)
            .map(|s| s.1.clone())
            .collect()
    }

    /// The rows `ivf_search` would return, from the loaded lists; exact
    /// vectors come from memory if kept, else from `db`.
    pub fn search<S: Store>(
        &self,
        repo: &Repo<S>,
        db: &Db,
        q: &[f32],
        k: usize,
        nprobe: usize,
        rescore: usize,
    ) -> Result<Vec<(String, f32)>, Error> {
        let keep = self.probe(q, nprobe, rescore.max(k));
        let mut exact: Vec<(String, f32)> = match &self.vectors {
            Some(v) => keep
                .into_iter()
                .map(|r| {
                    let s = dot(q, &v[&r]);
                    (r, s)
                })
                .collect(),
            None => {
                let keep: BTreeSet<String> = keep.into_iter().collect();
                repo.graph_nodes_in(&db.kv, &keep)?
                    .iter()
                    .map(|n| Ok((n.label.clone(), dot(q, &to_f32s(&repo.node_value(n)?)))))
                    .collect::<Result<_, Error>>()?
            }
        };
        exact.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        exact.truncate(k);
        Ok(exact)
    }
}

impl<S: Store> Repo<S> {
    /// Load `ivf` for repeated queries; with `vectors`, also every row's
    /// exact vector from `db` (dim x 4 bytes a row in memory).
    pub fn ivf_load_with(&self, ivf: &Ivf, db: &Db, vectors: bool) -> Result<IvfIndex, Error> {
        let cents = to_f32s(&self.read_content(&ivf.centroids)?);
        let mut lists = vec![(Vec::new(), Vec::new()); ivf.nlist];
        let mut packs: BTreeMap<Id, Vec<u8>> = BTreeMap::new();
        for n in self.graph_nodes(&ivf.lists)? {
            let Target::Content(Arg::Range { id, start, len }) = n.target else {
                return Err(Error::Decode("ivf code"));
            };
            if let std::collections::btree_map::Entry::Vacant(e) = packs.entry(id) {
                e.insert(self.read_content(&id)?);
            }
            let (c, row) = n.label.split_once(US).ok_or(Error::Decode("ivf label"))?;
            let c: usize = c.parse().map_err(|_| Error::Decode("ivf list"))?;
            let list = lists.get_mut(c).ok_or(Error::Decode("ivf list"))?;
            list.0.push(row.to_string());
            list.1
                .extend_from_slice(&packs[&id][start as usize..(start + len) as usize]);
        }
        let vectors = if vectors {
            let rows: BTreeSet<String> = lists.iter().flat_map(|l| l.0.iter().cloned()).collect();
            Some(
                self.graph_nodes_in(&db.kv, &rows)?
                    .iter()
                    .map(|n| Ok((n.label.clone(), to_f32s(&self.node_value(n)?))))
                    .collect::<Result<_, Error>>()?,
            )
        } else {
            None
        };
        Ok(IvfIndex {
            dim: ivf.dim,
            cents,
            lists,
            vectors,
            lanes: false,
        })
    }

    pub fn ivf_load(&self, ivf: &Ivf, db: &Db) -> Result<IvfIndex, Error> {
        self.ivf_load_with(ivf, db, false)
    }
}

#[cfg(test)]
mod lanes_tests {
    use super::*;

    /// E339: the dispatched lanes loops give the bits of the portable ones,
    /// for every length up to 300 (tails included) and extreme values.
    #[test]
    fn dispatched_lanes_match_portable_bits() {
        let mut seed = 11u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        for len in 0..300 {
            for round in 0..4 {
                let f = |x: u32| match round {
                    0 => (x % 2001) as f32 / 1000.0 - 1.0,
                    1 => {
                        f32::from_bits(x.rotate_left(7) & 0x7f7f_ffff)
                            * if x & 1 == 0 { 1.0 } else { -1.0 }
                    }
                    _ => ((x % 2001) as f32 - 1000.0) * 1e-30,
                };
                let a: Vec<f32> = (0..len).map(|_| f(next())).collect();
                let b: Vec<f32> = (0..len).map(|_| f(next())).collect();
                let c: Vec<u8> = (0..len).map(|_| next() as u8).collect();
                assert_eq!(
                    dot_lanes(&a, &b).to_bits(),
                    dot_lanes_body(&a, &b).to_bits(),
                    "dot len {len} round {round}"
                );
                assert_eq!(
                    code_dot_lanes(&a, &c).to_bits(),
                    code_dot_lanes_body(&a, &c).to_bits(),
                    "code len {len} round {round}"
                );
            }
        }
    }
}
