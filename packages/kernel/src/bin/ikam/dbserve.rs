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
//! - `GET views` and `GET view?name=V`: kept groups and indexes read from
//!   `--views FILE` (E361, `dbviews.rs`).
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

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use ikam_kernel::{Db, Error, Event, Id, Op, Repo, Store};

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
    /// Kept views (E361), built on a database's first view request.
    views: Option<crate::dbviews::Served>,
    /// Other processes write this store too (E370): appends commit only
    /// over the head they read, and read again on a conflict.
    shared: bool,
    /// E376: drop upserts that would write a key's current value again.
    skip_unchanged: bool,
    /// E376: per database, the keys written since its kv view's position,
    /// known while this process made every append since (see `Touched`).
    touched: Mutex<HashMap<String, Touched>>,
}

/// E376: what a database's deferred appends wrote after its kv view's
/// position `kv_at`, up to log position `head`: each key's latest value
/// (by its id; `None` once removed). The kv view does not show these yet.
/// Only appends this process made are known: a database whose view lags
/// for another reason (a restart, a store written elsewhere) has no entry,
/// and nothing is skipped until its view next folds.
struct Touched {
    kv_at: u64,
    head: u64,
    keys: HashMap<String, Option<Id>>,
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
            views: None,
            shared: false,
            skip_unchanged: false,
            touched: Mutex::new(HashMap::new()),
        }
    }

    /// E376 (off by default): an upsert whose key holds those very bytes
    /// already is dropped from its batch instead of logged, so importing
    /// unchanged rows again writes nothing. Its idempotency key is then
    /// not recorded, which a client that reuses idempotency keys for other
    /// values could notice. Off with a shared store (other processes'
    /// pending writes are not known).
    pub fn skip_unchanged(mut self, skip: bool) -> Server<S> {
        self.skip_unchanged = skip;
        self
    }

    /// E376: `events` without the upserts that would write a key's current
    /// value again, and how many were dropped. A key's current value is
    /// the one a pending deferred append wrote, else the kv view's. A key
    /// already written earlier in the batch is kept (that write may be a
    /// retry the log skips), and a subtree or range removal leaves it and
    /// every event after it as they are.
    fn drop_unchanged(
        &self,
        repo: &Repo<S>,
        name: &str,
        db: &Db,
        seq: u64,
        events: &[Event],
    ) -> Result<(Vec<Event>, usize), Error> {
        let touched = self.touched.lock().unwrap();
        let none = HashMap::new();
        let pending = if db.kv_at == seq {
            &none
        } else {
            match touched.get(name) {
                Some(t) if t.kv_at == db.kv_at && t.head == seq => &t.keys,
                _ => return Ok((events.to_vec(), 0)),
            }
        };
        let simple = |e: &Event| matches!(e.op, Op::Upsert(_) | Op::Remove { subtree: false });
        let lookup: BTreeSet<String> = events
            .iter()
            .take_while(|e| simple(e))
            .filter(|e| matches!(e.op, Op::Upsert(_)) && !pending.contains_key(&e.key))
            .map(|e| e.key.clone())
            .collect();
        let view: HashMap<String, Id> = repo
            .db_get_many(db, &lookup)?
            .into_iter()
            .map(|(k, v)| (k, Id::of(&v)))
            .collect();
        let mut seen = std::collections::HashSet::new();
        let mut kept = Vec::with_capacity(events.len());
        for (i, e) in events.iter().enumerate() {
            if !simple(e) {
                kept.extend_from_slice(&events[i..]);
                break;
            }
            let value = match &e.op {
                Op::Upsert(v) => Some(Id::of(v)),
                _ => None,
            };
            let current = match pending.get(&e.key) {
                Some(v) => *v,
                None => view.get(&e.key).copied(),
            };
            if seen.insert(e.key.as_str()) && value.is_some() && value == current {
                continue;
            }
            kept.push(e.clone());
        }
        let dropped = events.len() - kept.len();
        Ok((kept, dropped))
    }

    /// E376: record what an append from `db` (log at `seq`) to `new` wrote.
    fn note_touched(&self, name: &str, db: &Db, seq: u64, new: &Db, events: &[Event]) {
        let mut touched = self.touched.lock().unwrap();
        if new.kv_at == new.ids_at {
            touched.insert(
                name.to_string(),
                Touched {
                    kv_at: new.kv_at,
                    head: new.ids_at,
                    keys: HashMap::new(),
                },
            );
            return;
        }
        let known = db.kv_at == seq
            || touched
                .get(name)
                .is_some_and(|t| t.kv_at == db.kv_at && t.head == seq);
        let simple = events
            .iter()
            .all(|e| matches!(e.op, Op::Upsert(_) | Op::Remove { subtree: false }));
        // Every event logged (none a retry), so each is the key's latest.
        let all_logged = new.ids_at - seq == events.len() as u64;
        if !known || !simple || !all_logged || new.kv_at != db.kv_at {
            touched.remove(name);
            return;
        }
        // A fold since the last append (a read's `settle`) starts a new
        // interval: what the entry held is folded already.
        if db.kv_at == seq {
            touched.remove(name);
        }
        let t = touched.entry(name.to_string()).or_insert(Touched {
            kv_at: db.kv_at,
            head: seq,
            keys: HashMap::new(),
        });
        for e in events {
            let value = match &e.op {
                Op::Upsert(v) => Some(Id::of(v)),
                _ => None,
            };
            t.keys.insert(e.key.clone(), value);
        }
        t.head = new.ids_at;
    }

    /// E368: leave appends' kv folding until `events` wait (0: off).
    pub fn defer_kv(mut self, events: u64) -> Server<S> {
        self.defer_kv = events;
        self
    }

    /// E368: before a read of `name`'s head, fold whatever the kv view lags
    /// behind the log and commit it, so a read sees every answered write.
    /// Only the cached head is looked at when nothing lags, unless other
    /// processes write the store (E370), whose head may have moved.
    fn settle(&self, name: &str) -> Result<(), Error> {
        if !self.shared
            && let Some((_, db)) = self.heads.lock().unwrap().get(name)
            && db.kv_at == db.ids_at
        {
            return Ok(());
        }
        let mut repo = self.writer.lock().unwrap();
        let mut tries = 0;
        loop {
            let (head, db) = open(&self.heads, &mut repo, name)?;
            if db.kv_at == repo.db_head(&db.log)? {
                return Ok(());
            }
            let caught = repo.db_catch_up(&db)?;
            let msg = format!("db: fold {} event(s)", caught.kv_at - db.kv_at);
            let c = match head {
                Some(base) if self.shared => repo.db_commit_at(&refname(name), base, &caught, &msg),
                _ => repo.db_commit(&refname(name), &caught, &msg),
            };
            match c {
                // Another process moved the head: fold over its head.
                Err(Error::RefConflict(_)) if self.shared && tries < 16 => tries += 1,
                c => {
                    self.heads
                        .lock()
                        .unwrap()
                        .insert(name.to_string(), (Some(c?), caught));
                    return Ok(());
                }
            }
        }
    }

    /// Keep `views` for every database (E361).
    pub fn views(mut self, views: Option<crate::dbviews::Served>) -> Server<S> {
        self.views = views;
        self
    }

    /// Other processes write this store too (E370; off by default). Each
    /// handle sees what they store (`Repo::refresh_ledger`, E298, which
    /// the caller sets on the repo), and an append commits only over the
    /// head it read (`db_commit_at`), reading the new head and applying its
    /// events again on a conflict, so no process's events are lost.
    pub fn shared_store(mut self, shared: bool) -> Server<S> {
        self.shared = shared;
        self
    }

    /// Search vector rows with `index` (E358; exact by default).
    pub fn vector_index(mut self, index: crate::dbsearch::VectorIndex) -> Server<S> {
        self.search.index = index;
        self
    }

    /// Link a graph's first build on `n` threads (E389; off by default: 1).
    /// The graph differs a little from one linked a row at a time (E332).
    pub fn vector_threads(mut self, n: usize) -> Server<S> {
        self.search.threads = n;
        self
    }

    /// Build a prefix's first BM25 view off the writer's lock (E394; off
    /// by default; needs readers, `--concurrent-reads`).
    pub fn build_off_lock(mut self, on: bool) -> Server<S> {
        self.search.off_lock = on;
        self
    }

    /// Keep BM25 views on refs across restarts (E369; off by default).
    pub fn keep_search(mut self, keep: bool) -> Server<S> {
        self.search.keep = keep;
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
        // E368: a view answers at the head, so it folds what waits first.
        ("GET", "views" | "view") => {
            return match srv.settle(name) {
                Ok(()) => views(srv, name, route, req),
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
    if let Some(p) = q.keyword_prefix()
        && srv.search.off_lock
        && srv.readers.is_some()
        && let Err(e) = build_off_lock(srv, name, p)
    {
        return fail(e);
    }
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

/// E394 (`--build-off-lock`, off): a prefix's first BM25 view is built on a
/// reader at the head it saw, outside the writer's lock, so appends and
/// other reads go on while it builds; `bm25_at` then brings it to the
/// head under the lock (`bm25_live_update`, E369's path), as it does a
/// view kept from an earlier run. The view is the same one; only the
/// lock is held for less time.
fn build_off_lock<S: Store>(srv: &Server<S>, name: &str, prefix: &str) -> Result<(), Error> {
    let gate = srv.search.build_gate(name, prefix);
    let _one = gate.lock().unwrap();
    let db = {
        let mut w = srv.writer.lock().unwrap();
        if srv.search.bm25_held(&mut w, name, prefix) {
            return Ok(());
        }
        open(&srv.heads, &mut w, name)?.1
    };
    let v = srv.read(|r| crate::dbsearch::bm25_build(r, &db, prefix))?;
    srv.search.bm25_put(name, prefix, db, v);
    Ok(())
}

/// `GET views` and `GET view` (E361): the views are brought to the head
/// under the writer's lock, then read like any other read.
fn views<S: Store>(srv: &Server<S>, name: &str, route: &str, req: &Request) -> Response {
    let Some(served) = &srv.views else {
        return Response::text(404, "no views: start with --views FILE");
    };
    if route == "views" {
        return Response::new(200).body("application/json", served.list_json());
    }
    let Some(rule) = req.param("name").and_then(|v| served.rule(&v).cloned()) else {
        return Response::text(404, "no such view");
    };
    let limit = match req.param("limit").map(|n| n.parse()) {
        None => usize::MAX,
        Some(Ok(n)) => n,
        Some(Err(_)) => return Response::text(400, "limit=N"),
    };
    let index = rule.kind == crate::dbviews::Kind::Index;
    let prefix = match (index, req.param("eq"), req.param("prefix")) {
        (true, Some(x), None) => {
            format!("{}{}", crate::dbviews::index_value(&x), crate::dbviews::US)
        }
        (false, None, p) => p.unwrap_or_default(),
        _ => return Response::text(400, "an index takes eq=X; a group, prefix=P"),
    };
    // Kept views (E380) that fail a read are built afresh, once.
    let (db, rows) = loop {
        let (db, state) = {
            let mut w = srv.writer.lock().unwrap();
            let db = match open(&srv.heads, &mut w, name) {
                Ok((_, db)) => db,
                Err(e) => return fail(e),
            };
            match served.at(&mut w, name, &db) {
                Ok(st) => (db, st),
                Err(e) => return fail(e),
            }
        };
        match srv.read(|repo| served.rows(repo, &state, &rule.name, &prefix, limit)) {
            Ok(rows) => break (db, rows),
            Err(_) if served.forget(name) => continue,
            Err(e) => return fail(e),
        }
    };
    let keys_only = matches!(req.param("keys").as_deref(), Some("1" | "true"));
    srv.read(|repo| {
        if !index {
            return pairs_response(req, rows);
        }
        let keys = rows
            .into_iter()
            .map(|(_, k)| String::from_utf8(k).unwrap_or_default());
        if keys_only {
            return pairs_response(req, keys.map(|k| (k, Vec::new())).collect());
        }
        let keys: std::collections::BTreeSet<String> = keys.collect();
        match repo.db_get_many(&db, &keys) {
            Ok(pairs) if pairs.len() == keys.len() => pairs_response(req, pairs),
            Ok(_) => fail(Error::Decode("view row not in kv")),
            Err(e) => fail(e),
        }
    })
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
    // A conflict means another process committed since the head was read;
    // the events are applied again over its head (their idempotency keys
    // keep a retry from applying one twice).
    let mut tries = 0;
    loop {
        match append_once(srv, &mut repo, name, req, &events) {
            Err(Error::RefConflict(_)) if srv.shared && tries < 16 => tries += 1,
            Ok(r) => return r,
            Err(e) => return fail(e),
        }
    }
}

/// One try of `append`: read the head, apply `events`, commit.
fn append_once<S: Store>(
    srv: &Server<S>,
    repo: &mut Repo<S>,
    name: &str,
    req: &Request,
    events: &[Event],
) -> Result<Response, Error> {
    let (head, opened) = open(&srv.heads, repo, name)?;
    let db = match srv.keep_counts {
        0 => opened,
        // E368: counts are built from a current view, so a view left
        // behind by deferred appends folds first.
        d if !opened.counts.is_some_and(|c| c.depth == d) => {
            let o = repo.db_catch_up(&opened)?;
            repo.db_keep_counts(&o, d)?
        }
        d => repo.db_keep_counts(&opened, d)?,
    };
    let seq = repo.db_head(&db.log)?;
    if let Some(want) = req.header("if-head")
        && want.trim().parse::<u64>().ok() != Some(seq)
    {
        return Ok(Response::new(409).body("application/json", head_json(head, seq)));
    }
    // E376: unchanged upserts dropped, with `--skip-unchanged`.
    let (kept, skipped);
    let events = if srv.skip_unchanged && !srv.shared {
        (kept, skipped) = srv.drop_unchanged(repo, name, &db, seq, events)?;
        &kept[..]
    } else {
        skipped = 0;
        events
    };
    // E368: with `--defer-kv` the kv view folds once enough events wait;
    // without it, a view left behind (a store written with it) folds first.
    let new = if srv.defer_kv > 0 {
        let d = repo.db_append_deferred(&db, events)?;
        if d.ids_at - d.kv_at >= srv.defer_kv {
            repo.db_catch_up(&d)?
        } else {
            d
        }
    } else {
        let d = repo.db_catch_up(&db)?;
        repo.db_append(&d, events)?
    };
    if srv.skip_unchanged && !srv.shared {
        srv.note_touched(name, &db, seq, &new, events);
    }
    if new == db {
        // Every event was a retry (or unchanged) and no view was folded: nothing to
        // commit. (A batch of retries over a commit left behind by
        // `--defer-kv` still commits the folded view.)
        return Ok(Response::new(200).body("application/json", head_json(head, seq)));
    }
    let msg = match skipped {
        0 => format!("db: {} event(s)", new.ids_at - seq),
        n => format!("db: {} event(s), {n} unchanged skipped", new.ids_at - seq),
    };
    let c = match head {
        Some(base) if srv.shared => repo.db_commit_at(&refname(name), base, &new, &msg)?,
        None if srv.shared => repo.db_commit_new(&refname(name), &new, &msg)?,
        _ => repo.db_commit(&refname(name), &new, &msg)?,
    };
    srv.heads
        .lock()
        .unwrap()
        .insert(name.to_string(), (Some(c), new));
    Ok(Response::new(200).body("application/json", head_json(Some(c), new.ids_at)))
}

/// Serve `repo`'s databases on 127.0.0.1:PORT until stopped; with
/// `readers`, reads use their own `Repo`s (E232).
pub fn serve<S: Store + Send + 'static>(
    repo: Repo<S>,
    readers: Option<ReaderFactory<S>>,
    (keep_counts, defer_kv, skip_unchanged): (u32, u64, bool),
    (search, views): (crate::dbsearch::SearchOpts, Option<crate::dbviews::Served>),
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
    // A handle set to see other writers' objects (`--shared-store`) is one
    // whose appends must not overwrite theirs either.
    let shared = repo.refresh_ledger;
    let state = Arc::new(
        Server::new(repo, readers)
            .keep_counts(keep_counts)
            .defer_kv(defer_kv)
            .skip_unchanged(skip_unchanged)
            .vector_index(search.index)
            .vector_threads(search.threads)
            .keep_search(search.keep)
            .build_off_lock(search.off_lock)
            .views(views)
            .shared_store(shared),
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
            say!(
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
    say!("{t} databases on http://127.0.0.1:{port}/db/");
    if password {
        say!("{t} password required (IKAM_AUTH)");
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

    /// `format=json` pairs as (key, value) strings.
    fn pairs_of(r: &Response) -> Vec<(String, String)> {
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let j = crate::json::parse(std::str::from_utf8(&r.body).unwrap()).unwrap();
        j.as_arr()
            .iter()
            .map(|p| {
                let s = |i: usize| p.as_arr()[i].as_str().unwrap().to_string();
                (s(0), s(1))
            })
            .collect()
    }

    #[test]
    fn views_follow_appends_and_match_a_build_at_the_head() {
        for shared in [false, true] {
            // E381: plain view writes change how chunks are stored, not
            // the views' rows.
            for plain in [false, true] {
                viewed(shared, plain);
            }
        }
    }

    fn viewed(shared: bool, plain: bool) {
        let spec = "by_status group t/o/ by=field:status sum=field:amount\n\
                    by_cust index t/o/ by=field:customer\n\
                    per_day group t/e/ by=seg:0\n";
        let served = || {
            let v = crate::dbviews::Served::new(crate::dbviews::parse(spec).unwrap()).unwrap();
            Some(v.plain_writes(plain))
        };
        let store = ikam_kernel::SharedStore::new(MemStore::default());
        let make = || -> Option<ReaderFactory<_>> {
            shared.then(|| Box::new(|w: &Repo<_>| w.sibling()) as ReaderFactory<_>)
        };
        let st = Server::new(Repo::new(store.clone()), make()).views(served());
        let h = |st: &Server<_>, q: &str| handle(st, &req("GET", q, b"", &[]));
        let ct = [("content-type", EVENTS)];
        let o = |c: &str, s: &str, a: i64| {
            format!(r#"{{"customer":"{c}","status":"{s}","amount":{a}}}"#)
        };
        let body = encode_events(&[
            Event::upsert("t/o/1", o("c1", "open", 5).as_bytes(), "1"),
            Event::upsert("t/o/2", o("c2", "open", 7).as_bytes(), "2"),
            Event::upsert("t/o/3", o("c1", "paid", 11).as_bytes(), "3"),
            Event::upsert("t/o/4", b"not json", "4"),
            Event::upsert("t/e/2026-11-21/a", b"x", "5"),
            Event::upsert("t/e/2026-11-21/b", b"x", "6"),
            Event::upsert("t/e/2026-11-22/a", b"x", "7"),
            Event::upsert("u/o/9", o("c1", "open", 100).as_bytes(), "8"),
        ]);
        let a = handle(&st, &req("POST", "/db/m/append", &body, &ct));
        assert_eq!(a.status, 200);
        let pairs = |st: &Server<_>, q: &str| pairs_of(&h(st, &format!("{q}&format=json")));
        let p = |v: &[(&str, &str)]| -> Vec<(String, String)> {
            v.iter()
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .collect()
        };
        assert_eq!(
            pairs(&st, "/db/m/view?name=by_status"),
            p(&[("open", "2,12"), ("paid", "1,11")])
        );
        assert_eq!(
            pairs(&st, "/db/m/view?name=per_day"),
            p(&[("2026-11-21", "2,0"), ("2026-11-22", "1,0")])
        );
        assert_eq!(
            pairs(&st, "/db/m/view?name=per_day&prefix=2026-11-22"),
            p(&[("2026-11-22", "1,0")])
        );
        let c1 = pairs(&st, "/db/m/view?name=by_cust&eq=c1");
        assert_eq!(
            c1.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
            ["t/o/1", "t/o/3"]
        );
        assert_eq!(c1[0].1, o("c1", "open", 5));
        assert_eq!(
            pairs(&st, "/db/m/view?name=by_cust&eq=c1&keys=1&limit=1"),
            p(&[("t/o/1", "")])
        );
        // Changes: a moved order, a removed one, a new customer.
        let more = encode_events(&[
            Event::upsert("t/o/1", o("c3", "paid", 6).as_bytes(), "9"),
            Event::remove("t/o/2", false, "10"),
            Event::remove("t/e/2026-11-22", true, "11"),
        ]);
        assert_eq!(
            handle(&st, &req("POST", "/db/m/append", &more, &ct)).status,
            200
        );
        let after: Vec<_> = ["by_status", "per_day"]
            .iter()
            .map(|v| pairs(&st, &format!("/db/m/view?name={v}")))
            .collect();
        assert_eq!(after[0], p(&[("paid", "2,17")]));
        assert_eq!(after[1], p(&[("2026-11-21", "2,0")]));
        assert_eq!(
            pairs(&st, "/db/m/view?name=by_cust&eq=c1"),
            p(&[("t/o/3", &o("c1", "paid", 11))])
        );
        // The same rows as a server that builds the views at the head.
        let fresh = Server::new(Repo::new(store.clone()), make()).views(served());
        for (i, v) in ["by_status", "per_day"].iter().enumerate() {
            assert_eq!(pairs(&fresh, &format!("/db/m/view?name={v}")), after[i]);
        }
        let list = String::from_utf8(h(&st, "/db/m/views").body).unwrap();
        assert!(
            list.contains(r#"{"name":"by_cust","kind":"index","prefix":"t/o/"}"#),
            "{list}"
        );
        // Refusals.
        assert_eq!(h(&st, "/db/m/view?name=nope").status, 404);
        assert_eq!(h(&st, "/db/m/view?name=by_cust").status, 400);
        assert_eq!(h(&st, "/db/m/view?name=by_status&eq=x").status, 400);
        let plain = Server::new(Repo::new(store.clone()), make());
        assert_eq!(h(&plain, "/db/m/view?name=by_status").status, 404);
    }

    #[test]
    fn kept_search_views_outlive_a_restart() {
        // E369: with `keep_search` a restarted server reads the BM25 view
        // back (writing nothing when the rows have not changed) and
        // updates it from the rows changed since; scores match a fresh
        // build, and the ref keeps the view through gc. One server runs
        // at a time, as a restart does (two writing one store at once do
        // not see each other's writes, kept views or not).
        let store = ikam_kernel::SharedStore::new(MemStore::default());
        let server = |keep: bool| Server::new(Repo::new(store.clone()), None).keep_search(keep);
        let ct = [("content-type", EVENTS)];
        let append = |st: &Server<_>, ev: &[Event]| {
            let r = handle(st, &req("POST", "/db/s/append", &encode_events(ev), &ct));
            assert_eq!(r.status, 200);
        };
        let q = "/db/s/search?q=json+sound&prefix=t/&k=5";
        let body = |st: &Server<_>| {
            let r = handle(st, &req("GET", q, b"", &[]));
            assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
            r.body
        };
        let rows: Vec<Event> = (0..60)
            .map(|i| {
                let text = format!(
                    "row {i} about {} and json",
                    ["maps", "sound", "files"][i % 3]
                );
                Event::upsert(&format!("t/{i:03}"), text.as_bytes(), &format!("{i}"))
            })
            .collect();
        let st = server(true);
        append(&st, &rows);
        let first = body(&st);
        drop(st);
        let refname = crate::dbsearch::bm25_ref("s", "t/");
        assert!(store.get_ref(&refname).unwrap().is_some());
        // A restart with no change: read back, nothing written.
        let before = store.ids().unwrap().len();
        assert_eq!(body(&server(true)), first);
        assert_eq!(store.ids().unwrap().len(), before, "no rebuild");
        // Changes while no server kept the view, then gc: read back and
        // brought to the head, the same as a fresh build.
        append(
            &server(false),
            &[
                Event::upsert("t/001", b"json sound sound sound", "a"),
                Event::remove("t/004", false, "b"),
            ],
        );
        assert!(Repo::new(store.clone()).gc().unwrap() > 0);
        let kept = body(&server(true));
        let fresh = body(&server(false));
        assert_eq!(kept, fresh);
        assert_ne!(kept, first);
        assert_eq!(body(&server(true)), fresh, "read back after the update");
        // A ref naming what is gone is built again.
        let mut w = Repo::new(store.clone());
        let (at, mut v) = w.bm25_live_load(&refname).unwrap().unwrap();
        v.base.root = ikam_kernel::Id::of_content(b"never stored");
        w.bm25_live_save(&refname, &at, &v).unwrap();
        assert!(w.bm25_live_load(&refname).is_err());
        drop(w);
        assert_eq!(body(&server(true)), fresh);
        assert!(Repo::new(store.clone()).bm25_live_load(&refname).is_ok());
        // A long prefix gets a bounded ref name (hashed), and works.
        let long = format!("t/{}", "x".repeat(200));
        let lq = format!("/db/s/search?q=json&prefix={long}");
        assert_eq!(
            handle(&server(true), &req("GET", &lq, b"", &[])).status,
            200
        );
        let lref = crate::dbsearch::bm25_ref("s", &long);
        assert!(lref.len() < 80, "{lref}");
        assert!(store.get_ref(&lref).unwrap().is_some());
        // Without the option nothing is kept.
        let off = server(false);
        handle(&off, &req("GET", "/db/s/search?q=json&prefix=u/", b"", &[]));
        let other = crate::dbsearch::bm25_ref("s", "u/");
        assert!(store.get_ref(&other).unwrap().is_none());
    }

    #[test]
    fn kept_views_outlive_a_restart() {
        // E380: with `keep` a restarted server reads its views back
        // (writing nothing when the rows have not changed) and brings them
        // to the head from the rows changed since; rows match a fresh
        // build, the ref keeps them through gc, and views kept for other
        // rules or naming what is gone are built again.
        let spec = "by_cust index t/o/ by=field:customer\nby_status group t/o/ by=field:status\n";
        let store = ikam_kernel::SharedStore::new(MemStore::default());
        let server = |spec: &str, keep: bool| {
            let v = crate::dbviews::Served::new(crate::dbviews::parse(spec).unwrap()).unwrap();
            Server::new(Repo::new(store.clone()), None).views(Some(v.keep(keep)))
        };
        let ct = [("content-type", EVENTS)];
        let append = |st: &Server<_>, ev: &[Event]| {
            let r = handle(st, &req("POST", "/db/v/append", &encode_events(ev), &ct));
            assert_eq!(r.status, 200);
        };
        let o = |c: usize, s: &str| format!(r#"{{"customer":"c{c}","status":"{s}"}}"#);
        let rows: Vec<Event> = (0..80)
            .map(|i| {
                Event::upsert(
                    &format!("t/o/{i:03}"),
                    o(i % 7, "open").as_bytes(),
                    &format!("{i}"),
                )
            })
            .collect();
        let read = |st: &Server<_>| -> Vec<Vec<u8>> {
            ["/db/v/view?name=by_cust&eq=c3", "/db/v/view?name=by_status"]
                .iter()
                .map(|q| {
                    let r = handle(st, &req("GET", q, b"", &[]));
                    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
                    r.body
                })
                .collect()
        };
        let st = server(spec, true);
        append(&st, &rows);
        let first = read(&st);
        drop(st);
        let refname = crate::dbviews::views_ref("v");
        assert!(store.get_ref(&refname).unwrap().is_some());
        // A restart with no change: read back, nothing written.
        let before = store.ids().unwrap().len();
        assert_eq!(read(&server(spec, true)), first);
        assert_eq!(store.ids().unwrap().len(), before, "no rebuild");
        // Changes while no server kept the views, then gc: read back and
        // brought to the head, the same as a fresh build.
        append(
            &server(spec, false),
            &[
                Event::upsert("t/o/003", o(1, "paid").as_bytes(), "a"),
                Event::remove("t/o/010", false, "b"),
                Event::upsert("t/o/900", o(3, "paid").as_bytes(), "c"),
            ],
        );
        assert!(Repo::new(store.clone()).gc().unwrap() > 0);
        let kept = read(&server(spec, true));
        let fresh = read(&server(spec, false));
        assert_eq!(kept, fresh);
        assert_ne!(kept, first);
        let before = store.ids().unwrap().len();
        assert_eq!(
            read(&server(spec, true)),
            fresh,
            "read back after the update"
        );
        assert_eq!(
            store.ids().unwrap().len(),
            before,
            "no rebuild after the update"
        );
        // Other rules: built again for them, with their own rows.
        let other = "by_cust index t/o/ by=field:status\nby_status group t/o/ by=field:status\n";
        let mut w = Repo::new(store.clone());
        let kept_for = format!("{:?}", crate::dbviews::parse(other).unwrap());
        assert!(w.views_load(&refname, &kept_for).unwrap().is_none());
        drop(w);
        let r = handle(
            &server(other, true),
            &req(
                "GET",
                "/db/v/view?name=by_cust&eq=paid&keys=1&format=json",
                b"",
                &[],
            ),
        );
        assert_eq!(r.status, 200);
        assert_eq!(pairs_of(&r).len(), 2);
        // A ref naming what is gone is built again.
        let rules = format!("{:?}", crate::dbviews::parse(spec).unwrap());
        let mut w = Repo::new(store.clone());
        let mut state = match w.views_load(&refname, &rules).unwrap() {
            Some(st) => st,
            None => w
                .views_load(&refname, &kept_for)
                .unwrap()
                .expect("kept for the other rules"),
        };
        state.roots.insert(
            "by_cust".into(),
            ikam_kernel::Id::of_content(b"never stored"),
        );
        w.views_save(&refname, &rules, &state).unwrap();
        assert!(w.views_load(&refname, &rules).is_err());
        drop(w);
        assert_eq!(read(&server(spec, true)), fresh);
        assert!(
            Repo::new(store.clone())
                .views_load(&refname, &rules)
                .unwrap()
                .is_some()
        );
        // Without the option nothing is kept.
        let off = server(spec, false);
        handle(&off, &req("GET", "/db/w/view?name=by_status", b"", &[]));
        assert!(
            store
                .get_ref(&crate::dbviews::views_ref("w"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn kept_views_with_missing_chunks_are_built_again() {
        // E380 review: a kept root can be read while chunks under it are
        // gone; the first read that fails builds the views afresh.
        let spec = "by_cust index t/o/ by=field:customer\n";
        let ct = [("content-type", EVENTS)];
        let rules = format!("{:?}", crate::dbviews::parse(spec).unwrap());
        let server = |s: &ikam_kernel::SharedStore<MemStore>, keep: bool| {
            let v = crate::dbviews::Served::new(crate::dbviews::parse(spec).unwrap()).unwrap();
            Server::new(Repo::new(s.clone()), None).views(Some(v.keep(keep)))
        };
        let rows = |n: usize| -> Vec<u8> {
            let ev: Vec<Event> = (0..3000)
                .map(|i| {
                    let v = format!(r#"{{"customer":"c{}"}}"#, i % n);
                    Event::upsert(&format!("t/o/{i:05}"), v.as_bytes(), &format!("{i}"))
                })
                .collect();
            encode_events(&ev)
        };
        let q = "/db/v/view?name=by_cust&eq=c7&keys=1";
        let (a, b) = (
            ikam_kernel::SharedStore::new(MemStore::default()),
            ikam_kernel::SharedStore::new(MemStore::default()),
        );
        for (s, n) in [(&a, 50), (&b, 51)] {
            let st = server(s, true);
            let r = handle(&st, &req("POST", "/db/v/append", &rows(n), &ct));
            assert_eq!(r.status, 200);
            assert_eq!(handle(&st, &req("GET", q, b"", &[])).status, 200);
        }
        let want = handle(&server(&a, false), &req("GET", q, b"", &[])).body;
        // `a` keeps `b`'s root, copied alone: the chunks it lists are not in `a`.
        let refname = crate::dbviews::views_ref("v");
        let theirs = Repo::new(b.clone())
            .views_load(&refname, &rules)
            .unwrap()
            .unwrap();
        let root = theirs.roots["by_cust"];
        let chunk = Repo::new(b.clone()).get(&root).unwrap();
        let mut w = Repo::new(a.clone());
        let mut ours = w.views_load(&refname, &rules).unwrap().unwrap();
        ours.roots.insert("by_cust".into(), root);
        w.plain_hot_writes = true;
        assert_eq!(w.put(&chunk).unwrap(), root);
        w.views_save(&refname, &rules, &ours).unwrap();
        let loaded = w
            .views_load(&refname, &rules)
            .unwrap()
            .expect("the root reads");
        assert!(
            w.view_prefix(
                &crate::dbviews::Served::graph_for_tests(spec),
                &loaded,
                "by_cust",
                "",
                10_000
            )
            .is_err(),
            "a chunk under the root is missing"
        );
        drop(w);
        let st = server(&a, true);
        let r = handle(&st, &req("GET", q, b"", &[]));
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        assert_eq!(r.body, want);
        // Built afresh and kept again.
        let again = Repo::new(a.clone())
            .views_load(&refname, &rules)
            .unwrap()
            .unwrap();
        assert_ne!(again.roots["by_cust"], root);
    }

    #[test]
    fn shared_store_servers_see_each_others_writes() {
        // E370: two servers with their own handles on one store, as two
        // processes would be. By default the first cannot read what the
        // second stores as derivations, and an append can drop the
        // other's; with `shared_store` (and `refresh_ledger`) both read
        // everything and no event is lost.
        for shared in [false, true] {
            let store = ikam_kernel::SharedStore::new(MemStore::default());
            let server = || {
                let mut r = Repo::new(store.clone());
                r.refresh_ledger = shared;
                Server::new(r, None).shared_store(shared)
            };
            let (a, b) = (server(), server());
            let ct = [("content-type", EVENTS)];
            let append = |st: &Server<_>, ev: &[Event]| {
                handle(st, &req("POST", "/db/s/append", &encode_events(ev), &ct)).status
            };
            let row = |i: usize, w: &str| {
                let text = format!("row {i} about {w} and json");
                Event::upsert(&format!("t/{i:03}"), text.as_bytes(), &format!("{w}{i}"))
            };
            let words = ["maps", "sound", "files"];
            let rows: Vec<Event> = (0..60).map(|i| row(i, words[i % 3])).collect();
            assert_eq!(append(&a, &rows), 200);
            let q = "/db/s/search?q=json+sound&prefix=t/&k=5";
            let search = |st: &Server<_>| handle(st, &req("GET", q, b"", &[]));
            assert_eq!(search(&a).status, 200);
            // b rewrites rows a has searched; then a searches, reads a row
            // and appends.
            assert_eq!(append(&b, &[row(1, "sound sound"), row(4, "sound")]), 200);
            let after = search(&a);
            let got = handle(&a, &req("GET", "/db/s/get?key=t/001", b"", &[]));
            let fresh = search(&server()).body;
            let more = append(&a, &[row(70, "late")]);
            if !shared {
                // What this option is for.
                assert!(after.status == 500 || got.status == 500 || more == 500);
                continue;
            }
            assert_eq!(after.status, 200);
            assert_eq!(got.status, 200);
            assert_eq!(got.body, b"row 1 about sound sound and json");
            assert_eq!(more, 200);
            // The same answer as a server that builds at that head.
            assert_eq!(after.body, fresh);
            // Appends that race, from two threads: each server appends
            // over heads the other moves; every event lands.
            std::thread::scope(|sc| {
                for (st, base) in [(&a, 100), (&b, 200)] {
                    sc.spawn(move || {
                        for i in 0..10 {
                            let w = if base == 100 { "a" } else { "b" };
                            assert_eq!(append(st, &[row(base + i, w)]), 200);
                        }
                    });
                }
            });
            // Two servers creating one database at once: both batches land.
            std::thread::scope(|sc| {
                for (st, base) in [(&a, 300), (&b, 400)] {
                    sc.spawn(move || {
                        let ev: Vec<Event> = (0..20)
                            .map(|i| {
                                let k = format!("n/{:03}", base + i);
                                Event::upsert(&k, b"x", &k)
                            })
                            .collect();
                        let body = encode_events(&ev);
                        let r = handle(st, &req("POST", "/db/new/append", &body, &ct));
                        assert_eq!(r.status, 200);
                    });
                }
            });
            let c = server();
            let r = handle(&c, &req("GET", "/db/new/count?prefix=n/", b"", &[]));
            assert!(String::from_utf8(r.body).unwrap().contains("40"));
            let n = |p: &str| {
                let r = handle(
                    &c,
                    &req("GET", &format!("/db/s/count?prefix={p}"), b"", &[]),
                );
                String::from_utf8(r.body).unwrap()
            };
            assert!(n("t/").contains("81"), "{}", n("t/"));
            assert!(Repo::new(store.clone()).fsck().unwrap().is_empty());
        }
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

    #[test]
    fn a_search_view_built_off_the_lock_is_the_same_view() {
        // E394: a server that builds a prefix's first view on a reader, at
        // the head it saw, holds the same view as one that builds it under
        // the writer's lock, whether or not appends land between the build
        // and the search, and answers the same.
        let words = ["json", "parser", "images", "schema", "quick", "files"];
        let rows = |from: usize, n: usize, tag: &str| {
            let ev: Vec<Event> = (from..from + n)
                .map(|i| {
                    let text = format!(
                        "{} {} {} row {i}",
                        words[i % 6],
                        words[(i / 6) % 6],
                        words[(i * 7) % 6]
                    );
                    Event::upsert(&format!("t/{i:05}"), text.as_bytes(), &format!("{tag}{i}"))
                })
                .collect();
            encode_events(&ev)
        };
        let ct = [("content-type", EVENTS)];
        let server = |off: bool| {
            let store = ikam_kernel::SharedStore::new(MemStore::default());
            let make: ReaderFactory<_> = Box::new(|w: &Repo<_>| w.sibling());
            Server::new(Repo::new(store), Some(make)).build_off_lock(off)
        };
        let q = "/db/s/search?q=json+schema&prefix=t/&k=20";
        for moved in [false, true] {
            let (on, off) = (server(false), server(true));
            for st in [&on, &off] {
                assert_eq!(
                    handle(st, &req("POST", "/db/s/append", &rows(0, 3000, "a"), &ct)).status,
                    200
                );
            }
            if moved {
                // Each builds at the head before the second batch: the
                // locked one by a search, the other off the lock, as when
                // appends land while it builds.
                assert_eq!(handle(&on, &req("GET", q, b"", &[])).status, 200);
                build_off_lock(&off, "s", "t/").unwrap();
                for st in [&on, &off] {
                    let more = rows(2500, 1200, "b");
                    assert_eq!(
                        handle(st, &req("POST", "/db/s/append", &more, &ct)).status,
                        200
                    );
                }
            }
            let (a, b) = (
                handle(&on, &req("GET", q, b"", &[])),
                handle(&off, &req("GET", q, b"", &[])),
            );
            assert_eq!((a.status, b.status), (200, 200));
            assert_eq!(a.body, b.body, "moved {moved}");
            assert_eq!(keys_of(&a).len(), 20);
            assert_eq!(
                on.search.bm25_held_view("s", "t/"),
                off.search.bm25_held_view("s", "t/"),
                "moved {moved}"
            );
        }
    }

    #[test]
    fn first_vector_build_reads_pages_and_answers_as_a_scan() {
        // E389: more rows than one build page (4,096), and keys a page
        // boundary splits by one byte; the index answers as a scan does,
        // exactly or (graph, any thread count) for the nearest row itself.
        use crate::dbsearch::VectorIndex;
        let n = 4096 * 2 + 7;
        let v = |i: usize| {
            // Angles up to 5.7 radians: no two rows point the same way.
            let a = (i as f32) * 0.0007;
            vec_body(&[a.cos(), a.sin(), 0.0])
        };
        let mut ev: Vec<Event> = (0..n)
            .map(|i| Event::upsert(&format!("v/{i:05}"), &v(i), &format!("e{i}")))
            .collect();
        ev.push(Event::upsert("v/04095", &v(4095), "dup"));
        let up = vec_body(&[0.0, 0.0, 1.0]);
        ev.push(Event::upsert("v/04095\u{1}", &up, "edge"));
        ev.push(Event::upsert("w/x", &v(1), "other"));
        let body = encode_events(&ev);
        let ct = [("content-type", EVENTS)];
        for (index, threads) in [
            (VectorIndex::Exact, 1),
            (VectorIndex::Hnsw, 1),
            (VectorIndex::Hnsw, 4),
        ] {
            let st = Server::new(Repo::new(MemStore::default()), None)
                .vector_index(index)
                .vector_threads(threads);
            assert_eq!(
                handle(&st, &req("POST", "/db/s/append", &body, &ct)).status,
                200
            );
            for q in [10usize, 4095, 4096, 8198] {
                let r = handle(&st, &req("POST", "/db/s/vsearch?prefix=v/&k=3", &v(q), &[]));
                assert_eq!(r.status, 200);
                let keys = keys_of(&r);
                // Neighbours 0.0007 radians apart tie in f32: the row is
                // among the best three.
                assert!(
                    keys.contains(&format!("v/{q:05}")),
                    "{index:?} {threads} {q} {keys:?}"
                );
                if index == VectorIndex::Exact {
                    assert_eq!(keys.len(), 3);
                }
            }
            // The key one byte past a page's last is read (a scan finds it;
            // a graph need not reach a row orthogonal to all the others).
            if index == VectorIndex::Exact {
                let r = handle(&st, &req("POST", "/db/s/vsearch?prefix=v/&k=1", &up, &[]));
                assert_eq!(keys_of(&r), ["v/04095\u{1}"]);
            }
            let r = handle(&st, &req("GET", "/db/s/count?prefix=v/", b"", &[]));
            assert!(
                String::from_utf8(r.body)
                    .unwrap()
                    .contains(&format!("{}", n + 1))
            );
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

    /// E376: with `skip_unchanged`, upserts of a key's current value are
    /// not logged, and the rows read the same as a server that logs all.
    #[test]
    fn unchanged_upserts_are_skipped_with_the_same_rows() {
        for defer in [0, 3, 1000] {
            let make = |skip: bool| {
                Server::new(Repo::new(MemStore::default()), None)
                    .defer_kv(defer)
                    .skip_unchanged(skip)
            };
            let (plain, skip) = (make(false), make(true));
            let ct = [("content-type", EVENTS)];
            let head = |st: &Server<MemStore>| {
                let r = handle(st, &req("GET", "/db/m/head", b"", &[]));
                String::from_utf8(r.body).unwrap()
            };
            let rows = |run: &str, change: &[usize]| -> Vec<Event> {
                (0..6)
                    .map(|i| {
                        let v = if change.contains(&i) {
                            format!("{i}-new")
                        } else {
                            format!("{i}")
                        };
                        Event::upsert(&format!("t/r/{i}"), v.as_bytes(), &format!("{run}-{i}"))
                    })
                    .collect()
            };
            let batches: Vec<Vec<Event>> = vec![
                rows("load", &[]),
                rows("again", &[]),
                rows("edit", &[2]),
                // Back to the old value while the edit may still be
                // pending: must be logged, not taken as unchanged.
                rows("back", &[]),
                vec![
                    Event::upsert("t/r/1", b"x", "twice-a"),
                    Event::upsert("t/r/1", b"1", "twice-b"),
                ],
                vec![Event::remove("t/r/0", false, "rm")],
                // A retry of an old write, then that old value again: the
                // retry is not applied, so the second write must be.
                vec![Event::upsert("t/r/2", b"2-new", "edit-again")],
                vec![
                    Event::upsert("t/r/2", b"2", "back-2"),
                    Event::upsert("t/r/2", b"2", "after-retry"),
                ],
                rows("last", &[5]),
            ];
            let mut skipped_any = false;
            for b in &batches {
                for st in [&plain, &skip] {
                    let r = handle(st, &req("POST", "/db/m/append", &encode_events(b), &ct));
                    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
                }
                let scan = |st: &Server<MemStore>| {
                    unframe(&handle(st, &req("GET", "/db/m/scan?prefix=t/", b"", &[])).body)
                };
                assert_eq!(scan(&plain), scan(&skip), "defer {defer}");
                skipped_any |= head(&plain) != head(&skip);
            }
            assert!(skipped_any, "defer {defer}: nothing was skipped");
            // The second load wrote nothing.
            let st = make(true);
            for b in &batches[..2] {
                handle(&st, &req("POST", "/db/m/append", &encode_events(b), &ct));
            }
            assert!(head(&st).starts_with("{\"head\":6,"), "{}", head(&st));
            // A read folds between appends: the next interval still skips.
            let st = make(true);
            handle(
                &st,
                &req("POST", "/db/m/append", &encode_events(&batches[0]), &ct),
            );
            handle(&st, &req("GET", "/db/m/scan?prefix=t/", b"", &[]));
            for run in ["edit", "edit-same"] {
                let b = rows(run, &[2]);
                handle(&st, &req("POST", "/db/m/append", &encode_events(&b), &ct));
            }
            assert!(
                head(&st).starts_with("{\"head\":7,"),
                "defer {defer}: {}",
                head(&st)
            );
        }
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
