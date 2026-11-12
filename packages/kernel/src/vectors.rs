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
    fn store_plain(&mut self, bytes: &[u8]) -> Result<Id, Error> {
        let id = Id::of_content(bytes);
        if !self.has_content(&id)? {
            let encoded = self.encode_plain(bytes);
            self.materialize(id, &encoded, bytes)?;
        }
        Ok(id)
    }

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
        let id = self.store_plain(&pack)?;
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
            .map(|n| Ok((n.label.clone(), to_f32s(&self.kv_value(n)?))))
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
            centroids: self.store_plain(&from_f32s(cents))?,
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
                let c = nearest(&cents, ivf.dim, &to_f32s(&self.kv_value(&n)?));
                edits.insert(list_label(c, &row), None);
            }
            if let Some(n) = after {
                fresh.push((row, to_f32s(&self.kv_value(&n)?)));
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
            .map(|n| Ok((n.label.clone(), dot(q, &to_f32s(&self.kv_value(n)?)))))
            .collect::<Result<_, Error>>()?;
        exact.sort_by(|a, b| b.1.total_cmp(&a.1));
        exact.truncate(k);
        Ok(exact)
    }
}
