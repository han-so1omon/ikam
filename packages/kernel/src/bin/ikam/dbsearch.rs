//! Ranked and semantic search for `ikam db serve` (E358): the kernel's
//! BM25 (E324, kept up by edits as `Bm25Live`, E329), a vector index of
//! rows holding embeddings (an exact scan, or E332's graph as an option)
//! and their fusion (E312, E326), behind three routes.
//!
//! - `GET /db/NAME/search?q=TEXT&prefix=P[&k=N]`: the rows under `P` best
//!   matching `TEXT` by BM25 (values read as text).
//! - `POST /db/NAME/vsearch?prefix=VP[&k=N]`: the rows under `VP` nearest a
//!   query vector, by dot product. Rows under `VP` hold unit vectors as
//!   little-endian f32; the body is the query, the same way or as a JSON
//!   array of numbers. The server holds no embedding model: the client
//!   embeds rows and queries with its own.
//! - `POST /db/NAME/search?q=TEXT&prefix=P&vectors=VP[&k=N][&alpha=A]
//!   [&fuse=rrf]`, the query vector as body: both lists fused, convex
//!   with `A` on the keyword list (default `fusion::DEFAULT_ALPHA`), or
//!   reciprocal rank fusion. A row `P` + `S` and a vector `VP` + `S` are
//!   the same item.
//!
//! Answers are `[{"key":"...","score":S},...]`, best first, at the head
//! (no `at=`). An index is built on the first query of its database and
//! prefix, kept in memory, and brought to the head on later queries from
//! the rows that changed; a restarted server builds it again. The BM25
//! view's postings are written to the store as content no commit names.
//! Nothing is built for a database nobody searches.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ikam_kernel::bm25::Bm25Live;
use ikam_kernel::hnsw::HnswIndex;
use ikam_kernel::{Db, Error, Repo, Store, fusion, vectors};

use crate::http::{Request, Response};
use crate::json::{self, Json, quote};

/// The most rows one query may ask for.
const MAX_K: usize = 1000;

/// Rows taken from each list before fusing.
const FUSE_DEPTH: usize = 100;

/// HNSW settings E332 suggests: m 16, ef_build 100, search beam 80.
const HNSW_M: usize = 16;
const HNSW_EF_BUILD: usize = 100;
const HNSW_EF: usize = 80;

/// How vector rows are searched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VectorIndex {
    /// Every row scored: exact, in memory (dim x 4 bytes a row).
    #[default]
    Exact,
    /// E332's graph (option): about 0.93 of the exact top 10, scoring a
    /// few thousand rows a query whatever the table's size.
    Hnsw,
}

/// Vectors of the rows under one prefix.
enum Vecs {
    Exact {
        keys: Vec<String>,
        flat: Vec<f32>,
        pos: HashMap<String, usize>,
    },
    Hnsw(Box<HnswIndex>),
}

/// One kept vector index and the database it is at.
struct KeptVecs {
    at: Db,
    dim: usize,
    vecs: Vecs,
}

type Key = (String, String);

/// The indexes a server keeps, by database name and prefix.
#[derive(Default)]
pub struct Kept {
    bm25: Mutex<HashMap<Key, (Db, Bm25Live)>>,
    vecs: Mutex<HashMap<Key, Arc<Mutex<KeptVecs>>>>,
    pub index: VectorIndex,
}

fn fail(e: Error) -> Response {
    Response::text(500, e.to_string())
}

fn f32s(b: &[u8]) -> Option<Vec<f32>> {
    b.len().is_multiple_of(4).then(|| vectors::to_f32s(b))
}

impl Vecs {
    fn new(index: VectorIndex, dim: usize) -> Vecs {
        match index {
            VectorIndex::Exact => Vecs::Exact {
                keys: Vec::new(),
                flat: Vec::new(),
                pos: HashMap::new(),
            },
            VectorIndex::Hnsw => Vecs::Hnsw(Box::new(
                HnswIndex::new(dim, HNSW_M, HNSW_EF_BUILD).with_lanes(true),
            )),
        }
    }

    fn remove(&mut self, key: &str, dim: usize) {
        match self {
            Vecs::Exact { keys, flat, pos } => {
                let Some(i) = pos.remove(key) else { return };
                let last = keys.len() - 1;
                keys.swap_remove(i);
                if i != last {
                    let (a, b) = flat.split_at_mut(last * dim);
                    a[i * dim..][..dim].copy_from_slice(&b[..dim]);
                    pos.insert(keys[i].clone(), i);
                }
                flat.truncate(last * dim);
            }
            Vecs::Hnsw(h) => {
                h.remove(key);
            }
        }
    }

    fn set(&mut self, key: String, v: &[f32], dim: usize) {
        self.remove(&key, dim);
        match self {
            Vecs::Exact { keys, flat, pos } => {
                pos.insert(key.clone(), keys.len());
                keys.push(key);
                flat.extend_from_slice(v);
            }
            Vecs::Hnsw(h) => h.insert(key, v),
        }
    }

    fn top(&mut self, q: &[f32], k: usize, dim: usize) -> Vec<(String, f32)> {
        match self {
            Vecs::Exact { keys, flat, .. } => {
                let mut s: Vec<(f32, usize)> = flat
                    .chunks_exact(dim)
                    .enumerate()
                    .map(|(i, v)| (vectors::dot_lanes(q, v), i))
                    .collect();
                let k = k.min(s.len());
                if k == 0 {
                    return Vec::new();
                }
                let by = |a: &(f32, usize), b: &(f32, usize)| {
                    b.0.total_cmp(&a.0).then_with(|| keys[a.1].cmp(&keys[b.1]))
                };
                s.select_nth_unstable_by(k - 1, by);
                s.truncate(k);
                s.sort_by(by);
                s.into_iter().map(|(x, i)| (keys[i].clone(), x)).collect()
            }
            Vecs::Hnsw(h) => h.search(q, k, HNSW_EF.max(k)),
        }
    }
}

/// `k`, `q` and `prefix` of a request.
fn params(req: &Request, prefix: &str) -> Result<(usize, String), Response> {
    let k = match req.param("k") {
        None => 10,
        Some(n) => n
            .parse::<usize>()
            .map_err(|_| Response::text(400, "k=N"))?
            .min(MAX_K),
    };
    let p = req
        .param(prefix)
        .ok_or_else(|| Response::text(400, format!("{prefix}=P is needed")))?;
    Ok((k, p))
}

/// The query vector in a body: f32 little endian, or a JSON array.
fn query_vector(req: &Request) -> Result<Vec<f32>, Response> {
    let bad = || Response::text(400, "the body is the query vector: f32 LE or a JSON array");
    if req
        .header("content-type")
        .is_some_and(|t| t.starts_with("application/json"))
    {
        let text = std::str::from_utf8(&req.body).map_err(|_| bad())?;
        let j = json::parse(text).map_err(|_| bad())?;
        let mut out = Vec::new();
        for x in j.as_arr() {
            let Json::Num(n) = x else { return Err(bad()) };
            out.push(*n as f32);
        }
        if out.is_empty() {
            return Err(bad());
        }
        return Ok(out);
    }
    f32s(&req.body).filter(|v| !v.is_empty()).ok_or_else(bad)
}

fn hits(list: &[(String, f32)], k: usize) -> Response {
    let items: Vec<String> = list
        .iter()
        .take(k)
        .map(|(key, s)| format!("{{\"key\":{},\"score\":{s}}}", quote(key)))
        .collect();
    Response::new(200).body("application/json", format!("[{}]", items.join(",")))
}

impl Kept {
    /// The BM25 view of `prefix` in `name`, at `db` (built or updated under
    /// the writer's lock, which the caller holds as `w`).
    pub fn bm25_at<S: Store>(
        &self,
        w: &mut Repo<S>,
        name: &str,
        prefix: &str,
        db: &Db,
    ) -> Result<Bm25Live, Error> {
        let mut kept = self.bm25.lock().unwrap();
        let key = (name.to_string(), prefix.to_string());
        let v = match kept.get(&key) {
            Some((at, v)) if at == db => return Ok(*v),
            Some((at, v)) => w.bm25_live_update(v, at, db, prefix)?,
            None => {
                let rows = w.db_count_prefix(db, prefix)?;
                w.bm25_live_build(db, prefix, (rows / 20).max(1000))?
            }
        };
        kept.insert(key, (*db, v));
        Ok(v)
    }

    /// The vector index of `prefix` in `name`, brought to `db`.
    fn vecs_at<S: Store>(
        &self,
        r: &Repo<S>,
        name: &str,
        prefix: &str,
        db: &Db,
    ) -> Result<Arc<Mutex<KeptVecs>>, Response> {
        let key = (name.to_string(), prefix.to_string());
        let slot = self.vecs.lock().unwrap().get(&key).cloned();
        // An index built while the prefix held no vectors has no dimension
        // yet: build it again, from the rows, once the database changes.
        let slot = slot.filter(|s| {
            let k = s.lock().unwrap();
            k.dim > 0 || k.at == *db
        });
        let slot = match slot {
            Some(s) => s,
            None => {
                let rows = r.db_prefix(db, prefix, usize::MAX, false).map_err(fail)?;
                let dim = rows.first().map_or(0, |(_, v)| v.len() / 4);
                let mut vecs = Vecs::new(self.index, dim);
                for (k, v) in rows {
                    let v = f32s(&v).filter(|v| v.len() == dim).ok_or_else(|| {
                        Response::text(422, format!("{k:?} is not a vector of {dim}"))
                    })?;
                    vecs.set(k, &v, dim);
                }
                let s = Arc::new(Mutex::new(KeptVecs { at: *db, dim, vecs }));
                self.vecs.lock().unwrap().insert(key, s.clone());
                return Ok(s);
            }
        };
        {
            let mut k = slot.lock().unwrap();
            if k.at != *db {
                for (row, _, after) in r.graph_diff(&k.at.kv, &db.kv).map_err(fail)? {
                    if !row.starts_with(prefix) {
                        continue;
                    }
                    let dim = k.dim;
                    match after {
                        None => k.vecs.remove(&row, dim),
                        Some(_) => {
                            let v = r.db_get(db, &row).map_err(fail)?.unwrap_or_default();
                            let v = f32s(&v).filter(|v| v.len() == dim).ok_or_else(|| {
                                Response::text(422, format!("{row:?} is not a vector of {dim}"))
                            })?;
                            k.vecs.set(row, &v, dim);
                        }
                    }
                }
                k.at = *db;
            }
        }
        Ok(slot)
    }

    fn vector_top<S: Store>(
        &self,
        r: &Repo<S>,
        name: &str,
        prefix: &str,
        db: &Db,
        q: &[f32],
        k: usize,
    ) -> Result<Vec<(String, f32)>, Response> {
        let slot = self.vecs_at(r, name, prefix, db)?;
        let mut kept = slot.lock().unwrap();
        if kept.dim == 0 {
            return Ok(Vec::new());
        }
        if q.len() != kept.dim {
            return Err(Response::text(
                400,
                format!("the query has {} numbers, rows {}", q.len(), kept.dim),
            ));
        }
        let dim = kept.dim;
        Ok(kept.vecs.top(q, k, dim))
    }
}

/// A search request's parameters.
pub struct Query {
    k: usize,
    /// Text rows (`search`) or vector rows (`vsearch`).
    prefix: String,
    q: Option<String>,
    /// Vector rows, and the query vector.
    vectors: Option<(String, Vec<f32>)>,
    vsearch: bool,
}

impl Query {
    /// The parameters of `search` (`vsearch` false) or `vsearch`.
    pub fn parse(req: &Request, vsearch: bool) -> Result<Query, Response> {
        let (k, prefix) = params(req, "prefix")?;
        let q = req.param("q");
        if !vsearch && q.is_none() {
            return Err(Response::text(400, "search?q=TEXT&prefix=P"));
        }
        let vp = if vsearch {
            Some(prefix.clone())
        } else {
            req.param("vectors")
        };
        let vectors = match vp {
            Some(vp) => Some((vp, query_vector(req)?)),
            None => None,
        };
        Ok(Query {
            k,
            prefix,
            q,
            vectors,
            vsearch,
        })
    }

    /// The BM25 rows' prefix, when the query ranks by keywords.
    pub fn keyword_prefix(&self) -> Option<&str> {
        (!self.vsearch).then_some(self.prefix.as_str())
    }
}

impl Kept {
    /// The answer to `q` at `db`; `bm25` is the keyword view at `db` when
    /// `q` has keywords.
    pub fn answer<S: Store>(
        &self,
        r: &Repo<S>,
        name: &str,
        db: &Db,
        bm25: Option<Bm25Live>,
        q: &Query,
        req: &Request,
    ) -> Response {
        let fused = q.vectors.is_some() && !q.vsearch;
        let depth = if fused { FUSE_DEPTH.max(q.k) } else { q.k };
        let keyword = match (&bm25, &q.q) {
            (Some(v), Some(text)) => match r.bm25_live_search(v, text, depth) {
                Ok(l) => l,
                Err(e) => return fail(e),
            },
            _ => Vec::new(),
        };
        let Some((vp, qv)) = &q.vectors else {
            return hits(&keyword, q.k);
        };
        let near = match self.vector_top(r, name, vp, db, qv, depth) {
            Ok(l) => l,
            Err(resp) => return resp,
        };
        if q.vsearch {
            return hits(&near, q.k);
        }
        // A vector `vp` + S stands for the row `prefix` + S.
        let near: Vec<(String, f32)> = near
            .into_iter()
            .filter_map(|(key, s)| {
                Some((format!("{}{}", q.prefix, key.strip_prefix(vp.as_str())?), s))
            })
            .collect();
        let fused = match req.param("fuse").as_deref() {
            Some("rrf") => {
                let keys = |l: &[(String, f32)]| l.iter().map(|x| x.0.clone()).collect();
                fusion::rrf(&[keys(&keyword), keys(&near)], 60.0)
            }
            Some("convex") | None => {
                let alpha = match req.param("alpha").map(|a| a.parse::<f32>()) {
                    None => fusion::DEFAULT_ALPHA,
                    Some(Ok(a)) if (0.0..=1.0).contains(&a) => a,
                    Some(_) => return Response::text(400, "alpha is 0 to 1"),
                };
                fusion::convex(&keyword, &near, alpha)
            }
            Some(_) => return Response::text(400, "fuse=convex or fuse=rrf"),
        };
        hits(&fused, q.k)
    }
}
