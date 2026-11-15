//! E268: an incremental cold pass against a full repack.
//! `cargo run --release --example cold_pass -- [files|db] [N...] [cap=MINUTES] [group] [nofull]`
//!
//! E260 found a full repack costs 29 to 87 s on 2 MB stores and did not
//! finish in 12 minutes on a 100,000-row database. A full repack re-plans
//! every live file. `Repo::repack_incremental` (src/coldpass.rs) works only
//! on what is new since its last pass: it recompresses the new contents
//! with the existing dictionary (and, with `group`, groups the new small
//! contents first), then records the roots it saw under `meta/cold-pass`.
//!
//! Each workload runs on one store, in order:
//!
//! 1. Load. A first pass (no marker yet: everything is new).
//! 2. A session of edits. A second pass (only the session is new).
//! 3. A pass with nothing new.
//! 4. gc, then a full repack of a copy of the same store, for comparison,
//!    with a time cap (`cap=`, default 15 minutes) after which it is
//!    reported as not finished.
//!
//! - `files` (W1, as E260): 200 generated files (half markdown, half CSV);
//!   the session is 1,000 one-line edits, committed every 50.
//! - `db` (W2, as E260, with its opt-ins: deferred sync, `plain_db_events`,
//!   `needed_index`): a generated table of N rows (default 20,000 and
//!   100,000); the session is 2,000 upserts in batches of 100, committed
//!   after each batch.
//!
//! `kill [N] [K] [full]` kills K passes (with `full`, K full repacks)
//! at random points (see `kill_case`); `crash [N] [K] [synced]` stops a
//! recorded full repack after its writes at K points (see `crash_case`);
//! `gccheck [N]` checks a store before and after gc and a full repack
//! (see `gc_check`); `repro [N] [nopass] [save=DIR]` checks each step of
//! the database session and its full repack separately (see `repro`), and
//! `full DIR` runs a full repack on a copy of a saved store and fsck. `group` runs every pass with
//! `ColdPassOpts { group: true }` (off by default); `nofull` leaves out
//! the full repack.
//!
//! Every pass is followed by a full read (every file, or a database scan)
//! checked against the expected bytes, `db_rebuild` for the database, and
//! `fsck_deep`. Prints one JSON line per step. Fixtures are synthetic.

mod db_common;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use db_common::{Lcg, row};
use ikam_kernel::{ColdPassOpts, Db, Edit, Error, Event, FsStore, Id, Object, Repo, Store};

type R = Repo<FsStore>;

/// `group`: passes with `ColdPassOpts { group: true }`.
static GROUP: AtomicBool = AtomicBool::new(false);

fn opts() -> ColdPassOpts {
    ColdPassOpts {
        group: GROUP.load(Relaxed),
    }
}
/// `nofull`: no full repack for comparison.
/// `crash ... synced`: replay on a store that writes each object at once
/// (`FsStore::open`), so only the order of replacements can hurt.
static SYNCED: AtomicBool = AtomicBool::new(false);
static FULL: AtomicBool = AtomicBool::new(true);

fn baseline(workload: &str, read_ms: f64) {
    println!(
        "{{\"workload\":\"{workload}\",\"step\":\"read before any pass\",\"group\":{},\"read_ms\":{read_ms:.0}}}",
        GROUP.load(Relaxed)
    );
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn dir_bytes(p: &Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_bytes(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let dest = to.join(e.file_name());
        if e.metadata().unwrap().is_dir() {
            copy_dir(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

/// One pass, timed, then the store is read back and checked.
fn pass(repo: &mut R, dir: &Path, label: &str, workload: &str, check: &dyn Fn(&mut R) -> f64) {
    let disk_before = dir_bytes(dir);
    let t = Instant::now();
    let p = repo.repack_incremental_with(opts()).unwrap();
    let pass_ms = ms(t);
    let read_ms = check(repo);
    let t = Instant::now();
    assert!(repo.fsck_deep().unwrap().is_empty(), "fsck_deep");
    let fsck_ms = ms(t);
    println!(
        "{{\"workload\":\"{workload}\",\"step\":\"{label}\",\"group\":{},\"pass_ms\":{pass_ms:.0},\"new\":{},\"new_bytes_before\":{},\"new_bytes_after\":{},\"disk_before\":{disk_before},\"disk_after\":{},\"read_ms\":{read_ms:.0},\"fsck_ms\":{fsck_ms:.0}}}",
        GROUP.load(Relaxed),
        p.new,
        p.before,
        p.after,
        dir_bytes(dir),
    );
}

/// gc on the store, then a full repack of a copy, capped.
fn compare(
    repo: &mut R,
    dir: &Path,
    workload: &str,
    cap: Duration,
    opt_in: bool,
    check: &dyn Fn(&mut R) -> f64,
) {
    let t = Instant::now();
    let freed = repo.gc().unwrap();
    let gc_ms = ms(t);
    let disk_passes = dir_bytes(dir);
    let read_passes = check(repo);
    let (mut outcome, mut repack_ms, mut disk_full, mut read_full) = (
        "skipped",
        "null".to_string(),
        "null".to_string(),
        "null".to_string(),
    );
    if FULL.load(Relaxed) {
        // The last pass moved its ref, so every waiting object is durable
        // (E227) before the copy.
        let copy = tempfile::tempdir().unwrap();
        copy_dir(dir, copy.path());
        let path = copy.path().to_path_buf();
        let (tx, rx) = mpsc::channel();
        // The full repack runs on its own thread so a cap can end the
        // wait; a capped repack is left running and the process exits
        // after main.
        std::thread::spawn(move || {
            let mut full = Repo::new(FsStore::open_deferred(&path).unwrap());
            full.plain_db_events = opt_in;
            full.needed_index = opt_in;
            let t = Instant::now();
            full.repack().unwrap();
            full.gc().unwrap();
            let _ = tx.send((ms(t), dir_bytes(&path), full));
        });
        match rx.recv_timeout(cap) {
            Ok((m, d, mut full)) => {
                outcome = "done";
                repack_ms = format!("{m:.0}");
                disk_full = d.to_string();
                read_full = format!("{:.0}", check(&mut full));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => outcome = "not finished",
            Err(mpsc::RecvTimeoutError::Disconnected) => outcome = "failed",
        }
        // Keep the copy for a capped repack still running.
        std::mem::forget(copy);
    }
    println!(
        "{{\"workload\":\"{workload}\",\"step\":\"gc after passes, then full repack of a copy\",\"gc_ms\":{gc_ms:.0},\"gc_freed\":{freed},\"disk_after_passes_and_gc\":{disk_passes},\"read_ms_after_passes\":{read_passes:.0},\"full_repack\":\"{outcome}\",\"full_repack_ms\":{repack_ms},\"disk_after_full_repack\":{disk_full},\"read_ms_after_full_repack\":{read_full},\"cap_s\":{}}}",
        cap.as_secs()
    );
}

/// A generated file: markdown paragraphs or CSV rows (as E260).
fn file(rng: &mut Lcg, i: usize) -> (String, Vec<String>) {
    if i.is_multiple_of(2) {
        let lines = (0..40)
            .map(|j| {
                format!(
                    "Paragraph {j} of note {i}: reading {} at step {}.",
                    rng.next() % 9973,
                    rng.next() % 100
                )
            })
            .collect();
        (format!("notes/n{i:03}.md"), lines)
    } else {
        let lines = (0..200).map(|j| row(rng, (i * 1_000 + j) as u64)).collect();
        (format!("tables/t{i:03}.csv"), lines)
    }
}

fn tree_of(repo: &R) -> Id {
    match repo.get(&repo.resolve("main").unwrap()).unwrap() {
        Object::Commit(c) => c.tree,
        _ => unreachable!(),
    }
}

/// Reads every file on `main` and checks its bytes. Returns the time.
fn read_files(repo: &R, docs: &BTreeMap<String, Vec<u8>>) -> f64 {
    let t = Instant::now();
    let top = tree_of(repo);
    for (path, bytes) in docs {
        let mut id = top;
        for part in path.split('/') {
            let Object::Tree(entries) = repo.get(&id).unwrap() else {
                panic!("not a tree")
            };
            id = entries.into_iter().find(|e| e.name == part).unwrap().id;
        }
        assert_eq!(&repo.read_content(&id).unwrap(), bytes, "{path}");
    }
    ms(t)
}

fn files_case(cap: Duration) {
    const FILES: usize = 200;
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(FsStore::open_deferred(dir.path()).unwrap());
    let mut rng = Lcg(7);
    let mut docs: Vec<(String, Vec<String>)> = (0..FILES).map(|i| file(&mut rng, i)).collect();
    let mut edits = Vec::new();
    for (path, lines) in &docs {
        let id = repo.put_content(lines.join("\n").as_bytes()).unwrap().id;
        edits.push(Edit::Put {
            path: path.clone(),
            id,
        });
    }
    let mut work = repo.edit_tree(None, &edits).unwrap();
    repo.commit("main", work, "setup").unwrap();
    let expected = |docs: &[(String, Vec<String>)]| -> BTreeMap<String, Vec<u8>> {
        docs.iter()
            .map(|(p, l)| (p.clone(), l.join("\n").into_bytes()))
            .collect()
    };
    let now = expected(&docs);
    baseline("files", read_files(&repo, &now));
    pass(
        &mut repo,
        dir.path(),
        "first pass after load",
        "files",
        &|r| read_files(r, &now),
    );
    for i in 0..1_000 {
        let f = rng.next() as usize % FILES;
        let line = rng.next() as usize % docs[f].1.len();
        docs[f].1[line] = if f.is_multiple_of(2) {
            format!(
                "Edited paragraph {line}: value {} after edit {i}.",
                rng.next() % 9973
            )
        } else {
            row(&mut rng, (f * 1_000 + line) as u64 + 7)
        };
        let id = repo
            .put_content(docs[f].1.join("\n").as_bytes())
            .unwrap()
            .id;
        work = repo
            .edit_tree(
                Some(work),
                &[Edit::Put {
                    path: docs[f].0.clone(),
                    id,
                }],
            )
            .unwrap();
        if (i + 1) % 50 == 0 {
            repo.commit("main", work, "edit").unwrap();
        }
    }
    let now = expected(&docs);
    let before = read_files(&repo, &now);
    eprintln!("files: read before the second pass {before:.0} ms");
    pass(
        &mut repo,
        dir.path(),
        "second pass after 1,000 edits",
        "files",
        &|r| read_files(r, &now),
    );
    pass(
        &mut repo,
        dir.path(),
        "pass with nothing new",
        "files",
        &|r| read_files(r, &now),
    );
    compare(&mut repo, dir.path(), "files", cap, false, &|r| {
        read_files(r, &now)
    });
}

fn db_check(repo: &mut R, db: &Db, scan: &[(String, Vec<u8>)]) -> f64 {
    let t = Instant::now();
    assert_eq!(repo.db_scan(db).unwrap(), scan, "scan");
    let m = ms(t);
    assert_eq!(&repo.db_rebuild(&db.log).unwrap(), db, "rebuild");
    m
}

fn db_case(n: u64, cap: Duration) {
    let workload = format!("db {n} rows");
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(FsStore::open_deferred(dir.path()).unwrap());
    repo.plain_db_events = true;
    repo.needed_index = true;
    let mut rng = Lcg(42);
    let mut db = repo.db_empty().unwrap();
    let t = Instant::now();
    let mut start = 0;
    while start < n {
        let end = (start + 10_000).min(n);
        let events: Vec<Event> = (start..end)
            .map(|i| {
                Event::upsert(
                    &format!("t/{i:09}"),
                    row(&mut rng, i).as_bytes(),
                    &format!("load-{i}"),
                )
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
        start = end;
    }
    repo.db_commit("main", &db, "load").unwrap();
    eprintln!("{workload}: loaded in {:.0} ms", ms(t));
    let scan = repo.db_scan(&db).unwrap();
    baseline(&workload, db_check(&mut repo, &db, &scan));
    pass(
        &mut repo,
        dir.path(),
        "first pass after load",
        &workload,
        &|r| db_check(r, &db, &scan),
    );
    let t = Instant::now();
    for b in 0..20 {
        let events: Vec<Event> = (0..100)
            .map(|j| {
                let k = rng.next() % n;
                Event::upsert(
                    &format!("t/{k:09}"),
                    row(&mut rng, k).as_bytes(),
                    &format!("edit-{}", b * 100 + j),
                )
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
        repo.db_commit("main", &db, "append").unwrap();
    }
    eprintln!("{workload}: 2,000 appends in {:.0} ms", ms(t));
    let scan = repo.db_scan(&db).unwrap();
    let before = db_check(&mut repo, &db, &scan);
    eprintln!("{workload}: scan before the second pass {before:.0} ms");
    pass(
        &mut repo,
        dir.path(),
        "second pass after 2,000 appends",
        &workload,
        &|r| db_check(r, &db, &scan),
    );
    // The hot path after a pass: appends still work and stay consistent.
    let t = Instant::now();
    for j in 0..20 {
        let k = rng.next() % n;
        let e = Event::upsert(
            &format!("t/{k:09}"),
            row(&mut rng, k).as_bytes(),
            &format!("after-{j}"),
        );
        db = repo.db_append(&db, &[e]).unwrap();
        repo.db_commit("main", &db, "append").unwrap();
    }
    eprintln!(
        "{workload}: 20 single appends after the pass in {:.0} ms",
        ms(t)
    );
    let scan = repo.db_scan(&db).unwrap();
    pass(
        &mut repo,
        dir.path(),
        "pass after 20 more appends",
        &workload,
        &|r| db_check(r, &db, &scan),
    );
    pass(
        &mut repo,
        dir.path(),
        "pass with nothing new",
        &workload,
        &|r| db_check(r, &db, &scan),
    );
    compare(&mut repo, dir.path(), &workload, cap, true, &|r| {
        db_check(r, &db, &scan)
    });
}

/// Opens a store with the database opt-ins.
fn open_db(dir: &Path) -> R {
    let mut repo = Repo::new(FsStore::open_deferred(dir).unwrap());
    repo.plain_db_events = true;
    repo.needed_index = true;
    repo
}

/// `kill [N] [K]`: K passes killed at random times on copies of one store
/// (N rows, a first pass, then 1,000 appends), each reopened and checked,
/// then passed again to the end and checked.
fn kill_case(n: u64, kills: usize, full: bool) {
    let src = tempfile::tempdir().unwrap();
    let (db, scan) = kill_store(src.path(), n);
    kill_runs(src.path(), n, kills, full, &db, &scan);
}

/// The store the kill and crash tests start from: N rows, a first pass,
/// then 1,000 appends; its database and scan.
fn kill_store(dir: &Path, n: u64) -> (Db, Vec<(String, Vec<u8>)>) {
    let mut repo = open_db(dir);
    let mut rng = Lcg(42);
    let mut db = repo.db_empty().unwrap();
    let events: Vec<Event> = (0..n)
        .map(|i| {
            Event::upsert(
                &format!("t/{i:09}"),
                row(&mut rng, i).as_bytes(),
                &format!("load-{i}"),
            )
        })
        .collect();
    db = repo.db_append(&db, &events).unwrap();
    repo.db_commit("main", &db, "load").unwrap();
    repo.repack_incremental_with(opts()).unwrap();
    for b in 0..10 {
        let events: Vec<Event> = (0..100)
            .map(|j| {
                let k = rng.next() % n;
                Event::upsert(
                    &format!("t/{k:09}"),
                    row(&mut rng, k).as_bytes(),
                    &format!("edit-{}", b * 100 + j),
                )
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
        repo.db_commit("main", &db, "append").unwrap();
    }
    let scan = repo.db_scan(&db).unwrap();
    (db, scan)
}

fn kill_runs(src: &Path, n: u64, kills: usize, full: bool, db: &Db, scan: &[(String, Vec<u8>)]) {
    let mut rng = Lcg(7);
    let exe = std::env::current_exe().unwrap();
    let child = |dir: &Path| {
        let mut c = std::process::Command::new(&exe);
        c.arg(if full { "childfull" } else { "child" }).arg(dir);
        if GROUP.load(Relaxed) {
            c.arg("group");
        }
        c.spawn().unwrap()
    };
    // How long an unkilled pass takes, in a child like the others.
    let probe = tempfile::tempdir().unwrap();
    copy_dir(src, probe.path());
    let t = Instant::now();
    assert!(child(probe.path()).wait().unwrap().success());
    let full_ms = ms(t) as u64;
    // Clean: scans and rebuilds the same, and fsck finds nothing.
    let check = |repo: &mut R| -> bool {
        repo.db_scan(db).ok().as_deref() == Some(scan)
            && repo.db_rebuild(&db.log).ok().as_ref() == Some(db)
            && repo.fsck_deep().map(|b| b.is_empty()).unwrap_or(false)
    };
    let (mut killed, mut finished, mut bad_after_kill, mut bad_after_rerun) = (0, 0, 0, 0);
    for _ in 0..kills {
        let work = tempfile::tempdir().unwrap();
        copy_dir(src, work.path());
        let mut c = child(work.path());
        // A full repack plans in memory and writes only at its end: kill it
        // in the last fifth of its run, where the writes are.
        let at = match full {
            true => full_ms * 4 / 5 + rng.next() % (full_ms / 5).max(1),
            false => rng.next() % full_ms.max(1),
        };
        std::thread::sleep(Duration::from_millis(at));
        match c.try_wait().unwrap() {
            Some(_) => finished += 1,
            None => {
                c.kill().unwrap();
                c.wait().unwrap();
                killed += 1;
            }
        }
        let mut repo = open_db(work.path());
        if !check(&mut repo) {
            bad_after_kill += 1;
            continue;
        }
        if full {
            repo.repack().unwrap();
        } else {
            repo.repack_incremental_with(opts()).unwrap();
        }
        if !check(&mut repo) {
            bad_after_rerun += 1;
        }
    }
    let step = if full {
        "full repacks killed at random"
    } else {
        "passes killed at random"
    };
    println!(
        "{{\"workload\":\"db {n} rows\",\"step\":\"{step}\",\"pass_ms\":{full_ms},\"killed\":{killed},\"finished_first\":{finished},\"bad_after_kill\":{bad_after_kill},\"bad_after_rerun\":{bad_after_rerun}}}"
    );
}

/// One store write, as a full repack issues it.
#[derive(Clone)]
enum Op {
    Write(Id, Vec<u8>),
    Replace(Id, Vec<u8>),
    Delete(Id),
    SetRef(String, Option<Id>, Id),
    DeleteRef(String, Id),
    Sync,
}

/// A store that records every write it passes on (E310).
struct Recorder<S> {
    inner: S,
    ops: std::sync::Mutex<Vec<Op>>,
}

impl<S> Recorder<S> {
    fn push(&self, op: Op) {
        self.ops.lock().unwrap().push(op);
    }
}

impl<S: Store> Store for Recorder<S> {
    fn sync(&mut self) -> Result<(), Error> {
        self.push(Op::Sync);
        self.inner.sync()
    }
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        let new = self.inner.write(id, encoded)?;
        if new {
            self.push(Op::Write(id, encoded.to_vec()));
        }
        Ok(new)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.push(Op::Replace(id, encoded.to_vec()));
        self.inner.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        self.inner.read(id)
    }
    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        self.inner.read_head(id, n)
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
        self.push(Op::Delete(*id));
        self.inner.delete(id)
    }
    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        self.inner.get_ref(name)
    }
    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        self.push(Op::SetRef(name.into(), expected, new));
        self.inner.set_ref(name, expected, new)
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        self.inner.refs()
    }
    fn delete_ref(&mut self, name: &str, expected: Id) -> Result<(), Error> {
        self.push(Op::DeleteRef(name.into(), expected));
        self.inner.delete_ref(name, expected)
    }
}

/// Gives a store the recorded writes, in order.
fn replay<S: Store>(store: &mut S, ops: &[Op]) {
    for op in ops {
        match op {
            Op::Write(id, b) => {
                store.write(*id, b).unwrap();
            }
            Op::Replace(id, b) => store.replace(*id, b).unwrap(),
            Op::Delete(id) => store.delete(id).unwrap(),
            Op::SetRef(name, e, new) => store.set_ref(name, *e, *new).unwrap(),
            Op::DeleteRef(name, e) => store.delete_ref(name, *e).unwrap(),
            Op::Sync => store.sync().unwrap(),
        }
    }
}

/// `crash [N] [K] [synced] [group]` (E310): the full repack of the kill
/// store, its writes recorded once; then, for K points spread over those
/// writes (and the writes at each change of kind, at most 3K in all), a
/// copy of the store given the writes up to that point on a deferred
/// FsStore (with `synced`, one that places each write at once) whose
/// handle is then lost without a drop (a crash: waiting objects never
/// reach their place), reopened and checked.
fn crash_case(n: u64, points: usize) {
    let src = tempfile::tempdir().unwrap();
    let (db, scan) = kill_store(src.path(), n);
    let trace = tempfile::tempdir().unwrap();
    copy_dir(src.path(), trace.path());
    let mut repo = Repo::new(Recorder {
        inner: FsStore::open_deferred(trace.path()).unwrap(),
        ops: Default::default(),
    });
    repo.plain_db_events = true;
    repo.needed_index = true;
    let t = Instant::now();
    repo.repack().unwrap();
    let repack_ms = ms(t);
    let ops: Vec<Op> = std::mem::take(&mut *repo.store().ops.lock().unwrap());
    drop(repo);
    let total = ops.len();
    let count = |f: fn(&Op) -> bool| ops.iter().filter(|o| f(o)).count();
    let (writes, replaces, deletes, refs, syncs) = (
        count(|o| matches!(o, Op::Write(..))),
        count(|o| matches!(o, Op::Replace(..))),
        count(|o| matches!(o, Op::Delete(..))),
        count(|o| matches!(o, Op::SetRef(..) | Op::DeleteRef(..))),
        count(|o| matches!(o, Op::Sync)),
    );
    // Evenly spread points, the end, and the first write of each kind.
    let mut at: std::collections::BTreeSet<usize> =
        (0..=points).map(|i| i * total / points.max(1)).collect();
    for i in 1..total {
        if std::mem::discriminant(&ops[i]) != std::mem::discriminant(&ops[i - 1]) {
            at.extend([i, i + 1]);
        }
        if at.len() > 3 * points {
            break;
        }
    }
    at.retain(|&k| k <= total);
    let check = |repo: &mut R| -> bool {
        repo.db_scan(&db).ok().as_ref() == Some(&scan)
            && repo.db_rebuild(&db.log).ok().as_ref() == Some(&db)
            && repo.fsck_deep().map(|b| b.is_empty()).unwrap_or(false)
    };
    // Control: every write, then the handle dropped as usual.
    {
        let work = tempfile::tempdir().unwrap();
        copy_dir(src.path(), work.path());
        let mut store = FsStore::open_deferred(work.path()).unwrap();
        replay(&mut store, &ops);
        drop(store);
        let mut repo = open_db(work.path());
        assert!(
            check(&mut repo),
            "the replayed repack, dropped as usual, is clean"
        );
    }
    let (mut bad, mut first_bad, mut bad_at) = (0, None, Vec::new());
    for &k in &at {
        let work = tempfile::tempdir().unwrap();
        copy_dir(src.path(), work.path());
        let mut store = match SYNCED.load(Relaxed) {
            true => FsStore::open(work.path()).unwrap(),
            false => FsStore::open_deferred(work.path()).unwrap(),
        };
        replay(&mut store, &ops[..k]);
        // The crash: the handle is lost, so nothing waiting is placed.
        std::mem::forget(store);
        let mut repo = open_db(work.path());
        if !check(&mut repo) {
            bad += 1;
            first_bad.get_or_insert(k);
            bad_at.push(k);
        }
    }
    let kind = |k: usize| match k.checked_sub(1).map(|i| &ops[i]) {
        Some(Op::Write(..)) => "write",
        Some(Op::Replace(..)) => "replace",
        Some(Op::Delete(..)) => "delete",
        Some(Op::SetRef(..) | Op::DeleteRef(..)) => "ref",
        Some(Op::Sync) => "sync",
        None => "none",
    };
    println!(
        "{{\"workload\":\"db {n} rows\",\"step\":\"full repack, crash after each of {} points\",\"replay_store\":\"{}\",\"repack_ms\":{repack_ms:.0},\"ops\":{total},\"writes\":{writes},\"replaces\":{replaces},\"deletes\":{deletes},\"ref_moves\":{refs},\"syncs\":{syncs},\"bad\":{bad},\"first_bad_after\":{},\"first_bad_op\":\"{}\",\"last_bad_after\":{}}}",
        at.len(),
        if SYNCED.load(Relaxed) {
            "synced"
        } else {
            "deferred"
        },
        first_bad.map_or(-1, |k| k as i64),
        first_bad.map_or("none", kind),
        bad_at.last().map_or(-1, |&k| k as i64)
    );
}

/// `gccheck [N]`: E268's database session without the reads in between,
/// then a check before gc, after gc, and after a full repack of a copy.
fn gc_check(n: u64) {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = open_db(dir.path());
    let mut rng = Lcg(42);
    let mut db = repo.db_empty().unwrap();
    let mut start = 0;
    while start < n {
        let end = (start + 10_000).min(n);
        let events: Vec<Event> = (start..end)
            .map(|i| {
                Event::upsert(
                    &format!("t/{i:09}"),
                    row(&mut rng, i).as_bytes(),
                    &format!("load-{i}"),
                )
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
        start = end;
    }
    repo.db_commit("main", &db, "load").unwrap();
    repo.repack_incremental_with(opts()).unwrap();
    for b in 0..20 {
        let events: Vec<Event> = (0..100)
            .map(|j| {
                let k = rng.next() % n;
                Event::upsert(
                    &format!("t/{k:09}"),
                    row(&mut rng, k).as_bytes(),
                    &format!("edit-{}", b * 100 + j),
                )
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
        repo.db_commit("main", &db, "append").unwrap();
    }
    repo.repack_incremental_with(opts()).unwrap();
    let scan = repo.db_scan(&db).unwrap();
    let ok = |repo: &mut R, at: &str| {
        let s = repo.db_scan(&db).map(|s| s == scan);
        let r = repo.db_rebuild(&db.log).map(|d| d == db);
        let f = repo.fsck_deep().map(|f| f.len());
        eprintln!("{at}: scan {s:?} rebuild {r:?} fsck problems {f:?}");
    };
    ok(&mut repo, "before gc");
    repo.gc().unwrap();
    ok(&mut repo, "after gc");
    let t = Instant::now();
    repo.repack().unwrap();
    eprintln!("full repack {:.0} ms", ms(t));
    ok(&mut repo, "after full repack");
    repo.gc().unwrap();
    ok(&mut repo, "after full repack and gc");
}

/// `repro [N] [group]`: E268's database session without the timed reads,
/// then each step of `compare` checked without stopping at the first error.
fn repro(n: u64, passes: bool, save: Option<&Path>) {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = open_db(dir.path());
    let mut rng = Lcg(42);
    let mut db = repo.db_empty().unwrap();
    let mut start = 0;
    while start < n {
        let end = (start + 10_000).min(n);
        let events: Vec<Event> = (start..end)
            .map(|i| {
                Event::upsert(
                    &format!("t/{i:09}"),
                    row(&mut rng, i).as_bytes(),
                    &format!("load-{i}"),
                )
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
        start = end;
    }
    repo.db_commit("main", &db, "load").unwrap();
    let _ = repo.db_scan(&db).unwrap();
    if passes {
        repo.repack_incremental_with(opts()).unwrap();
    }
    for b in 0..20 {
        let events: Vec<Event> = (0..100)
            .map(|j| {
                let k = rng.next() % n;
                Event::upsert(
                    &format!("t/{k:09}"),
                    row(&mut rng, k).as_bytes(),
                    &format!("edit-{}", b * 100 + j),
                )
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
        repo.db_commit("main", &db, "append").unwrap();
    }
    let _ = repo.db_scan(&db).unwrap();
    if passes {
        repo.repack_incremental_with(opts()).unwrap();
    }
    for j in 0..20 {
        let k = rng.next() % n;
        let e = Event::upsert(
            &format!("t/{k:09}"),
            row(&mut rng, k).as_bytes(),
            &format!("after-{j}"),
        );
        db = repo.db_append(&db, &[e]).unwrap();
        repo.db_commit("main", &db, "append").unwrap();
    }
    let scan = repo.db_scan(&db).unwrap();
    if passes {
        repo.repack_incremental_with(opts()).unwrap();
    }
    if passes {
        repo.repack_incremental_with(opts()).unwrap();
    }
    let ok = |repo: &mut R, at: &str| {
        let f = repo.fsck_deep().map(|f| {
            f.iter()
                .take(5)
                .map(|(id, e)| format!("{id:?} {e:?}"))
                .collect::<Vec<_>>()
        });
        let s = repo.db_scan(&db).map(|s| s == scan);
        let r = repo.db_rebuild(&db.log).map(|d| d == db);
        eprintln!("{at}: fsck {f:?} scan {s:?} rebuild {r:?}");
    };
    ok(&mut repo, "after passes");
    repo.gc().unwrap();
    ok(&mut repo, "after gc");
    if let Some(save) = save {
        copy_dir(dir.path(), save);
    }
    let copy = tempfile::tempdir().unwrap();
    copy_dir(dir.path(), copy.path());
    let mut full = open_db(copy.path());
    ok(&mut full, "copy reopened");
    let t = Instant::now();
    full.repack().unwrap();
    eprintln!("full repack {:.0} ms", ms(t));
    ok(&mut full, "copy after full repack");
    full.gc().unwrap();
    ok(&mut full, "copy after full repack and gc");
    drop(full);
    let mut again = open_db(copy.path());
    ok(&mut again, "copy reopened after full repack");
    if let Some(save) = save {
        let before = FsStore::open_deferred(save).unwrap();
        let bad = again.fsck_deep().unwrap();
        eprintln!("{} objects fail after the full repack", bad.len());
        for (id, e) in bad.iter().take(12) {
            let tag = |s: &dyn ikam_kernel::Store| {
                s.read(id).ok().and_then(|b| b.first().map(|t| *t as char))
            };
            eprintln!(
                "{id:?} {e:?} before {:?} after {:?}",
                tag(&before),
                tag(again.store())
            );
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    GROUP.store(args.iter().any(|a| a == "group"), Relaxed);
    SYNCED.store(args.iter().any(|a| a == "synced"), Relaxed);
    if args.first().map(String::as_str) == Some("child") {
        open_db(Path::new(&args[1]))
            .repack_incremental_with(opts())
            .unwrap();
        return;
    }
    if args.first().map(String::as_str) == Some("childfull") {
        open_db(Path::new(&args[1])).repack().unwrap();
        return;
    }
    if args.first().map(String::as_str) == Some("full") {
        let copy = tempfile::tempdir().unwrap();
        copy_dir(Path::new(&args[1]), copy.path());
        let mut full = open_db(copy.path());
        let t = Instant::now();
        full.repack().unwrap();
        eprintln!("full repack {:.0} ms", ms(t));
        let bad = full.fsck_deep().unwrap();
        eprintln!("{} objects fail after the full repack", bad.len());
        return;
    }
    if args.first().map(String::as_str) == Some("repro") {
        let save = args.iter().find_map(|a| a.strip_prefix("save="));
        repro(
            args.get(1).and_then(|a| a.parse().ok()).unwrap_or(20_000),
            !args.iter().any(|a| a == "nopass"),
            save.map(Path::new),
        );
        return;
    }
    if args.first().map(String::as_str) == Some("gccheck") {
        gc_check(args.get(1).and_then(|a| a.parse().ok()).unwrap_or(20_000));
        return;
    }
    if args.first().map(String::as_str) == Some("crash") {
        let n = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(1_000);
        let k = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(100);
        crash_case(n, k);
        return;
    }
    if args.first().map(String::as_str) == Some("kill") {
        let n = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(5_000);
        let k = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(30);
        kill_case(n, k, args.iter().any(|a| a == "full"));
        return;
    }
    let all = args.iter().all(|a| a != "files" && a != "db");
    let cap = args
        .iter()
        .find_map(|a| a.strip_prefix("cap=")?.parse::<u64>().ok())
        .unwrap_or(15);
    let cap = Duration::from_secs(cap * 60);
    FULL.store(!args.iter().any(|a| a == "nofull"), Relaxed);
    let sizes: Vec<u64> = args.iter().filter_map(|a| a.parse().ok()).collect();
    let sizes = if sizes.is_empty() {
        vec![20_000, 100_000]
    } else {
        sizes
    };
    if all || args.iter().any(|a| a == "files") {
        files_case(cap);
    }
    if all || args.iter().any(|a| a == "db") {
        for n in sizes {
            db_case(n, cap);
        }
    }
    // A capped full repack may still be running on its own thread.
    std::process::exit(0);
}
