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
//!
//! `--keep-search` (E369, off by default) keeps each BM25 view on a ref of
//! its own, `search/NAME/p` and the prefix in hex, written after every
//! build or update: a restarted server reads the view back and updates it
//! from the rows changed since, instead of building it again. The ref
//! keeps the view's graphs (and the two databases it was built from)
//! through `gc`. Vector indexes are not kept (an exact one loads in a
//! scan; `--hnsw` builds again).

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

/// E389: rows read at a time while a vector index is first built.
const BUILD_PAGE: usize = 4096;

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
    /// Keep BM25 views on refs across restarts (E369).
    pub keep: bool,
    /// E389 (off: 1): threads that link a graph's first build.
    pub threads: usize,
    /// E394 (off): build a prefix's first BM25 view on a reader, outside
    /// the writer's lock (`build_off_lock` in dbserve.rs).
    pub off_lock: bool,
    /// E394: one build at a time per database and prefix.
    building: Mutex<HashMap<Key, Arc<Mutex<()>>>>,
}

/// A fresh BM25 view of `prefix` at `db`, merged into its base once its
/// delta marks a twentieth of the rows (at least 1,000).
pub fn bm25_build<S: Store>(r: &mut Repo<S>, db: &Db, prefix: &str) -> Result<Bm25Live, Error> {
    let rows = r.db_count_prefix(db, prefix)?;
    r.bm25_live_build(db, prefix, (rows / 20).max(1000))
}

/// How a server searches: its vector index and whether it keeps BM25
/// views across restarts (E369).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SearchOpts {
    pub index: VectorIndex,
    pub keep: bool,
    /// E389: `--hnsw-threads N` (off: 1).
    pub threads: usize,
    /// E394: `--build-off-lock` (off).
    pub off_lock: bool,
}

/// The ref a kept BM25 view of `prefix` in `name` lives on: `p` and the
/// prefix in hex, or for a prefix over 32 bytes `h` and its BLAKE3 hash,
/// so the name stays within a file name's length on disk.
pub fn bm25_ref(name: &str, prefix: &str) -> String {
    if prefix.len() > 32 {
        return format!(
            "search/{name}/h{}",
            blake3::hash(prefix.as_bytes()).to_hex()
        );
    }
    let hex: String = prefix.bytes().map(|b| format!("{b:02x}")).collect();
    format!("search/{name}/p{hex}")
}

fn fail(e: Error) -> Response {
    Response::text(500, e.to_string())
}

fn f32s(b: &[u8]) -> Option<Vec<f32>> {
    b.len().is_multiple_of(4).then(|| vectors::to_f32s(b))
}

impl Vecs {
    fn new(index: VectorIndex, dim: usize, threads: usize) -> Vecs {
        match index {
            VectorIndex::Exact => Vecs::Exact {
                keys: Vec::new(),
                flat: Vec::new(),
                pos: HashMap::new(),
            },
            VectorIndex::Hnsw => Vecs::Hnsw(Box::new(
                HnswIndex::new(dim, HNSW_M, HNSW_EF_BUILD)
                    .with_lanes(true)
                    .with_threads(threads.max(1)),
            )),
        }
    }

    /// The first build's rows, in key order: as `set` one at a time, or
    /// for the graph as `HnswIndex::extend` (batches on its threads).
    fn extend(&mut self, rows: Vec<(String, Vec<f32>)>, dim: usize) {
        match self {
            Vecs::Hnsw(h) => h.extend(rows),
            _ => {
                for (k, v) in rows {
                    self.set(k, &v, dim);
                }
            }
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
    /// E394: whether a BM25 view of `prefix` in `name` is held, in memory
    /// or on its kept ref (then read into memory). The caller holds the
    /// writer's lock as `w`.
    pub fn bm25_held<S: Store>(&self, w: &mut Repo<S>, name: &str, prefix: &str) -> bool {
        let key = (name.to_string(), prefix.to_string());
        let mut kept = self.bm25.lock().unwrap();
        if kept.contains_key(&key) {
            return true;
        }
        if self.keep
            && let Ok(Some(held)) = w.bm25_live_load(&bm25_ref(name, prefix))
        {
            kept.insert(key, held);
            return true;
        }
        false
    }

    /// E394: hold `v`, a view built at `db` off the writer's lock, unless
    /// one is held already; the next `bm25_at` brings it to its head.
    pub fn bm25_put(&self, name: &str, prefix: &str, db: Db, v: Bm25Live) {
        let key = (name.to_string(), prefix.to_string());
        self.bm25.lock().unwrap().entry(key).or_insert((db, v));
    }

    /// The BM25 view held for `prefix` in `name` and the database it is at.
    #[cfg(test)]
    pub fn bm25_held_view(&self, name: &str, prefix: &str) -> Option<(Db, Bm25Live)> {
        let key = (name.to_string(), prefix.to_string());
        self.bm25.lock().unwrap().get(&key).copied()
    }

    /// E394: the lock a build of `prefix` in `name` holds, so a second
    /// search of a prefix being built waits for that build.
    pub fn build_gate(&self, name: &str, prefix: &str) -> Arc<Mutex<()>> {
        let key = (name.to_string(), prefix.to_string());
        self.building
            .lock()
            .unwrap()
            .entry(key)
            .or_default()
            .clone()
    }

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
        let refname = bm25_ref(name, prefix);
        let held = match kept.get(&key) {
            Some(held) => Some(*held),
            // A view kept by an earlier run; one that cannot be read is
            // built again.
            None if self.keep => w.bm25_live_load(&refname).ok().flatten(),
            None => None,
        };
        let v = match held {
            Some((at, v)) if at == *db => {
                kept.insert(key, (at, v));
                return Ok(v);
            }
            Some((at, v)) => w.bm25_live_update(&v, &at, db, prefix)?,
            None => bm25_build(w, db, prefix)?,
        };
        // A view that cannot be kept is still served; the next restart
        // builds it again.
        if self.keep
            && let Err(e) = w.bm25_live_save(&refname, db, &v)
        {
            say!(
                "{} keeping the search view of {name}: {e}",
                crate::live::stamp()
            );
        }
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
                // E389: the rows are read a page at a time, so the build
                // holds one page of values beside the index (the index was
                // the same when every value was read first).
                let end = ikam_kernel::prefix_end(prefix);
                let page = |from: &str| {
                    r.db_range(db, from, end.as_deref(), BUILD_PAGE, false)
                        .map_err(fail)
                };
                let mut rows = page(prefix)?;
                let dim = rows.first().map_or(0, |(_, v)| v.len() / 4);
                let mut vecs = Vecs::new(self.index, dim, self.threads);
                while !rows.is_empty() {
                    let next = rows.last().map(|(k, _)| format!("{k}\0"));
                    let full = rows.len() == BUILD_PAGE;
                    let mut page_vecs = Vec::with_capacity(rows.len());
                    for (k, v) in rows {
                        let v = f32s(&v).filter(|v| v.len() == dim).ok_or_else(|| {
                            Response::text(422, format!("{k:?} is not a vector of {dim}"))
                        })?;
                        page_vecs.push((k, v));
                    }
                    vecs.extend(page_vecs, dim);
                    rows = match (full, next) {
                        (true, Some(n)) => page(&n)?,
                        _ => Vec::new(),
                    };
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
