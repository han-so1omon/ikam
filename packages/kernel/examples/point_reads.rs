//! E335: where a read of one row by key goes. A database of `rows` rows
//! of 1,024-byte values (256 f32s, as a vector row) in memory; for random
//! keys, the time of the walk to the row's node (`graph_node`), of reading
//! its value (`db_get`), and of a batch of 50 (`graph_nodes_in` and the
//! values), with what the store returned for each: objects, bytes, and
//! the time of the store's own reads. Then the same with E294's read
//! cache, and the cost of hashing those bytes (every read is verified).
//!
//! `cargo run --release --example point_reads -- [rows] [appends]`
//!
//! `appends`: the rows written in that many appends (default 1). `CACHE=0`
//! or `CACHE=1` runs only the pass without or with the read cache.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Instant;

use ikam_kernel::vectors::from_f32s;
use ikam_kernel::{Error, Event, Id, MemStore, Repo, Store};

/// Reads, bytes and nanoseconds of the store's own reads.
#[derive(Default)]
struct Count {
    reads: Cell<u64>,
    bytes: Cell<u64>,
    ns: Cell<u64>,
}

/// MemStore with its reads counted.
struct Counted {
    inner: MemStore,
    count: Rc<Count>,
}

impl Counted {
    fn note(&self, out: &Result<Vec<u8>, Error>, t: Instant) {
        self.count
            .ns
            .set(self.count.ns.get() + t.elapsed().as_nanos() as u64);
        if let Ok(b) = out {
            self.count.reads.set(self.count.reads.get() + 1);
            self.count
                .bytes
                .set(self.count.bytes.get() + b.len() as u64);
        }
    }
}

impl Store for Counted {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        self.inner.write(id, encoded)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.inner.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        let t = Instant::now();
        let out = self.inner.read(id);
        self.note(&out, t);
        out
    }
    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        let t = Instant::now();
        let out = self.inner.read_head(id, n);
        self.note(&out, t);
        out
    }
    fn stored_len(&self, id: &Id) -> Result<usize, Error> {
        self.inner.stored_len(id)
    }
    fn has(&self, id: &Id) -> bool {
        self.inner.has(id)
    }
    fn ids(&self) -> Result<Vec<Id>, Error> {
        self.inner.ids()
    }
    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        self.inner.ids_with_prefix(prefix)
    }
    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        self.inner.delete(id)
    }
    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        self.inner.get_ref(name)
    }
    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        self.inner.set_ref(name, expected, new)
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        self.inner.refs()
    }
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let rows = args.first().copied().unwrap_or(100_000);
    let appends = args.get(1).copied().unwrap_or(1).max(1);
    let count = Rc::new(Count::default());
    let mut repo = Repo::new(Counted {
        inner: MemStore::default(),
        count: count.clone(),
    });
    let mut rng = Lcg(3);
    let keys: Vec<String> = (0..rows).map(|i| format!("v/{i:07}")).collect();
    let mut db = repo.db_empty().unwrap();
    let t = Instant::now();
    for part in keys.chunks(rows.div_ceil(appends)) {
        let events: Vec<Event> = part
            .iter()
            .map(|k| {
                let v: Vec<f32> = (0..256)
                    .map(|_| (rng.next() % 2001) as f32 / 1000.0 - 1.0)
                    .collect();
                Event::upsert(k, &from_f32s(&v), &format!("s{k}"))
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
    }
    println!(
        "{{\"rows\":{rows},\"appends\":{appends},\"append_ms\":{:.0}}}",
        t.elapsed().as_secs_f64() * 1e3
    );
    let probe: Vec<&String> = (0..2000)
        .map(|_| &keys[rng.next() as usize % rows])
        .collect();
    let reset = || {
        count.reads.set(0);
        count.bytes.set(0);
        count.ns.set(0);
    };
    let report = |what: &str, cache: usize, n: usize, t: Instant| {
        let n = n as f64;
        println!(
            "{{\"read\":\"{what}\",\"read_cache\":{cache},\"us_a_row\":{:.1},\"objects_a_row\":{:.2},\"bytes_a_row\":{:.0},\"store_us_a_row\":{:.1}}}",
            t.elapsed().as_secs_f64() * 1e6 / n,
            count.reads.get() as f64 / n,
            count.bytes.get() as f64 / n,
            count.ns.get() as f64 / 1e3 / n
        );
    };
    // `CACHE=0` or `CACHE=1`: only that pass (for a profile).
    let caches: Vec<usize> = match std::env::var("CACHE").as_deref() {
        Ok("0") => vec![0],
        Ok("1") => vec![512 << 20],
        _ => vec![0, 512 << 20],
    };
    for cache in caches {
        repo.read_cache = cache;
        // Twice with the cache: the first pass fills it.
        for pass in 0..if cache > 0 { 2 } else { 1 } {
            let tag = |s: &str| {
                if cache > 0 && pass == 0 {
                    format!("{s} (filling)")
                } else {
                    s.to_string()
                }
            };
            reset();
            let t = Instant::now();
            for k in &probe {
                repo.graph_node(&db.kv, k).unwrap().unwrap();
            }
            report(&tag("graph_node"), cache, probe.len(), t);
            reset();
            let t = Instant::now();
            for k in &probe {
                repo.db_get(&db, k).unwrap().unwrap();
            }
            report(&tag("db_get"), cache, probe.len(), t);
            reset();
            let t = Instant::now();
            for batch in probe.chunks(50) {
                let want: BTreeSet<String> = batch.iter().map(|k| k.to_string()).collect();
                let nodes = repo.graph_nodes_in(&db.kv, &want).unwrap();
                assert_eq!(nodes.len(), want.len());
            }
            report(&tag("graph_nodes_in x50"), cache, probe.len(), t);
        }
    }
    // What a kept zstd context holds after a chunk-sized frame (E335, E336).
    let sample: Vec<u8> = (0..8192u32).flat_map(|i| (i % 251).to_be_bytes()).collect();
    let mut cctx = zstd::zstd_safe::CCtx::create();
    cctx.set_parameter(zstd::zstd_safe::CParameter::CompressionLevel(3))
        .unwrap();
    let mut frame = Vec::with_capacity(sample.len() + 1024);
    cctx.compress2(&mut frame, &sample).unwrap();
    let mut dctx = zstd::zstd_safe::DCtx::create();
    let mut back = Vec::with_capacity(sample.len());
    dctx.decompress(&mut back, &frame).unwrap();
    assert_eq!(back, sample);
    println!(
        "{{\"zstd_kept_bytes\":{{\"compress_level3\":{},\"decompress\":{}}}}}",
        cctx.sizeof(),
        dctx.sizeof()
    );
    // Hashing: what verifying those bytes costs, per KB.
    for size in [1024usize, 4096, 16384, 65536] {
        let buf = vec![7u8; size];
        let n = (64 << 20) / size;
        let t = Instant::now();
        let mut x = 0u8;
        for _ in 0..n {
            x ^= blake3::hash(&buf).as_bytes()[0];
        }
        println!(
            "{{\"blake3_bytes\":{size},\"us\":{:.2},\"x\":{x}}}",
            t.elapsed().as_secs_f64() * 1e6 / n as f64
        );
    }
}
