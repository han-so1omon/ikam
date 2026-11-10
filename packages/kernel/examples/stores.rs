//! E122 store bake-off: the same workloads over every `Store` backend.
//!
//! `cargo run --release --features stores --example stores -- PHASE ARGS`
//!
//! - `corpus SRC WORK [BACKEND]`: copy every object of the FsStore at SRC (a
//!   store the CLI ingested from the bench corpora) into each backend; time
//!   the writes and one commit, then cold and warm reads of every object in a
//!   shuffled order, each compared byte for byte with the source.
//! - `db WORK [BACKEND]`: the database layer: 1,000 single appends each
//!   committed (a ref move), 10,000 appends in batches of 1,000, and 1,000 bare
//!   ref swaps; with the files each commit adds or changes (what a sync to an
//!   object store would upload).
//! - `build BACKEND DIR N` / `probe BACKEND DIR N`: N small synthetic objects
//!   (64 to 511 bytes, id = BLAKE3 of the bytes), then, in a fresh process with
//!   the page cache dropped, open time and cold, missing and warm lookups.
//! - `crash WORK BACKEND KILLS`: a child appends and commits database events
//!   in a loop and is killed with SIGKILL at a random moment, KILLS times; after
//!   each kill the store is reopened, fsck must be clean, every acknowledged
//!   commit must be there, and the views rebuilt from the log must equal the
//!   committed ones byte for byte.
//!
//! - `crash-big WORK BACKEND KILLS N` (E174): `crash` on a store that already
//!   holds N small contents (the synthetic bytes, put as ikam contents; built
//!   once and kept in WORK). After each kill: the time to reopen, every
//!   acknowledged commit there, the views rebuilt equal, the store lists at
//!   least N objects, and 100,000 of the contents read back byte for byte.
//!   After the last kill, a full fsck.
//!
//! - `churn BACKEND DIR N` (E176): on a store `build` made, delete 3 in 10
//!   of the objects and replace 1 in 10 with new bytes, then (PackStore
//!   backends) time the pack cleaner, and check every object reads as it
//!   should after a reopen.
//!
//! - `crash-store WORK BACKEND KILLS N` (E174): the store alone, without the
//!   Repo above it, on a store that already holds N synthetic objects: a
//!   child writes batches of objects (100 bytes to 200 KB) and a commit object
//!   naming them and the previous commit, moves `heads/log` to it, and
//!   acknowledges; it is killed at random. After each kill: reopen time, the
//!   acknowledged commit reachable from the ref, the last 2,000 commits and
//!   every object they name read back and hash to their ids, and 100,000 of
//!   the N objects read back.
//!
//! - `writers DIR THREADS COMMITS MODE` (E177): THREADS writers, each making
//!   COMMITS commits on its own ref in one packs-fast store, flushing alone
//!   under a shared lock (`alone`) or sharing flushes (`group`); then a
//!   reopen checks every chain.
//! - `crash-group WORK KILLS THREADS` (E177): writers sharing flushes in a
//!   child killed at random; after each kill every writer's ref is at or past
//!   its last acknowledged commit and its chain reads back.
//!
//! Every line printed is one JSON object.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use ikam_kernel::{Error, Event, FsStore, Id, Repo, Store};

const BACKENDS: &[&str] = &[
    "files",
    "packs",
    "packs-lazy",
    "packs-log",
    "packs-fast",
    "packs-filter",
    "packs-fast-filter",
    "sqlite",
    "sqlite-normal",
    "lmdb",
    "redb",
    "fjall",
    "fjall-kvsep",
    "rocksdb",
    "rocksdb-blob",
    "hybrid-refs",
    "hybrid-index",
];

/// Any backend behind one type, so a `Repo` can run over each.
struct Dyn(Box<dyn Store>);

impl Store for Dyn {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        self.0.write(id, encoded)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.0.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        self.0.read(id)
    }
    fn has(&self, id: &Id) -> bool {
        self.0.has(id)
    }
    fn ids(&self) -> Result<Vec<Id>, Error> {
        self.0.ids()
    }
    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        self.0.ids_with_prefix(prefix)
    }
    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        self.0.delete(id)
    }
    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        self.0.get_ref(name)
    }
    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        self.0.set_ref(name, expected, new)
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        self.0.refs()
    }
}

fn open(name: &str, dir: &Path) -> Dyn {
    use ikam_kernel::backends::*;
    let s: Box<dyn Store> = match name {
        "files" => Box::new(FsStore::open(dir).unwrap()),
        "packs" => Box::new(ikam_kernel::backends::PackStore::open(dir).unwrap()),
        "packs-lazy" => Box::new(
            PackStore::open_with(
                dir,
                PackOptions {
                    lazy_checkpoint: true,
                    ..PackOptions::default()
                },
            )
            .unwrap(),
        ),
        "packs-log" => Box::new(
            PackStore::open_with(
                dir,
                PackOptions {
                    ref_log: true,
                    ..PackOptions::default()
                },
            )
            .unwrap(),
        ),
        "packs-fast" => Box::new(
            PackStore::open_with(
                dir,
                PackOptions {
                    lazy_checkpoint: true,
                    ref_log: true,
                    ..PackOptions::default()
                },
            )
            .unwrap(),
        ),
        "packs-filter" => Box::new(
            PackStore::open_with(
                dir,
                PackOptions {
                    filters: true,
                    ..PackOptions::default()
                },
            )
            .unwrap(),
        ),
        "packs-fast-filter" => Box::new(
            PackStore::open_with(
                dir,
                PackOptions {
                    lazy_checkpoint: true,
                    ref_log: true,
                    filters: true,
                },
            )
            .unwrap(),
        ),
        "sqlite" => Box::new(sqlite::open(dir, sqlite::Sync::Full).unwrap()),
        "sqlite-normal" => Box::new(sqlite::open(dir, sqlite::Sync::Normal).unwrap()),
        "lmdb" => Box::new(lmdb::open(dir).unwrap()),
        "redb" => Box::new(redb_store::open(dir).unwrap()),
        "fjall" => Box::new(fjall_store::open(dir, false).unwrap()),
        "fjall-kvsep" => Box::new(fjall_store::open(dir, true).unwrap()),
        "rocksdb" => Box::new(rocks::open(dir, false).unwrap()),
        "rocksdb-blob" => Box::new(rocks::open(dir, true).unwrap()),
        "hybrid-refs" => Box::new(hybrid::RefsHybrid::open(dir).unwrap()),
        "hybrid-index" => Box::new(hybrid::IndexHybrid::open(dir).unwrap()),
        _ => panic!("unknown backend {name}"),
    };
    Dyn(s)
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(n + 8);
        while out.len() < n {
            out.extend_from_slice(&self.next().to_le_bytes());
        }
        out.truncate(n);
        out
    }
}

fn shuffle<T>(v: &mut [T], seed: u64) {
    let mut r = Rng(seed);
    for i in (1..v.len()).rev() {
        v.swap(i, r.below(i as u64 + 1) as usize);
    }
}

/// Files under `dir`, their apparent bytes and their allocated bytes.
fn disk(dir: &Path) -> (u64, u64, u64) {
    use std::os::unix::fs::MetadataExt;
    let (mut files, mut len, mut alloc) = (0, 0, 0);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let m = e.metadata().unwrap();
            if m.is_dir() {
                stack.push(e.path());
            } else {
                files += 1;
                len += m.len();
                alloc += m.blocks() * 512;
            }
        }
    }
    (files, len, alloc)
}

/// Each file's (length, modified time): what changed between two states.
fn files_state(dir: &Path) -> BTreeMap<PathBuf, (u64, i128)> {
    use std::os::unix::fs::MetadataExt;
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let m = e.metadata().unwrap();
            if m.is_dir() {
                stack.push(e.path());
            } else {
                out.insert(
                    e.path(),
                    (
                        m.len(),
                        m.mtime() as i128 * 1_000_000_000 + m.mtime_nsec() as i128,
                    ),
                );
            }
        }
    }
    out
}

/// New or changed files, the bytes they grew by, and the size of those that changed.
fn diff(a: &BTreeMap<PathBuf, (u64, i128)>, b: &BTreeMap<PathBuf, (u64, i128)>) -> (u64, u64, u64) {
    let (mut n, mut grown, mut size) = (0, 0, 0);
    for (p, &(len, t)) in b {
        match a.get(p) {
            Some(&(l0, t0)) if l0 == len && t0 == t => {}
            Some(&(l0, _)) => {
                n += 1;
                grown += len.saturating_sub(l0);
                size += len;
            }
            None => {
                n += 1;
                grown += len;
                size += len;
            }
        }
    }
    (n, grown, size)
}

fn drop_caches() -> bool {
    let _ = Command::new("sync").status();
    std::fs::write("/proc/sys/vm/drop_caches", "3").is_ok()
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn wanted(arg: Option<&String>) -> Vec<&'static str> {
    match arg {
        Some(b) => vec![BACKENDS.iter().find(|x| *x == b).copied().expect("backend")],
        None => BACKENDS.to_vec(),
    }
}

fn corpus(src: &Path, work: &Path, only: Option<&String>) {
    let source = FsStore::open(src).unwrap();
    let mut objects: Vec<(Id, Vec<u8>)> = source
        .ids()
        .unwrap()
        .into_iter()
        .map(|id| (id, source.read(&id).unwrap()))
        .collect();
    objects.sort_by_key(|(id, _)| *id);
    let bytes: u64 = objects.iter().map(|(_, b)| b.len() as u64).sum();
    let mut order: Vec<usize> = (0..objects.len()).collect();
    shuffle(&mut order, 7);
    for name in wanted(only) {
        let dir = work.join(name);
        let _ = std::fs::remove_dir_all(&dir);
        let t = Instant::now();
        {
            let mut s = open(name, &dir);
            for (id, b) in &objects {
                s.write(*id, b).unwrap();
            }
            s.set_ref("heads/main", None, objects[0].0).unwrap();
        }
        let write = t.elapsed();
        let (files, len, alloc) = disk(&dir);
        let cold = drop_caches();
        let t = Instant::now();
        let s = open(name, &dir);
        let open_ms = ms(t.elapsed());
        let read = |s: &Dyn| {
            let t = Instant::now();
            for &i in &order {
                let (id, b) = &objects[i];
                assert!(s.read(id).unwrap() == *b, "{name}: object {id} differs");
            }
            t.elapsed()
        };
        let cold_read = read(&s);
        let warm_read = read(&s);
        let t = Instant::now();
        let ids = s.ids().unwrap().len();
        let list_ms = ms(t.elapsed());
        assert_eq!(ids, objects.len());
        println!(
            "{{\"phase\":\"corpus\",\"backend\":\"{name}\",\"objects\":{},\"bytes\":{bytes},\"write_s\":{:.3},\"write_mb_s\":{:.1},\"files\":{files},\"apparent\":{len},\"allocated\":{alloc},\"cold\":{cold},\"open_ms\":{open_ms:.2},\"cold_read_s\":{:.3},\"cold_read_mb_s\":{:.1},\"warm_read_s\":{:.3},\"warm_read_mb_s\":{:.1},\"list_ms\":{list_ms:.1}}}",
            objects.len(),
            write.as_secs_f64(),
            bytes as f64 / 1e6 / write.as_secs_f64(),
            cold_read.as_secs_f64(),
            bytes as f64 / 1e6 / cold_read.as_secs_f64(),
            warm_read.as_secs_f64(),
            bytes as f64 / 1e6 / warm_read.as_secs_f64(),
        );
    }
}

fn events(rng: &mut Rng, from: u64, n: u64) -> Vec<Event> {
    (from..from + n)
        .map(|i| {
            let key = format!("k/{:05}", rng.below(5_000));
            let len = 16 + rng.below(400) as usize;
            let value: Vec<u8> = format!("{i}:")
                .into_bytes()
                .into_iter()
                .chain(rng.bytes(len))
                .collect();
            Event::upsert(&key, &value, &format!("e{i}"))
        })
        .collect()
}

fn db(work: &Path, only: Option<&String>) {
    for name in wanted(only) {
        let dir = work.join(format!("db-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        let mut rng = Rng(11);
        let mut repo = Repo::new(open(name, &dir));
        let mut d = repo.db_empty().unwrap();
        let mut single = Vec::new();
        let (mut changed, mut grown, mut size) = (0u64, 0u64, 0u64);
        let mut before = files_state(&dir);
        for i in 0..1_000 {
            let t = Instant::now();
            d = repo.db_append(&d, &events(&mut rng, i, 1)).unwrap();
            repo.db_commit("heads/db", &d, "append").unwrap();
            single.push(ms(t.elapsed()));
            let after = files_state(&dir);
            let (n, g, s) = diff(&before, &after);
            changed += n;
            grown += g;
            size += s;
            before = after;
        }
        let t = Instant::now();
        for b in 0..10 {
            d = repo
                .db_append(&d, &events(&mut rng, 1_000 + b * 1_000, 1_000))
                .unwrap();
            repo.db_commit("heads/db", &d, "batch").unwrap();
        }
        let batch = t.elapsed();
        let rebuilt = repo.db_rebuild(&d.log).unwrap();
        assert!(
            rebuilt.ids == d.ids && rebuilt.kv == d.kv,
            "{name}: rebuild differs"
        );
        drop(repo);
        // Bare ref swaps: nothing else waiting to commit.
        let mut s = open(name, &dir);
        let mut prev = s.get_ref("heads/db").unwrap();
        let ids: Vec<Id> = (0..1_000u32)
            .map(|i| Id::from_bytes(*blake3::hash(&i.to_le_bytes()).as_bytes()))
            .collect();
        let mut swaps = Vec::new();
        for id in &ids {
            let t = Instant::now();
            s.set_ref("heads/swap", prev.filter(|_| !swaps.is_empty()), *id)
                .unwrap();
            swaps.push(ms(t.elapsed()) * 1e3);
            prev = Some(*id);
        }
        let (files, len, alloc) = disk(&dir);
        let mean = single.iter().sum::<f64>() / single.len() as f64;
        println!(
            "{{\"phase\":\"db\",\"backend\":\"{name}\",\"single_ms_mean\":{mean:.3},\"single_ms_p50\":{:.3},\"single_ms_p99\":{:.3},\"batch_10k_s\":{:.3},\"swap_us_p50\":{:.1},\"swap_us_p99\":{:.1},\"files_changed_per_commit\":{:.2},\"bytes_grown_per_commit\":{},\"changed_file_bytes_per_commit\":{},\"files\":{files},\"apparent\":{len},\"allocated\":{alloc}}}",
            pct(&mut single, 0.5),
            pct(&mut single, 0.99),
            batch.as_secs_f64(),
            pct(&mut swaps, 0.5),
            pct(&mut swaps, 0.99),
            changed as f64 / 1_000.0,
            grown / 1_000,
            size / 1_000,
        );
    }
}

fn synthetic(i: u64) -> (Id, Vec<u8>) {
    let mut r = Rng(i);
    let len = 64 + r.below(448) as usize;
    let mut b = i.to_le_bytes().to_vec();
    b.extend(r.bytes(len - 8));
    (Id::from_bytes(*blake3::hash(&b).as_bytes()), b)
}

fn build(name: &str, dir: &Path, n: u64) {
    let _ = std::fs::remove_dir_all(dir);
    let t = Instant::now();
    let mut s = open(name, dir);
    let mut head = None;
    for i in 0..n {
        let (id, b) = synthetic(i);
        s.write(id, &b).unwrap();
        if (i + 1) % 100_000 == 0 || i + 1 == n {
            s.set_ref("heads/scale", head, id).unwrap();
            head = Some(id);
        }
    }
    drop(s);
    let (files, len, alloc) = disk(dir);
    println!(
        "{{\"phase\":\"build\",\"backend\":\"{name}\",\"n\":{n},\"build_s\":{:.1},\"files\":{files},\"apparent\":{len},\"allocated\":{alloc}}}",
        t.elapsed().as_secs_f64()
    );
}

fn probe(name: &str, dir: &Path, n: u64) {
    let cold = drop_caches();
    let t = Instant::now();
    let s = open(name, dir);
    let open_ms = ms(t.elapsed());
    let t = Instant::now();
    assert!(s.get_ref("heads/scale").unwrap().is_some());
    let first_ref_ms = ms(t.elapsed());
    let mut r = Rng(99);
    let mut lookup = |count: usize, missing: bool| {
        let mut v = Vec::with_capacity(count);
        for _ in 0..count {
            let i = if missing { n + r.below(n) } else { r.below(n) };
            let (id, b) = synthetic(i);
            let t = Instant::now();
            if missing {
                assert!(!s.has(&id));
            } else {
                assert!(s.read(&id).unwrap() == b);
            }
            v.push(ms(t.elapsed()) * 1e3);
        }
        v
    };
    let mut cold_reads = lookup(2_000, false);
    let mut missing = lookup(10_000, true);
    let mut warm = lookup(20_000, false);
    let mut warm2 = lookup(20_000, false);
    let _ = &mut warm;
    println!(
        "{{\"phase\":\"probe\",\"backend\":\"{name}\",\"n\":{n},\"cold\":{cold},\"open_ms\":{open_ms:.2},\"first_ref_ms\":{first_ref_ms:.2},\"cold_read_us_p50\":{:.1},\"cold_read_us_p99\":{:.1},\"missing_us_p50\":{:.1},\"missing_us_p99\":{:.1},\"read_us_p50\":{:.1},\"read_us_p99\":{:.1}}}",
        pct(&mut cold_reads, 0.5),
        pct(&mut cold_reads, 0.99),
        pct(&mut missing, 0.5),
        pct(&mut missing, 0.99),
        pct(&mut warm2, 0.5),
        pct(&mut warm2, 0.99),
    );
}

/// The child of `crash`: append and commit forever, acknowledging each commit.
fn crash_child(name: &str, dir: &Path) {
    let mut repo = Repo::new(open(name, dir));
    if std::env::var_os("CRASH_EMPTY_SEEDS").is_some() {
        // E174: on a big store a fresh handle's first write reads every
        // stored content for the seed index (E130); scope it to nothing.
        repo.set_seed_scope(Some(Vec::new()));
    }
    let mut d = match repo.store().get_ref("heads/db").unwrap() {
        Some(c) => repo.db_at(&c).unwrap(),
        None => repo.db_empty().unwrap(),
    };
    let mut seq = repo.db_head(&d.log).unwrap();
    let mut rng = Rng(seq ^ 0xc0ffee);
    let out = std::io::stdout();
    loop {
        // Unreachable large content between commits, so kills land mid-write too.
        let len = 50_000 + rng.below(200_000) as usize;
        let blob = rng.bytes(len);
        repo.put_content(&blob).unwrap();
        let n = 1 + rng.below(40);
        d = repo.db_append(&d, &events(&mut rng, seq, n)).unwrap();
        let before = repo.store().get_ref("heads/db").unwrap();
        if let Err(e) = repo.db_commit("heads/db", &d, "crash") {
            let now = repo.store().get_ref("heads/db").unwrap();
            eprintln!(
                "{name}: child commit failed after {seq} events: {e}; ref before {before:?}, now {now:?}"
            );
            std::process::exit(3);
        }
        seq = repo.db_head(&d.log).unwrap();
        let mut o = out.lock();
        writeln!(o, "ack {seq}").unwrap();
        o.flush().unwrap();
    }
}

fn crash(work: &Path, name: &str, kills: u32) {
    let dir = work.join(format!("crash-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    let exe = std::env::current_exe().unwrap();
    let mut rng = Rng(5);
    let (mut ok, mut lost, mut bad, mut failed) = (0, 0, 0, 0);
    let mut max_seq = 0u64;
    for _ in 0..kills {
        let mut child = Command::new(&exe)
            .args(["crash-child", name, dir.to_str().unwrap()])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let acks = std::thread::spawn(move || {
            let mut last = 0u64;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(n) = line.strip_prefix("ack ") {
                    last = n.parse().unwrap();
                }
            }
            last
        });
        std::thread::sleep(Duration::from_millis(150 + rng.below(1_500)));
        let early = child.try_wait().unwrap();
        child.kill().unwrap(); // SIGKILL
        child.wait().unwrap();
        if let Some(status) = early {
            failed += 1;
            eprintln!("{name}: child exited before the kill: {status}");
        }
        let acked = acks.join().unwrap();
        let mut repo = Repo::new(open(name, &dir));
        let head = repo.store().get_ref("heads/db").unwrap();
        let verdict = (|| -> Result<u64, String> {
            let Some(c) = head else {
                return if acked == 0 {
                    Ok(0)
                } else {
                    Err("ref lost".into())
                };
            };
            let d = repo.db_at(&c).map_err(|e| e.to_string())?;
            let seq = repo.db_head(&d.log).map_err(|e| e.to_string())?;
            if seq < acked {
                return Err(format!("acked {acked} but store has {seq}"));
            }
            let problems = repo.fsck().map_err(|e| e.to_string())?;
            if !problems.is_empty() {
                return Err(format!("fsck: {} problems", problems.len()));
            }
            let rebuilt = repo.db_rebuild(&d.log).map_err(|e| e.to_string())?;
            if rebuilt.ids != d.ids || rebuilt.kv != d.kv {
                return Err("rebuilt views differ".into());
            }
            Ok(seq)
        })();
        match verdict {
            Ok(seq) => {
                ok += 1;
                max_seq = max_seq.max(seq);
            }
            Err(e) if e.contains("acked") || e.contains("ref lost") => {
                lost += 1;
                eprintln!("{name}: {e}");
            }
            Err(e) => {
                bad += 1;
                eprintln!("{name}: {e}");
            }
        }
    }
    println!(
        "{{\"phase\":\"crash\",\"backend\":\"{name}\",\"kills\":{kills},\"clean\":{ok},\"lost_acked\":{lost},\"corrupt\":{bad},\"child_failed\":{failed},\"events\":{max_seq}}}"
    );
}

fn crash_big(work: &Path, name: &str, kills: u32, n: u64) {
    let dir = work.join(format!("crash-big-{name}-{n}"));
    let marker = dir.join("BUILT");
    if !marker.exists() {
        let _ = std::fs::remove_dir_all(&dir);
        let t = Instant::now();
        let mut repo = Repo::new(open(name, &dir));
        let mut last = None;
        for i in 0..n {
            last = Some(repo.put_content(&synthetic(i).1).unwrap().id);
        }
        drop(repo);
        open(name, &dir).set_ref("heads/scale", None, last.unwrap()).unwrap();
        std::fs::write(&marker, b"").unwrap();
        println!(
            "{{\"phase\":\"crash-big-build\",\"backend\":\"{name}\",\"n\":{n},\"build_s\":{:.1}}}",
            t.elapsed().as_secs_f64()
        );
    }
    let exe = std::env::current_exe().unwrap();
    // SAFETY: set before any thread is started.
    unsafe { std::env::set_var("CRASH_EMPTY_SEEDS", "1") };
    let mut rng = Rng(7);
    let (mut ok, mut lost, mut bad, mut failed) = (0, 0, 0, 0);
    let mut max_seq = 0u64;
    let mut open_ms = Vec::new();
    for k in 0..kills {
        let mut child = Command::new(&exe)
            .args(["crash-child", name, dir.to_str().unwrap()])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let acks = std::thread::spawn(move || {
            let mut last = 0u64;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(v) = line.strip_prefix("ack ") {
                    last = v.parse().unwrap();
                }
            }
            last
        });
        // A default PackStore takes about 0.4 s to open at 10M: give the
        // child time to open and commit before the kill.
        std::thread::sleep(Duration::from_millis(700 + rng.below(2_000)));
        let early = child.try_wait().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        if let Some(status) = early {
            failed += 1;
            eprintln!("{name}: child exited before the kill: {status}");
        }
        let acked = acks.join().unwrap();
        let t = Instant::now();
        let mut repo = Repo::new(open(name, &dir));
        open_ms.push(ms(t.elapsed()));
        let head = repo.store().get_ref("heads/db").unwrap();
        let verdict = (|| -> Result<u64, String> {
            if repo.store().get_ref("heads/scale").map_err(|e| e.to_string())?.is_none() {
                return Err("scale ref lost".into());
            }
            let Some(c) = head else {
                return if acked == 0 { Ok(0) } else { Err("ref lost".into()) };
            };
            let d = repo.db_at(&c).map_err(|e| e.to_string())?;
            let seq = repo.db_head(&d.log).map_err(|e| e.to_string())?;
            if seq < acked {
                return Err(format!("acked {acked} but store has {seq}"));
            }
            let rebuilt = repo.db_rebuild(&d.log).map_err(|e| e.to_string())?;
            if rebuilt.ids != d.ids || rebuilt.kv != d.kv {
                return Err("rebuilt views differ".into());
            }
            let count = repo.store().ids().map_err(|e| e.to_string())?.len() as u64;
            if count < n {
                return Err(format!("{count} objects listed, built {n}"));
            }
            let mut r = Rng(1000 + k as u64);
            for _ in 0..100_000 {
                let b = synthetic(r.below(n)).1;
                if repo.read_content(&Id::of_content(&b)).map_err(|e| e.to_string())? != b {
                    return Err("content differs".into());
                }
            }
            Ok(seq)
        })();
        match verdict {
            Ok(seq) => {
                ok += 1;
                max_seq = max_seq.max(seq);
            }
            Err(e) if e.contains("acked") || e.contains("lost") => {
                lost += 1;
                eprintln!("{name}: {e}");
            }
            Err(e) => {
                bad += 1;
                eprintln!("{name}: {e}");
            }
        }
    }
    let t = Instant::now();
    let fsck = Repo::new(open(name, &dir)).fsck().map(|p| p.len()).unwrap_or(usize::MAX);
    let fsck_s = t.elapsed().as_secs_f64();
    let (files, len, _) = disk(&dir);
    println!(
        "{{\"phase\":\"crash-big\",\"fsck_problems\":{fsck},\"fsck_s\":{fsck_s:.1},\"backend\":\"{name}\",\"n\":{n},\"kills\":{kills},\"clean\":{ok},\"lost_acked\":{lost},\"corrupt\":{bad},\"child_failed\":{failed},\"events\":{max_seq},\"reopen_ms_p50\":{:.1},\"reopen_ms_max\":{:.1},\"files\":{files},\"apparent\":{len}}}",
        pct(&mut open_ms.clone(), 0.5),
        open_ms.iter().cloned().fold(0.0, f64::max),
    );
}

fn churned(i: u64) -> Option<Vec<u8>> {
    match i % 10 {
        0..=2 => None,
        3 => {
            let mut b = synthetic(i).1;
            b.reverse();
            Some(b)
        }
        _ => Some(synthetic(i).1),
    }
}

fn churn(name: &str, dir: &Path, n: u64) {
    use ikam_kernel::backends::{PackOptions, PackStore};
    let opts = match name {
        "packs" => PackOptions::default(),
        "packs-fast" => PackOptions { lazy_checkpoint: true, ref_log: true, ..PackOptions::default() },
        "packs-fast-filter" => PackOptions { lazy_checkpoint: true, ref_log: true, filters: true },
        _ => panic!("churn is for PackStore backends"),
    };
    let t = Instant::now();
    let mut s = PackStore::open_with(dir, opts).unwrap();
    for i in 0..n {
        let id = synthetic(i).0;
        match churned(i) {
            None => s.delete(&id).unwrap(),
            Some(b) if i % 10 == 3 => s.replace(id, &b).unwrap(),
            Some(_) => {}
        }
    }
    let head = s.get_ref("heads/scale").unwrap();
    s.set_ref("heads/scale", head, synthetic(n - 1).0).unwrap();
    let churn_s = t.elapsed().as_secs_f64();
    let (_, before, _) = disk(dir);
    let t = Instant::now();
    let c = s.compact().unwrap();
    let compact_s = t.elapsed().as_secs_f64();
    drop(s);
    let (_, after, _) = disk(dir);
    let t = Instant::now();
    let s = PackStore::open_with(dir, opts).unwrap();
    let open_ms = ms(t.elapsed());
    let mut wrong = 0u64;
    for i in (0..n).step_by(97) {
        let id = synthetic(i).0;
        match churned(i) {
            None => wrong += s.has(&id) as u64,
            Some(b) => wrong += (s.read(&id).ok() != Some(b)) as u64,
        }
    }
    println!(
        "{{\"phase\":\"churn\",\"backend\":\"{name}\",\"n\":{n},\"churn_s\":{churn_s:.1},\"compact_s\":{compact_s:.1},\"kept\":{},\"dropped\":{},\"packs_before\":{},\"packs_after\":{},\"bytes_before\":{before},\"bytes_after\":{after},\"open_ms\":{open_ms:.1},\"sampled_wrong\":{wrong}}}",
        c.kept, c.dropped, c.packs_before, c.packs_after
    );
}

fn commit_obj(seq: u64, prev: Option<Id>, ids: &[Id]) -> (Id, Vec<u8>) {
    let mut b = b"commit".to_vec();
    b.extend(seq.to_le_bytes());
    b.extend(prev.map_or([0u8; 32], |p| *p.as_bytes()));
    for id in ids {
        b.extend(id.as_bytes());
    }
    (Id::from_bytes(*blake3::hash(&b).as_bytes()), b)
}

fn parse_commit(b: &[u8]) -> Option<(u64, Option<Id>, Vec<Id>)> {
    let rest = b.strip_prefix(b"commit")?;
    let seq = u64::from_le_bytes(rest.get(..8)?.try_into().ok()?);
    let prev: [u8; 32] = rest.get(8..40)?.try_into().ok()?;
    let prev = (prev != [0u8; 32]).then(|| Id::from_bytes(prev));
    let ids = rest[40..]
        .chunks_exact(32)
        .map(|c| Id::from_bytes(c.try_into().unwrap()))
        .collect();
    Some((seq, prev, ids))
}

fn crash_store_child(name: &str, dir: &Path) {
    let mut s = open(name, dir);
    let mut head = s.get_ref("heads/log").unwrap();
    let mut seq = match head {
        Some(h) => parse_commit(&s.read(&h).unwrap()).unwrap().0,
        None => 0,
    };
    let mut rng = Rng(seq ^ 0xfeed ^ std::process::id() as u64);
    let out = std::io::stdout();
    loop {
        seq += 1;
        let mut ids = Vec::new();
        for _ in 0..1 + rng.below(8) {
            let cap = if rng.below(4) == 0 { 200_000 } else { 2_000 };
            let len = 100 + rng.below(cap) as usize;
            let mut b = seq.to_le_bytes().to_vec();
            b.extend(rng.bytes(len));
            let id = Id::from_bytes(*blake3::hash(&b).as_bytes());
            s.write(id, &b).unwrap();
            ids.push(id);
        }
        let (cid, cb) = commit_obj(seq, head, &ids);
        s.write(cid, &cb).unwrap();
        s.set_ref("heads/log", head, cid).unwrap();
        head = Some(cid);
        let mut o = out.lock();
        writeln!(o, "ack {seq}").unwrap();
        o.flush().unwrap();
    }
}

fn crash_store(work: &Path, name: &str, kills: u32, n: u64) {
    let dir = work.join(format!("crash-store-{name}-{n}"));
    let marker = dir.join("BUILT");
    if !marker.exists() {
        build(name, &dir, n);
        std::fs::write(&marker, b"").unwrap();
    }
    let exe = std::env::current_exe().unwrap();
    let mut rng = Rng(11);
    let (mut ok, mut lost, mut bad, mut failed) = (0, 0, 0, 0);
    let (mut max_seq, mut checked) = (0u64, 0u64);
    let mut open_ms = Vec::new();
    for k in 0..kills {
        let mut child = Command::new(&exe)
            .args(["crash-store-child", name, dir.to_str().unwrap()])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let acks = std::thread::spawn(move || {
            let mut last = 0u64;
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(v) = line.strip_prefix("ack ") {
                    last = v.parse().unwrap();
                }
            }
            last
        });
        std::thread::sleep(Duration::from_millis(500 + rng.below(1_500)));
        let early = child.try_wait().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        if let Some(status) = early {
            failed += 1;
            eprintln!("{name}: child exited before the kill: {status}");
        }
        let acked = acks.join().unwrap();
        let t = Instant::now();
        let s = open(name, &dir);
        open_ms.push(ms(t.elapsed()));
        let verdict = (|| -> Result<u64, String> {
            if s.get_ref("heads/scale").map_err(|e| e.to_string())?.is_none() {
                return Err("scale ref lost".into());
            }
            let Some(h) = s.get_ref("heads/log").map_err(|e| e.to_string())? else {
                return if acked == 0 { Ok(0) } else { Err("ref lost".into()) };
            };
            let mut at = Some(h);
            let mut top = None;
            let mut walked = 0;
            while let Some(c) = at {
                if walked == 2_000 {
                    break;
                }
                let b = s.read(&c).map_err(|e| format!("commit unreadable: {e}"))?;
                if *blake3::hash(&b).as_bytes() != *c.as_bytes() {
                    return Err("commit hash differs".into());
                }
                let (seq, prev, ids) = parse_commit(&b).ok_or("not a commit")?;
                top.get_or_insert(seq);
                for id in ids {
                    let o = s.read(&id).map_err(|e| format!("object unreadable: {e}"))?;
                    if *blake3::hash(&o).as_bytes() != *id.as_bytes() {
                        return Err("object hash differs".into());
                    }
                    checked += 1;
                }
                at = prev;
                walked += 1;
            }
            let top = top.unwrap_or(0);
            if top < acked {
                return Err(format!("acked {acked} but ref is at {top}"));
            }
            let mut r = Rng(2000 + k as u64);
            for _ in 0..100_000 {
                let (id, b) = synthetic(r.below(n));
                if s.read(&id).map_err(|e| e.to_string())? != b {
                    return Err("seed object differs".into());
                }
            }
            Ok(top)
        })();
        drop(s);
        match verdict {
            Ok(seq) => {
                ok += 1;
                max_seq = max_seq.max(seq);
            }
            Err(e) if e.contains("acked") || e.contains("lost") => {
                lost += 1;
                eprintln!("{name}: {e}");
            }
            Err(e) => {
                bad += 1;
                eprintln!("{name}: {e}");
            }
        }
    }
    let (files, len, _) = disk(&dir);
    println!(
        "{{\"phase\":\"crash-store\",\"backend\":\"{name}\",\"n\":{n},\"kills\":{kills},\"clean\":{ok},\"lost_acked\":{lost},\"corrupt\":{bad},\"child_failed\":{failed},\"commits\":{max_seq},\"objects_checked\":{checked},\"reopen_ms_p50\":{:.1},\"reopen_ms_max\":{:.1},\"files\":{files},\"apparent\":{len}}}",
        pct(&mut open_ms.clone(), 0.5),
        open_ms.iter().cloned().fold(0.0, f64::max),
    );
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let p = |i: usize| PathBuf::from(&a[i]);
    match a.first().map(String::as_str) {
        Some("corpus") => corpus(&p(1), &p(2), a.get(3)),
        Some("db") => db(&p(1), a.get(2)),
        Some("build") => build(&a[1], &p(2), a[3].parse().unwrap()),
        Some("probe") => probe(&a[1], &p(2), a[3].parse().unwrap()),
        Some("crash") => crash(&p(1), &a[2], a[3].parse().unwrap()),
        Some("churn") => churn(&a[1], &p(2), a[3].parse().unwrap()),
        Some("crash-store") => crash_store(&p(1), &a[2], a[3].parse().unwrap(), a[4].parse().unwrap()),
        Some("crash-store-child") => crash_store_child(&a[1], &p(2)),
        Some("writers") => writers(&p(1), a[2].parse().unwrap(), a[3].parse().unwrap(), &a[4]),
        Some("crash-group") => crash_group(&p(1), a[2].parse().unwrap(), a[3].parse().unwrap()),
        Some("crash-group-child") => crash_group_child(&p(1), a[2].parse().unwrap()),
        Some("crash-big") => crash_big(&p(1), &a[2], a[3].parse().unwrap(), a[4].parse().unwrap()),
        Some("crash-child") => crash_child(&a[1], &p(2)),
        _ => eprintln!("see the module docs for phases"),
    }
}

fn fast_pack(dir: &Path) -> ikam_kernel::backends::PackStore {
    use ikam_kernel::backends::*;
    PackStore::open_with(
        dir,
        PackOptions {
            lazy_checkpoint: true,
            ref_log: true,
            ..PackOptions::default()
        },
    )
    .unwrap()
}

/// One writer's next commit: 1 to 4 objects of 100 to 2,000 bytes, then a
/// commit object naming them and the writer's previous commit.
fn writer_batch(t: u64, seq: u64, head: Option<Id>, rng: &mut Rng) -> (Vec<(Id, Vec<u8>)>, Id) {
    let mut objs = Vec::new();
    let mut ids = Vec::new();
    for _ in 0..1 + rng.below(4) {
        let mut b = t.to_le_bytes().to_vec();
        b.extend(seq.to_le_bytes());
        let len = 100 + rng.below(1_900) as usize;
        b.extend(rng.bytes(len));
        let id = Id::from_bytes(*blake3::hash(&b).as_bytes());
        ids.push(id);
        objs.push((id, b));
    }
    let (cid, cb) = commit_obj(seq, head, &ids);
    objs.push((cid, cb));
    (objs, cid)
}

/// Walk up to `limit` commits back from `head`, checking every hash.
/// Returns the top sequence number and how many commits were walked.
fn walk_chain(s: &dyn Store, head: Id, limit: usize) -> Result<(u64, usize), String> {
    let mut at = Some(head);
    let mut top = None;
    let mut walked = 0;
    while let Some(c) = at {
        if walked == limit {
            break;
        }
        let b = s.read(&c).map_err(|e| format!("commit unreadable: {e}"))?;
        if *blake3::hash(&b).as_bytes() != *c.as_bytes() {
            return Err("commit hash differs".into());
        }
        let (seq, prev, ids) = parse_commit(&b).ok_or("not a commit")?;
        top.get_or_insert(seq);
        for id in ids {
            let o = s.read(&id).map_err(|e| format!("object unreadable: {e}"))?;
            if *blake3::hash(&o).as_bytes() != *id.as_bytes() {
                return Err("object hash differs".into());
            }
        }
        at = prev;
        walked += 1;
    }
    Ok((top.unwrap_or(0), walked))
}

/// E177 `writers DIR THREADS COMMITS MODE`: THREADS writers each make
/// COMMITS commits on their own ref (`heads/wN`) in one fresh packs-fast
/// store, either each flushing alone under a shared lock (`alone`) or
/// sharing flushes (`group`). Then a reopen checks every chain.
fn writers(dir: &Path, threads: u64, commits: u64, mode: &str) {
    use ikam_kernel::backends::GroupCommit;
    use std::sync::{Arc, Mutex};
    let _ = std::fs::remove_dir_all(dir);
    let store = fast_pack(dir);
    enum Shared {
        Alone(Mutex<ikam_kernel::backends::PackStore>),
        Group(GroupCommit),
    }
    let shared = Arc::new(match mode {
        "alone" => Shared::Alone(Mutex::new(store)),
        "group" => Shared::Group(GroupCommit::new(store).unwrap()),
        _ => panic!("mode is alone or group"),
    });
    let t0 = Instant::now();
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let mut rng = Rng(77 + t);
                let name = format!("heads/w{t}");
                let mut head = None;
                let mut lat = Vec::with_capacity(commits as usize);
                for seq in 1..=commits {
                    let (objs, cid) = writer_batch(t, seq, head, &mut rng);
                    let s = Instant::now();
                    match &*shared {
                        Shared::Alone(m) => {
                            let mut st = m.lock().unwrap();
                            for (id, b) in &objs {
                                st.write(*id, b).unwrap();
                            }
                            st.set_ref(&name, head, cid).unwrap();
                        }
                        Shared::Group(g) => g.commit(&objs, &name, head, cid).unwrap(),
                    }
                    lat.push(ms(s.elapsed()));
                    head = Some(cid);
                }
                lat
            })
        })
        .collect();
    let mut lat: Vec<f64> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
    let secs = t0.elapsed().as_secs_f64();
    let shared = Arc::try_unwrap(shared).ok().unwrap();
    let flushes = match &shared {
        Shared::Group(g) => g.flushes(),
        Shared::Alone(_) => threads * commits,
    };
    drop(shared);
    let s = fast_pack(dir);
    let mut bad = 0;
    for t in 0..threads {
        let r = s
            .get_ref(&format!("heads/w{t}"))
            .map_err(|e| e.to_string())
            .and_then(|h| h.ok_or("ref lost".to_string()))
            .and_then(|h| walk_chain(&s, h, usize::MAX));
        match r {
            Ok((top, walked)) if top == commits && walked as u64 == commits => {}
            Ok((top, walked)) => {
                bad += 1;
                eprintln!("w{t}: top {top}, walked {walked}");
            }
            Err(e) => {
                bad += 1;
                eprintln!("w{t}: {e}");
            }
        }
    }
    let total = threads * commits;
    println!(
        "{{\"phase\":\"writers\",\"mode\":\"{mode}\",\"threads\":{threads},\"commits\":{total},\"secs\":{secs:.2},\"commits_per_s\":{:.0},\"flushes\":{flushes},\"commits_per_flush\":{:.2},\"lat_ms_p50\":{:.2},\"lat_ms_p99\":{:.2},\"bad_chains\":{bad}}}",
        total as f64 / secs,
        total as f64 / flushes.max(1) as f64,
        pct(&mut lat, 0.5),
        pct(&mut lat, 0.99),
    );
}

/// E177 `crash-group WORK KILLS THREADS`: THREADS writers sharing flushes
/// in a child that is killed at random, KILLS times; after each kill every
/// writer's ref must be at or past its last acknowledged commit, and the
/// last 2,000 commits of each chain must read back and hash to their ids.
fn crash_group(work: &Path, kills: u32, threads: u64) {
    let dir = work.join(format!("crash-group-{threads}"));
    let _ = std::fs::remove_dir_all(&dir);
    drop(fast_pack(&dir));
    let exe = std::env::current_exe().unwrap();
    let mut rng = Rng(13);
    let (mut ok, mut lost, mut bad, mut failed) = (0, 0, 0, 0);
    let mut open_ms = Vec::new();
    let mut acked_total = 0u64;
    for k in 0..kills {
        let mut child = Command::new(&exe)
            .args(["crash-group-child", dir.to_str().unwrap(), &threads.to_string()])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let n = threads as usize;
        let acks = std::thread::spawn(move || {
            let mut last = vec![0u64; n];
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Some(v) = line.strip_prefix("ack ") {
                    let mut it = v.split(' ');
                    let t: usize = it.next().unwrap().parse().unwrap();
                    let seq: u64 = it.next().unwrap().parse().unwrap();
                    last[t] = last[t].max(seq);
                }
            }
            last
        });
        std::thread::sleep(Duration::from_millis(300 + rng.below(1_200)));
        let early = child.try_wait().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        if let Some(status) = early {
            failed += 1;
            eprintln!("child exited before the kill: {status}");
        }
        let acked = acks.join().unwrap();
        let t = Instant::now();
        let s = fast_pack(&dir);
        open_ms.push(ms(t.elapsed()));
        let mut verdict = Ok(());
        for (w, &a) in acked.iter().enumerate() {
            let r = match s.get_ref(&format!("heads/w{w}")) {
                Err(e) => Err(e.to_string()),
                Ok(None) if a == 0 => Ok((0, 0)),
                Ok(None) => Err("ref lost".to_string()),
                Ok(Some(h)) => walk_chain(&s, h, 2_000),
            };
            match r {
                Ok((top, _)) if top >= a => {}
                Ok((top, _)) => verdict = Err(format!("kill {k} w{w}: acked {a} but ref is at {top}")),
                Err(e) => verdict = Err(format!("kill {k} w{w}: {e}")),
            }
        }
        acked_total = acked.iter().sum::<u64>().max(acked_total);
        drop(s);
        match verdict {
            Ok(()) => ok += 1,
            Err(e) if e.contains("acked") || e.contains("lost") => {
                lost += 1;
                eprintln!("{e}");
            }
            Err(e) => {
                bad += 1;
                eprintln!("{e}");
            }
        }
    }
    let (files, len, _) = disk(&dir);
    println!(
        "{{\"phase\":\"crash-group\",\"threads\":{threads},\"kills\":{kills},\"clean\":{ok},\"lost_acked\":{lost},\"corrupt\":{bad},\"child_failed\":{failed},\"commits_acked\":{acked_total},\"reopen_ms_p50\":{:.1},\"reopen_ms_max\":{:.1},\"files\":{files},\"apparent\":{len}}}",
        pct(&mut open_ms.clone(), 0.5),
        open_ms.iter().cloned().fold(0.0, f64::max),
    );
}

fn crash_group_child(dir: &Path, threads: u64) {
    use ikam_kernel::backends::GroupCommit;
    use std::sync::Arc;
    let g = Arc::new(GroupCommit::new(fast_pack(dir)).unwrap());
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let g = g.clone();
            std::thread::spawn(move || {
                let name = format!("heads/w{t}");
                let mut head = g.get_ref(&name).unwrap();
                let mut seq = 0;
                if let Some(h) = head {
                    let st = fast_view(&g, h);
                    seq = st;
                }
                let mut rng = Rng(seq ^ (t << 32) ^ std::process::id() as u64);
                let out = std::io::stdout();
                loop {
                    seq += 1;
                    let (objs, cid) = writer_batch(t, seq, head, &mut rng);
                    g.commit(&objs, &name, head, cid).unwrap();
                    head = Some(cid);
                    let mut o = out.lock();
                    writeln!(o, "ack {t} {seq}").unwrap();
                    o.flush().unwrap();
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}

/// The sequence number of commit `h`, read through the group's store.
fn fast_view(g: &ikam_kernel::backends::GroupCommit, h: Id) -> u64 {
    parse_commit(&g.read(&h).unwrap()).unwrap().0
}
