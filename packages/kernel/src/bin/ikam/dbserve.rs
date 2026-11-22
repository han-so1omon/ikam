//! `ikam db serve`: the database mode (src/db.rs) over HTTP, so another
//! program can keep its records in ikam (E229).
//!
//! Each database NAME is the ref `db/NAME`; every append is committed before
//! it is answered (ack after commit), so an answered write survives a crash.
//! Reads take `at=COMMIT` to read an earlier version, and default to the
//! head.
//!
//! Routes, under `/db/NAME/`:
//! - `POST append`: a batch of events, binary (below); header `If-Head: N`
//!   refuses the batch with 409 unless the log's last sequence number is N.
//!   Answers `{"head":N,"commit":"..."}`.
//! - `GET get?key=K`: the value's bytes, or 404.
//! - `POST get`: keys one per line in the body; answers the frames of the
//!   keys that are set.
//! - `GET scan?prefix=P` or `?start=S&end=E`, with `limit=N` and `reverse=1`:
//!   frames in key order (reversed from the last with `reverse`).
//! - `GET count?prefix=P` (or start and end): `{"count":N}`, reading no values.
//! - `POST span`: prefixes one per line; for each, three frames: the prefix
//!   and its key count in decimal, then its first and its last pair (an
//!   empty key and value when it has none). One request in place of a
//!   count and two scans per prefix (E229: a list of 20 groups made 60).
//!   With `--keep-counts N` (E233) `count?prefix=` and `span` read kept
//!   counts for prefixes ending in `/` of up to N segments.
//! - `GET head`: `{"head":N,"commit":"..."|null}`.
//! - `GET search?q=TEXT&prefix=P`, `POST vsearch?prefix=VP` and their
//!   fusion: ranked and vector search (E358, `dbsearch.rs`).
//!
//! `GET /db/` lists the databases. Add `format=json` to a scan or a get for
//! `[["key","value"],...]` (values must be UTF-8).
//!
//! Frames (`application/x-ikam-kv`): for each pair, key length (u32, big
//! endian), key, value length, value. Events (`application/x-ikam-events`):
//! for each, an op byte (0 upsert, 1 remove, 2 remove with its subtree,
//! 3 remove a key range, E240), then key, idempotency key and the value
//! (upsert) or the range's end (3), each as a u32 length and its bytes.
//!
//! A password: `IKAM_AUTH` for every request, as in `ikam live`;
//! `IKAM_READ_AUTH` (another `ikam passwd` line) may also read.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ikam_kernel::{Db, Error, Event, Id, Repo, Store};

use crate::auth::{Auth, Hosts};
use crate::http::{self, Request, Response};
use crate::json::quote;

pub const EVENTS: &str = "application/x-ikam-events";
pub const FRAMES: &str = "application/x-ikam-kv";

/// Head commit and database of each name opened so far.
type Heads = HashMap<String, (Option<Id>, Db)>;

/// Makes another `Repo` on the writer's store (`Repo::sibling` on a
/// `SharedStore`), for reads that need not wait for the writer (E232,
/// `--concurrent-reads`).
pub type ReaderFactory<S> = Box<dyn Fn(&Repo<S>) -> Repo<S> + Send + Sync>;

/// How to make a reader, and the readers made so far and not in use.
type Readers<S> = (ReaderFactory<S>, Mutex<Vec<Repo<S>>>);

/// What the server holds. Writes go through `writer`, one at a time. Reads
/// use `writer` too unless `readers` makes their own `Repo`s on a shared
/// store (E232), so a page that reads many chunks does not hold up an
/// append and two pages do not wait for each other. Each read works from
/// the roots of the head it saw when it started.
pub struct Server<S: Store> {
    writer: Mutex<Repo<S>>,
    heads: Mutex<Heads>,
    readers: Option<Readers<S>>,
    /// Keep counts of key prefixes up to this many segments (E233); 0: no.
    keep_counts: u32,
    /// E368: fold appends into the kv view once this many events wait
    /// (and before a read of the head); 0: fold every append.
    defer_kv: u64,
    /// Ranked and vector search indexes (E358), built when first searched.
    search: crate::dbsearch::Kept,
}

fn refname(name: &str) -> String {
    format!("db/{name}")
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// The head of `name` and its database, through the cache.
fn open<S: Store>(
    heads: &Mutex<Heads>,
    repo: &mut Repo<S>,
    name: &str,
) -> Result<(Option<Id>, Db), Error> {
    let head = repo.store().get_ref(&refname(name))?;
    if let Some(cached) = heads.lock().unwrap().get(name)
        && cached.0 == head
    {
        return Ok(*cached);
    }
    let db = match head {
        Some(c) => repo.db_at(&c)?,
        None => repo.db_empty()?,
    };
    heads.lock().unwrap().insert(name.to_string(), (head, db));
    Ok((head, db))
}

fn at<S: Store>(
    heads: &Mutex<Heads>,
    repo: &mut Repo<S>,
    name: &str,
    at: Option<&str>,
) -> Result<Db, Response> {
    match at {
        None => open(heads, repo, name).map(|(_, db)| db).map_err(fail),
        Some(hex) => {
            let id: Id = hex
                .parse()
                .map_err(|_| Response::text(400, "at: a full commit id"))?;
            // E368: a commit whose kv view lags is folded for the read.
            repo.db_at(&id)
                .and_then(|db| repo.db_catch_up(&db))
                .map_err(fail)
        }
    }
}

impl<S: Store> Server<S> {
    pub fn new(repo: Repo<S>, readers: Option<ReaderFactory<S>>) -> Server<S> {
        Server {
            writer: Mutex::new(repo),
            heads: Mutex::new(HashMap::new()),
            readers: readers.map(|f| (f, Mutex::new(Vec::new()))),
            keep_counts: 0,
            search: Default::default(),
            defer_kv: 0,
        }
    }

    /// E368: leave appends' kv folding until `events` wait (0: off).
    pub fn defer_kv(mut self, events: u64) -> Server<S> {
        self.defer_kv = events;
        self
    }

    /// E368: before a read of `name`'s head, fold whatever the kv view lags
    /// behind the log and commit it, so a read sees every answered write.
    /// Only the cached head is looked at when nothing lags.
    fn settle(&self, name: &str) -> Result<(), Error> {
        if let Some((_, db)) = self.heads.lock().unwrap().get(name)
            && db.kv_at == db.ids_at
        {
            return Ok(());
        }
        let mut repo = self.writer.lock().unwrap();
        let (_, db) = open(&self.heads, &mut repo, name)?;
        if db.kv_at == repo.db_head(&db.log)? {
            return Ok(());
        }
        let caught = repo.db_catch_up(&db)?;
        let msg = format!("db: fold {} event(s)", caught.kv_at - db.kv_at);
        let c = repo.db_commit(&refname(name), &caught, &msg)?;
        self.heads
            .lock()
            .unwrap()
            .insert(name.to_string(), (Some(c), caught));
        Ok(())
    }

    /// Search vector rows with `index` (E358; exact by default).
    pub fn vector_index(mut self, index: crate::dbsearch::VectorIndex) -> Server<S> {
        self.search.index = index;
        self
    }

    /// Keep counts of key prefixes of up to `depth` segments in every
    /// database this server appends to (E233): built on a database's next
    /// append when it has none, and edited by each append after.
    pub fn keep_counts(mut self, depth: u32) -> Server<S> {
        self.keep_counts = depth;
        self
    }

    /// Run a read with a `Repo`: a reader of its own when there are
    /// readers, else the writer.
    fn read<T>(&self, f: impl FnOnce(&mut Repo<S>) -> T) -> T {
        match &self.readers {
            None => f(&mut self.writer.lock().unwrap()),
            Some((make, pool)) => {
                let pooled = pool.lock().unwrap().pop();
                let mut repo = pooled.unwrap_or_else(|| make(&self.writer.lock().unwrap()));
                let out = f(&mut repo);
                pool.lock().unwrap().push(repo);
                out
            }
        }
    }
}

fn fail(e: Error) -> Response {
    match e {
        Error::IdempotencyConflict(k) => Response::text(
            409,
            format!("idempotency key {k} was used for another event"),
        ),
        Error::InvalidName(k) => Response::text(400, format!("invalid key {k:?}")),
        e => Response::text(500, e.to_string()),
    }
}

/// Decode a body of events.
pub fn decode_events(mut b: &[u8]) -> Result<Vec<Event>, String> {
    fn take<'a>(b: &mut &'a [u8]) -> Result<&'a [u8], String> {
        if b.len() < 4 {
            return Err("truncated length".into());
        }
        let n = u32::from_be_bytes(b[..4].try_into().unwrap()) as usize;
        if b.len() < 4 + n {
            return Err("truncated field".into());
        }
        let out = &b[4..4 + n];
        *b = &b[4 + n..];
        Ok(out)
    }
    let text = |v: &[u8]| String::from_utf8(v.to_vec()).map_err(|_| "keys are UTF-8".to_string());
    let mut out = Vec::new();
    while let Some((&op, rest)) = b.split_first() {
        b = rest;
        let key = text(take(&mut b)?)?;
        let idem = text(take(&mut b)?)?;
        out.push(match op {
            0 => Event::upsert(&key, take(&mut b)?, &idem),
            1 | 2 => Event::remove(&key, op == 2, &idem),
            3 => Event::remove_range(&key, &text(take(&mut b)?)?, &idem),
            _ => return Err(format!("unknown op {op}")),
        });
    }
    Ok(out)
}

/// Encode events as `decode_events` reads them (the tests and clients).
#[cfg_attr(not(test), allow(dead_code))]
pub fn encode_events(events: &[Event]) -> Vec<u8> {
    let mut out = Vec::new();
    let put = |out: &mut Vec<u8>, b: &[u8]| {
        out.extend_from_slice(&(b.len() as u32).to_be_bytes());
        out.extend_from_slice(b);
    };
    for e in events {
        match &e.op {
            ikam_kernel::Op::Upsert(v) => {
                out.push(0);
                put(&mut out, e.key.as_bytes());
                put(&mut out, e.idempotency_key.as_bytes());
                put(&mut out, v);
            }
            ikam_kernel::Op::Remove { subtree } => {
                out.push(if *subtree { 2 } else { 1 });
                put(&mut out, e.key.as_bytes());
                put(&mut out, e.idempotency_key.as_bytes());
            }
            ikam_kernel::Op::RemoveRange { end } => {
                out.push(3);
                put(&mut out, e.key.as_bytes());
                put(&mut out, e.idempotency_key.as_bytes());
                put(&mut out, end.as_bytes());
            }
        }
    }
    out
}

fn frames(pairs: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, v) in pairs {
        out.extend_from_slice(&(k.len() as u32).to_be_bytes());
        out.extend_from_slice(k.as_bytes());
        out.extend_from_slice(&(v.len() as u32).to_be_bytes());
        out.extend_from_slice(v);
    }
    out
}

fn pairs_response(req: &Request, pairs: Vec<(String, Vec<u8>)>) -> Response {
    if req.param("format").as_deref() == Some("json") {
        let mut items = Vec::with_capacity(pairs.len());
        for (k, v) in pairs {
            let Ok(v) = String::from_utf8(v) else {
                return Response::text(415, format!("value of {k:?} is not UTF-8"));
            };
            items.push(format!("[{},{}]", quote(&k), quote(&v)));
        }
        Response::new(200).body("application/json", format!("[{}]", items.join(",")))
    } else {
        Response::new(200).body(FRAMES, frames(&pairs))
    }
}

fn head_json(head: Option<Id>, seq: u64) -> String {
    format!(
        "{{\"head\":{seq},\"commit\":{}}}",
        head.map_or("null".to_string(), |c| quote(&c.to_string()))
    )
}

fn handle<S: Store>(srv: &Server<S>, req: &Request) -> Response {
    let path = http::decode(&req.path, false);
    let Some(rest) = path.strip_prefix("/db/") else {
        return Response::text(404, "not found");
    };
    if rest.is_empty() {
        let names: Vec<String> = match srv.read(|r| r.store().refs()) {
            Ok(refs) => refs
                .into_iter()
                .filter_map(|(r, _)| r.strip_prefix("db/").map(quote))
                .collect(),
            Err(e) => return fail(e),
        };
        return Response::new(200).body("application/json", format!("[{}]", names.join(",")));
    }
    let Some((name, route)) = rest.split_once('/') else {
        return Response::text(404, "not found");
    };
    if !valid_name(name) {
        return Response::text(400, "database names are [A-Za-z0-9._-]");
    }
    match (req.method.as_str(), route) {
        ("POST", "append") => return append(srv, name, req),
        // E368: a search answers at the head, so it folds what waits first.
        ("GET" | "POST", "search") | ("POST", "vsearch") => {
            return match srv.settle(name) {
                Ok(()) => search(srv, name, req),
                Err(e) => fail(e),
            };
        }
        _ => {}
    }
    let heads = &srv.heads;
    let at_param = req.param("at");
    if at_param.is_none()
        && route != "head"
        && let Err(e) = srv.settle(name)
    {
        return fail(e);
    }
    srv.read(|repo| {
        let at = at_param.as_deref();
        match (req.method.as_str(), route) {
            ("GET", "head") => match open(heads, repo, name) {
                Ok((head, db)) => match repo.db_head(&db.log) {
                    Ok(seq) => Response::new(200).body("application/json", head_json(head, seq)),
                    Err(e) => fail(e),
                },
                Err(e) => fail(e),
            },
            ("GET", "get") => {
                let Some(key) = req.param("key") else {
                    return Response::text(400, "get?key=K");
                };
                let db = match self::at(heads, repo, name, at) {
                    Ok(db) => db,
                    Err(r) => return r,
                };
                match repo.db_get(&db, &key) {
                    Ok(Some(v)) if req.param("format").as_deref() == Some("json") => {
                        pairs_response(req, vec![(key, v)])
                    }
                    Ok(Some(v)) => Response::new(200).body("application/octet-stream", v),
                    Ok(None) => Response::text(404, "no such key"),
                    Err(e) => fail(e),
                }
            }
            ("POST", "get") => {
                let Ok(body) = std::str::from_utf8(&req.body) else {
                    return Response::text(400, "keys are UTF-8");
                };
                let db = match self::at(heads, repo, name, at) {
                    Ok(db) => db,
                    Err(r) => return r,
                };
                let mut out = Vec::new();
                for key in body.lines().filter(|l| !l.is_empty()) {
                    match repo.db_get(&db, key) {
                        Ok(Some(v)) => out.push((key.to_string(), v)),
                        Ok(None) => {}
                        Err(e) => return fail(e),
                    }
                }
                pairs_response(req, out)
            }
            ("GET", "scan") => {
                let limit = match req.param("limit") {
                    None => usize::MAX,
                    Some(n) => match n.parse() {
                        Ok(n) => n,
                        Err(_) => return Response::text(400, "limit=N"),
                    },
                };
                let reverse = matches!(req.param("reverse").as_deref(), Some("1" | "true"));
                let db = match self::at(heads, repo, name, at) {
                    Ok(db) => db,
                    Err(r) => return r,
                };
                let got = match (req.param("prefix"), req.param("start")) {
                    (Some(p), None) => repo.db_prefix(&db, &p, limit, reverse),
                    (None, start) => {
                        let start = start.unwrap_or_default();
                        let end = req.param("end");
                        repo.db_range(&db, &start, end.as_deref(), limit, reverse)
                    }
                    (Some(_), Some(_)) => return Response::text(400, "prefix or start, not both"),
                };
                match got {
                    Ok(pairs) => pairs_response(req, pairs),
                    Err(e) => fail(e),
                }
            }
            ("GET", "count") => {
                let db = match self::at(heads, repo, name, at) {
                    Ok(db) => db,
                    Err(r) => return r,
                };
                let (start, end) = match (req.param("prefix"), req.param("start")) {
                    (Some(p), None) => (p.clone(), ikam_kernel::prefix_end(&p)),
                    (None, start) => (start.unwrap_or_default(), req.param("end")),
                    (Some(_), Some(_)) => return Response::text(400, "prefix or start, not both"),
                };
                let n = match req.param("prefix") {
                    Some(p) => repo.db_count_prefix(&db, &p),
                    None => repo.db_count(&db, &start, end.as_deref()),
                };
                match n {
                    Ok(n) => {
                        Response::new(200).body("application/json", format!("{{\"count\":{n}}}"))
                    }
                    Err(e) => fail(e),
                }
            }
            ("POST", "span") => {
                let Ok(body) = std::str::from_utf8(&req.body) else {
                    return Response::text(400, "prefixes are UTF-8");
                };
                let db = match self::at(heads, repo, name, at) {
                    Ok(db) => db,
                    Err(r) => return r,
                };
                let prefixes: Vec<&str> = body.lines().filter(|l| !l.is_empty()).collect();
                let counts = match repo.db_count_prefixes(&db, &prefixes) {
                    Ok(n) => n,
                    Err(e) => return fail(e),
                };
                let mut out = Vec::new();
                for (p, n) in prefixes.into_iter().zip(counts) {
                    out.push((p.to_string(), n.to_string().into_bytes()));
                    for reverse in [false, true] {
                        match repo.db_prefix(&db, p, 1, reverse) {
                            Ok(mut got) => out.push(got.pop().unwrap_or_default()),
                            Err(e) => return fail(e),
                        }
                    }
                }
                Response::new(200).body(FRAMES, frames(&out))
            }
            _ => Response::text(404, "not found"),
        }
    })
}

/// `search` and `vsearch` (E358, `dbsearch.rs`): the keyword view is
/// brought to the head under the writer's lock (it writes postings), then
/// the query runs on a reader.
fn search<S: Store>(srv: &Server<S>, name: &str, req: &Request) -> Response {
    let q = match crate::dbsearch::Query::parse(req, req.path.ends_with("/vsearch")) {
        Ok(q) => q,
        Err(r) => return r,
    };
    let (db, bm25) = {
        let mut w = srv.writer.lock().unwrap();
        let db = match open(&srv.heads, &mut w, name) {
            Ok((_, db)) => db,
            Err(e) => return fail(e),
        };
        match q.keyword_prefix() {
            None => (db, None),
            Some(p) => match srv.search.bm25_at(&mut w, name, p, &db) {
                Ok(v) => (db, Some(v)),
                Err(e) => return fail(e),
            },
        }
    };
    srv.read(|r| srv.search.answer(r, name, &db, bm25, &q, req))
}

/// `POST append`, under the writer's lock.
fn append<S: Store>(srv: &Server<S>, name: &str, req: &Request) -> Response {
    if req
        .header("content-type")
        .is_some_and(|t| !t.starts_with(EVENTS))
    {
        return Response::text(415, format!("events are {EVENTS}"));
    }
    let events = match decode_events(&req.body) {
        Ok(e) => e,
        Err(e) => return Response::text(400, e),
    };
    let mut repo = srv.writer.lock().unwrap();
    let (head, opened) = match open(&srv.heads, &mut repo, name) {
        Ok(x) => x,
        Err(e) => return fail(e),
    };
    let db = match srv.keep_counts {
        0 => opened,
        // E368: counts are built from a current view, so a view left
        // behind by deferred appends folds first.
        d if !opened.counts.is_some_and(|c| c.depth == d) => {
            match repo
                .db_catch_up(&opened)
                .and_then(|o| repo.db_keep_counts(&o, d))
            {
                Ok(db) => db,
                Err(e) => return fail(e),
            }
        }
        d => match repo.db_keep_counts(&opened, d) {
            Ok(db) => db,
            Err(e) => return fail(e),
        },
    };
    let seq = match repo.db_head(&db.log) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    if let Some(want) = req.header("if-head")
        && want.trim().parse::<u64>().ok() != Some(seq)
    {
        return Response::new(409).body("application/json", head_json(head, seq));
    }
    // E368: with `--defer-kv` the kv view folds once enough events wait;
    // without it, a view left behind (a store written with it) folds first.
    let new = if srv.defer_kv > 0 {
        repo.db_append_deferred(&db, &events).and_then(|d| {
            if d.ids_at - d.kv_at >= srv.defer_kv {
                repo.db_catch_up(&d)
            } else {
                Ok(d)
            }
        })
    } else {
        repo.db_catch_up(&db)
            .and_then(|d| repo.db_append(&d, &events))
    };
    let new = match new {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    if new == db {
        // Every event was a retry and no view was folded: nothing to
        // commit. (A batch of retries over a commit left behind by
        // `--defer-kv` still commits the folded view.)
        return Response::new(200).body("application/json", head_json(head, seq));
    }
    let msg = format!("db: {} event(s)", new.ids_at - seq);
    match repo.db_commit(&refname(name), &new, &msg) {
        Ok(c) => {
            srv.heads
                .lock()
                .unwrap()
                .insert(name.to_string(), (Some(c), new));
            Response::new(200).body("application/json", head_json(Some(c), new.ids_at))
        }
        Err(e) => fail(e),
    }
}

/// Serve `repo`'s databases on 127.0.0.1:PORT until stopped; with
/// `readers`, reads use their own `Repo`s (E232).
pub fn serve<S: Store + Send + 'static>(
    repo: Repo<S>,
    readers: Option<ReaderFactory<S>>,
    (keep_counts, defer_kv): (u32, u64),
    index: crate::dbsearch::VectorIndex,
    port: u16,
    allowed: Vec<String>,
    log_requests: bool,
) -> Result<(), String> {
    let auth = Auth::from_env()?;
    let reader = match std::env::var("IKAM_READ_AUTH") {
        Ok(line) => Some(Auth::parse(&line)?),
        Err(_) => None,
    };
    if reader.is_some() && auth.is_none() {
        return Err("IKAM_READ_AUTH needs IKAM_AUTH too".into());
    }
    let state = Arc::new(
        Server::new(repo, readers)
            .keep_counts(keep_counts)
            .defer_kv(defer_kv)
            .vector_index(index),
    );
    let hosts = Hosts::new(port, allowed);
    let password = auth.is_some();
    let handler = Arc::new(move |req: Request| {
        let started = std::time::Instant::now();
        let resp = if !req.header("host").is_some_and(|h| hosts.ok(h)) {
            Response::text(403, "unknown Host")
        } else {
            let refused = auth.as_ref().and_then(|a| a.check(&req));
            let read_ok =
                req.method == "GET" && reader.as_ref().is_some_and(|r| r.check(&req).is_none());
            match refused {
                Some(r) if !read_ok => r,
                _ => handle(&state, &req),
            }
        };
        if log_requests {
            eprintln!(
                "{} {} {} -> {} {:.0}ms",
                crate::live::stamp(),
                req.method,
                req.path,
                resp.status,
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        resp
    });
    let t = crate::live::stamp();
    eprintln!("{t} databases on http://127.0.0.1:{port}/db/");
    if password {
        eprintln!("{t} password required (IKAM_AUTH)");
    }
    http::serve(port, handler, http::Limits::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikam_kernel::MemStore;

    fn req(method: &str, target: &str, body: &[u8], headers: &[(&str, &str)]) -> Request {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        Request {
            method: method.into(),
            path: path.into(),
            query: query.into(),
            headers: headers
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            body: body.to_vec(),
        }
    }

    fn unframe(mut b: &[u8]) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let take = |b: &mut &[u8]| {
            let n = u32::from_be_bytes(b[..4].try_into().unwrap()) as usize;
            let s = String::from_utf8(b[4..4 + n].to_vec()).unwrap();
            *b = &b[4 + n..];
            s
        };
        while !b.is_empty() {
            let k = take(&mut b);
            let v = take(&mut b);
            out.push((k, v));
        }
        out
    }

    #[test]
    fn events_round_trip() {
        let events = vec![
            Event::upsert("t/a/1", b"{\"x\":1}", "i1"),
            Event::upsert("t/a/2", b"", "i2"),
            Event::remove("t/a", true, "i3"),
            Event::remove("t/b", false, "i4"),
            Event::remove_range("t/c/1", "t/c/5", "i5"),
        ];
        assert_eq!(decode_events(&encode_events(&events)).unwrap(), events);
        assert!(decode_events(&[0, 0, 0, 0, 9]).is_err());
        assert!(decode_events(&[7, 0, 0, 0, 0, 0, 0, 0, 0]).is_err());
    }

    fn vec_body(v: &[f32]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_le_bytes()).collect()
    }

    fn keys_of(r: &Response) -> Vec<String> {
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let j = crate::json::parse(std::str::from_utf8(&r.body).unwrap()).unwrap();
        j.as_arr()
            .iter()
            .map(|h| h.get("key").unwrap().as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn search_ranks_follows_appends_and_fuses() {
        use crate::dbsearch::VectorIndex;
        for shared in [false, true] {
            for index in [VectorIndex::Exact, VectorIndex::Hnsw] {
                // E368: with folds left for reads, a search folds first.
                for defer in [0, 100] {
                    searched(shared, index, defer);
                }
            }
        }
    }

    fn searched(shared: bool, index: crate::dbsearch::VectorIndex, defer: u64) {
        let store = ikam_kernel::SharedStore::new(MemStore::default());
        let make = || -> Option<ReaderFactory<_>> {
            shared.then(|| Box::new(|w: &Repo<_>| w.sibling()) as ReaderFactory<_>)
        };
        let st = Server::new(Repo::new(store.clone()), make())
            .vector_index(index)
            .defer_kv(defer);
        let h = |st: &Server<_>, r: Request| handle(st, &r);
        let ct = [("content-type", EVENTS)];
        let x = |a: f32, b: f32| vec_body(&[a, b, 0.0]);
        let body = encode_events(&[
            Event::upsert("t/a", b"a parser for json files", "1"),
            Event::upsert("t/b", b"resize images quickly", "2"),
            Event::upsert("t/c", b"json json schema checks", "3"),
            Event::upsert("u/z", b"json outside the prefix", "4"),
            Event::upsert("v/a", &x(1.0, 0.0), "5"),
            Event::upsert("v/b", &x(0.0, 1.0), "6"),
            Event::upsert("v/c", &x(0.6, 0.8), "7"),
        ]);
        assert_eq!(h(&st, req("POST", "/db/s/append", &body, &ct)).status, 200);
        let get = |st: &Server<_>, q: &str| keys_of(&h(st, req("GET", q, b"", &[])));
        assert_eq!(get(&st, "/db/s/search?q=json&prefix=t/"), ["t/c", "t/a"]);
        assert_eq!(get(&st, "/db/s/search?q=json&prefix=t/&k=1"), ["t/c"]);
        assert!(get(&st, "/db/nobody/search?q=json&prefix=t/").is_empty());
        // The index follows appends: a changed row and a removed one.
        let more = encode_events(&[
            Event::upsert("t/b", b"json images", "8"),
            Event::remove("t/c", false, "9"),
            Event::upsert("v/b", &x(0.8, 0.6), "10"),
        ]);
        assert_eq!(h(&st, req("POST", "/db/s/append", &more, &ct)).status, 200);
        let after = get(&st, "/db/s/search?q=json&prefix=t/");
        assert_eq!(after, ["t/b", "t/a"]);
        // The same answer as a server that builds the index at the head.
        let fresh = Server::new(Repo::new(store.clone()), make()).vector_index(index);
        assert_eq!(get(&fresh, "/db/s/search?q=json&prefix=t/"), after);
        // Vectors: nearest by dot product, as f32 or JSON.
        let v = |st: &Server<_>, q: &str, body: Vec<u8>, json: bool| {
            let hdr = [("content-type", "application/json")];
            keys_of(&h(st, req("POST", q, &body, if json { &hdr } else { &[] })))
        };
        assert_eq!(
            v(&st, "/db/s/vsearch?prefix=v/&k=2", x(1.0, 0.0), false),
            ["v/a", "v/b"]
        );
        assert_eq!(
            v(&st, "/db/s/vsearch?prefix=v/", b"[0,1,0]".to_vec(), true),
            ["v/c", "v/b", "v/a"]
        );
        assert_eq!(
            v(&st, "/db/s/vsearch?prefix=v/", b"[0,1,0]".to_vec(), true)[0],
            "v/c"
        );
        // Fused: keyword rows t/a and t/b; the vector picks between them.
        let fuse = "/db/s/search?q=json&prefix=t/&vectors=v/&alpha=0.3";
        assert_eq!(v(&st, fuse, x(1.0, 0.0), false)[0], "t/a");
        assert_eq!(v(&st, fuse, x(0.0, 1.0), false)[0], "t/b");
        let rrf = "/db/s/search?q=json&prefix=t/&vectors=v/&fuse=rrf";
        assert_eq!(
            v(&st, rrf, x(0.0, 1.0), false).len(),
            3,
            "t/c's vector is kept"
        );
        // Refusals.
        let status = |q: &str, body: &[u8]| h(&st, req("POST", q, body, &[])).status;
        assert_eq!(
            status("/db/s/vsearch?prefix=v/", &vec_body(&[1.0, 0.0])),
            400
        );
        assert_eq!(status("/db/s/vsearch?prefix=v/", b"abc"), 400);
        assert_eq!(status("/db/s/vsearch", &x(1.0, 0.0)), 400);
        assert_eq!(status("/db/s/search?prefix=t/", b""), 400);
        assert_eq!(status(&format!("{fuse}&fuse=max"), &x(1.0, 0.0)), 400);
        assert_eq!(status("/db/s/vsearch?prefix=t/", &x(1.0, 0.0)), 422);
        // A prefix searched while empty takes its dimension from the first
        // vectors appended later.
        assert!(v(&st, "/db/s/vsearch?prefix=w/", x(1.0, 0.0), false).is_empty());
        let late = encode_events(&[
            Event::upsert("w/a", &x(1.0, 0.0), "11"),
            Event::upsert("w/b", &x(0.0, 1.0), "12"),
        ]);
        assert_eq!(h(&st, req("POST", "/db/s/append", &late, &ct)).status, 200);
        assert_eq!(
            v(&st, "/db/s/vsearch?prefix=w/", x(0.0, 1.0), false),
            ["w/b", "w/a"]
        );
    }

    /// A server without `--defer-kv` over a commit whose kv view lags folds
    /// it and commits the fold, even for an empty batch or one of retries.
    #[test]
    fn a_retry_over_a_lagging_commit_commits_the_fold() {
        for batch in [vec![], vec![Event::upsert("t/a", b"1", "a")]] {
            let mut repo = Repo::new(MemStore::default());
            let empty = repo.db_empty().unwrap();
            let late = repo
                .db_append_deferred(&empty, &[Event::upsert("t/a", b"1", "a")])
                .unwrap();
            repo.db_commit(&refname("m"), &late, "deferred").unwrap();
            let st = Server::new(repo, None);
            let ct = [("content-type", EVENTS)];
            let r = handle(
                &st,
                &req("POST", "/db/m/append", &encode_events(&batch), &ct),
            );
            assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
            let mut w = st.writer.lock().unwrap();
            let c = w.store().get_ref(&refname("m")).unwrap().unwrap();
            let db = w.db_at(&c).unwrap();
            assert_eq!((db.kv_at, db.ids_at), (1, 1), "{} events", batch.len());
            assert!(w.db_append(&db, &[Event::upsert("t/b", b"2", "b")]).is_ok());
        }
    }

    #[test]
    fn append_scan_get_and_read_at_an_older_commit() {
        for shared in [false, true] {
            for keep_counts in [0, 3] {
                // E368: with the kv view folded every 2 events, or only
                // when read, every answer is the same.
                for defer in [0, 2, 1000] {
                    served_routes(shared, keep_counts, defer);
                }
            }
        }
    }

    fn served_routes(shared: bool, keep_counts: u32, defer: u64) {
        let st = if shared {
            let store = ikam_kernel::SharedStore::new(MemStore::default());
            let make: ReaderFactory<_> = Box::new(|w: &Repo<_>| w.sibling());
            Server::new(Repo::new(store), Some(make))
        } else {
            Server::new(
                Repo::new(ikam_kernel::SharedStore::new(MemStore::default())),
                None,
            )
        }
        .keep_counts(keep_counts)
        .defer_kv(defer);
        let h = |r: Request| handle(&st, &r);
        let ev = [(EVENTS.to_string())];
        let ct = [("content-type", ev[0].as_str())];
        let body = encode_events(&[
            Event::upsert("t/items/k1/2024-01", b"1.5", "a"),
            Event::upsert("t/items/k1/2024-02", b"2.5", "b"),
            Event::upsert("t/items/k2/2024-01", b"9", "c"),
        ]);
        let r = h(req("POST", "/db/m/append", &body, &ct));
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let first = String::from_utf8(r.body).unwrap();
        assert!(first.starts_with("{\"head\":3,\"commit\":\""), "{first}");
        let commit = first.split('"').nth(5).unwrap().to_string();
        // A retry changes nothing; a stale If-Head is refused.
        let r = h(req("POST", "/db/m/append", &body, &ct));
        assert_eq!(String::from_utf8(r.body).unwrap(), first);
        let more = encode_events(&[Event::upsert("t/items/k1/2024-01", b"1.75", "d")]);
        let mut stale = ct.to_vec();
        stale.push(("if-head", "2"));
        assert_eq!(h(req("POST", "/db/m/append", &more, &stale)).status, 409);
        let mut fresh = ct.to_vec();
        fresh.push(("if-head", "3"));
        assert_eq!(h(req("POST", "/db/m/append", &more, &fresh)).status, 200);

        let r = h(req("GET", "/db/m/scan?prefix=t/items/k1/", b"", &[]));
        assert_eq!(
            unframe(&r.body),
            vec![
                ("t/items/k1/2024-01".into(), "1.75".into()),
                ("t/items/k1/2024-02".into(), "2.5".into())
            ]
        );
        let r = h(req(
            "GET",
            &format!("/db/m/scan?prefix=t/items/k1/&at={commit}"),
            b"",
            &[],
        ));
        assert_eq!(unframe(&r.body)[0].1, "1.5", "the older commit's value");
        let r = h(req(
            "GET",
            "/db/m/scan?start=t/items/k1/2024-02&limit=1&format=json",
            b"",
            &[],
        ));
        assert_eq!(
            String::from_utf8(r.body).unwrap(),
            r#"[["t/items/k1/2024-02","2.5"]]"#
        );
        let r = h(req(
            "GET",
            "/db/m/scan?prefix=t/items/&reverse=1&limit=1",
            b"",
            &[],
        ));
        assert_eq!(unframe(&r.body)[0].0, "t/items/k2/2024-01");
        let r = h(req("GET", "/db/m/get?key=t/items/k2/2024-01", b"", &[]));
        assert_eq!(r.body, b"9");
        assert_eq!(h(req("GET", "/db/m/get?key=nope", b"", &[])).status, 404);
        let r = h(req(
            "POST",
            "/db/m/get",
            b"t/items/k2/2024-01\nnope\nt/items/k1/2024-02\n",
            &[],
        ));
        assert_eq!(unframe(&r.body).len(), 2);
        let r = h(req("GET", "/db/m/count?prefix=t/items/k1/", b"", &[]));
        assert_eq!(String::from_utf8(r.body).unwrap(), r#"{"count":2}"#);
        let r = h(req("POST", "/db/m/span", b"t/items/k1/\nt/none/\n", &[]));
        let got = unframe(&r.body);
        let keys: Vec<&str> = got.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            [
                "t/items/k1/",
                "t/items/k1/2024-01",
                "t/items/k1/2024-02",
                "t/none/",
                "",
                ""
            ]
        );
        assert_eq!(got[0].1, "2");
        assert_eq!(got[3].1, "0");
        let r = h(req("GET", "/db/m/head", b"", &[]));
        assert!(
            String::from_utf8(r.body)
                .unwrap()
                .starts_with("{\"head\":4,")
        );
        let r = h(req("GET", "/db/", b"", &[]));
        assert_eq!(String::from_utf8(r.body).unwrap(), r#"["m"]"#);
        assert_eq!(h(req("GET", "/db/../x/head", b"", &[])).status, 400);
        let r = h(req(
            "POST",
            "/db/m/append",
            &body,
            &[("content-type", "text/plain")],
        ));
        assert_eq!(r.status, 415);
    }
}
