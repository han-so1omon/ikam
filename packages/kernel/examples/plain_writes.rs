//! E261: plain hot writes, with the cold pass doing the planning later.
//! `cargo run --release --example plain_writes -- [files] [db] [N...] [full]`
//!
//! Every new content today goes through the storage-plan search (slices,
//! templates, containers) before the write returns. `Repo::plain_hot_writes`
//! skips that search: a write stores the content compressed, as it is, and
//! the cold pass (`repack_incremental`, E268) recompresses it later. This
//! harness runs the same session twice, planned at write time (today) and
//! with plain hot writes, then one cold pass on each, and compares the
//! latency of each write, what is stored after the session and after the
//! pass, and read time.
//!
//! - `files` (W1, as E260 and E268): 200 generated files (half markdown,
//!   half CSV), then 1,000 one-line edits, committed every 50. A write is
//!   storing the edited file and editing the tree.
//! - `db` (W2, as E260, with its opt-ins: deferred sync, `plain_db_events`,
//!   `needed_index`): a generated table of N rows (default 20,000), then
//!   2,000 upserts in batches of 100 and 200 single upserts, each
//!   committed. `plain_db_events` already stores events plainly; what is
//!   left to skip is the search for the database's own chunks.
//!
//! Every store is read back and checked after the session and after the
//! pass (every file, or a scan plus `db_rebuild`), and `fsck_deep` runs
//! after the pass. With `full`, a full repack and gc follow the pass, for
//! comparison (the only pass that finds slices and templates); with the
//! environment variable `TAGS` set, stored bytes by encoding tag are
//! printed before and after it. FsStore with deferred sync (E227) in a temporary
//! directory. Prints one JSON line per step. Fixtures are synthetic.

mod db_common;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::Instant;

use db_common::{Lcg, row};
use ikam_kernel::{Db, Edit, Event, FsStore, Id, Object, Repo};

type R = Repo<FsStore>;

/// `full`: a full repack and gc after the cold pass, for comparison.
static FULL: AtomicBool = AtomicBool::new(false);

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * p).round() as usize]
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

/// Bytes of every stored object (what the store holds, without the
/// filesystem's per-file overhead).
fn stored_bytes(repo: &R) -> usize {
    use ikam_kernel::Store;
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).map(|b| b.len()).unwrap_or(0))
        .sum()
}

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

fn label(plain: bool) -> &'static str {
    if plain {
        "plain hot writes"
    } else {
        "planned at write"
    }
}

/// One cold pass, then the store read back and checked.
fn after_pass(
    repo: &mut R,
    dir: &Path,
    workload: &str,
    plain: bool,
    check: &dyn Fn(&mut R) -> f64,
) {
    let t = Instant::now();
    let p = repo.repack_incremental().unwrap();
    let pass_ms = ms(t);
    let read_ms = check(repo);
    assert!(repo.fsck_deep().unwrap().is_empty(), "fsck_deep");
    println!(
        "{{\"workload\":\"{workload}\",\"mode\":\"{}\",\"step\":\"after one cold pass\",\"pass_ms\":{pass_ms:.0},\"new\":{},\"stored\":{},\"disk\":{},\"read_ms\":{read_ms:.0}}}",
        label(plain),
        p.new,
        stored_bytes(repo),
        dir_bytes(dir)
    );
    if FULL.load(Relaxed) {
        tags(repo, "before full repack");
        let t = Instant::now();
        repo.repack().unwrap();
        repo.gc().unwrap();
        let repack_ms = ms(t);
        tags(repo, "after full repack");
        let read_ms = check(repo);
        assert!(repo.fsck_deep().unwrap().is_empty(), "fsck_deep");
        println!(
            "{{\"workload\":\"{workload}\",\"mode\":\"{}\",\"step\":\"after a full repack and gc\",\"repack_ms\":{repack_ms:.0},\"stored\":{},\"disk\":{},\"read_ms\":{read_ms:.0}}}",
            label(plain),
            stored_bytes(repo),
            dir_bytes(dir)
        );
    }
}

/// With TAGS set: stored bytes by encoding tag (the first byte).
fn tags(repo: &R, at: &str) {
    use ikam_kernel::Store;
    if std::env::var_os("TAGS").is_none() {
        return;
    }
    let s = repo.store();
    let mut by: BTreeMap<u8, (usize, usize)> = BTreeMap::new();
    for id in s.ids().unwrap() {
        let b = s.read(&id).unwrap();
        let e = by.entry(*b.first().unwrap_or(&0)).or_default();
        e.0 += 1;
        e.1 += b.len();
    }
    let parts: Vec<String> = by
        .iter()
        .map(|(t, (n, b))| format!("{}:{n}/{b}", *t as char))
        .collect();
    eprintln!("tags {at}: {}", parts.join(" "));
}

fn files_case(plain: bool) {
    const FILES: usize = 200;
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(FsStore::open_deferred(dir.path()).unwrap());
    repo.plain_hot_writes = plain;
    let mut rng = Lcg(7);
    let mut docs: Vec<(String, Vec<String>)> = (0..FILES).map(|i| file(&mut rng, i)).collect();
    let t = Instant::now();
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
    let load_ms = ms(t);
    let mut lat = Vec::new();
    let mut commits = Vec::new();
    let session = Instant::now();
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
        let t = Instant::now();
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
        lat.push(ms(t));
        if (i + 1) % 50 == 0 {
            let t = Instant::now();
            repo.commit("main", work, "edit").unwrap();
            commits.push(ms(t));
        }
    }
    let session_ms = ms(session);
    let now: BTreeMap<String, Vec<u8>> = docs
        .iter()
        .map(|(p, l)| (p.clone(), l.join("\n").into_bytes()))
        .collect();
    let read_ms = read_files(&repo, &now);
    println!(
        "{{\"workload\":\"files\",\"mode\":\"{}\",\"step\":\"session of 1,000 edits\",\"load_ms\":{load_ms:.0},\"session_ms\":{session_ms:.0},\"edit_p50_ms\":{:.2},\"edit_p95_ms\":{:.2},\"edit_max_ms\":{:.1},\"commit_p50_ms\":{:.2},\"stored\":{},\"disk\":{},\"read_ms\":{read_ms:.0}}}",
        label(plain),
        pct(&mut lat, 0.5),
        pct(&mut lat, 0.95),
        pct(&mut lat, 1.0),
        pct(&mut commits, 0.5),
        stored_bytes(&repo),
        dir_bytes(dir.path())
    );
    after_pass(&mut repo, dir.path(), "files", plain, &|r| {
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

fn db_case(n: u64, plain: bool) {
    let workload = format!("db {n} rows");
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(FsStore::open_deferred(dir.path()).unwrap());
    repo.plain_db_events = true;
    repo.needed_index = true;
    repo.plain_hot_writes = plain;
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
    let load_ms = ms(t);
    let mut batches = Vec::new();
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
        let t = Instant::now();
        db = repo.db_append(&db, &events).unwrap();
        repo.db_commit("main", &db, "append").unwrap();
        batches.push(ms(t));
    }
    let mut singles = Vec::new();
    for j in 0..200 {
        let k = rng.next() % n;
        let e = Event::upsert(
            &format!("t/{k:09}"),
            row(&mut rng, k).as_bytes(),
            &format!("single-{j}"),
        );
        let t = Instant::now();
        db = repo.db_append(&db, &[e]).unwrap();
        repo.db_commit("main", &db, "append").unwrap();
        singles.push(ms(t));
    }
    let scan = repo.db_scan(&db).unwrap();
    let read_ms = db_check(&mut repo, &db, &scan);
    println!(
        "{{\"workload\":\"{workload}\",\"mode\":\"{}\",\"step\":\"session of 2,000 batched and 200 single upserts\",\"load_ms\":{load_ms:.0},\"batch_p50_ms\":{:.1},\"batch_p95_ms\":{:.1},\"single_p50_ms\":{:.2},\"single_p95_ms\":{:.2},\"single_max_ms\":{:.1},\"stored\":{},\"disk\":{},\"read_ms\":{read_ms:.0}}}",
        label(plain),
        pct(&mut batches, 0.5),
        pct(&mut batches, 0.95),
        pct(&mut singles, 0.5),
        pct(&mut singles, 0.95),
        pct(&mut singles, 1.0),
        stored_bytes(&repo),
        dir_bytes(dir.path())
    );
    after_pass(&mut repo, dir.path(), &workload, plain, &|r| {
        db_check(r, &db, &scan)
    });
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let all = args.iter().all(|a| a != "files" && a != "db");
    FULL.store(args.iter().any(|a| a == "full"), Relaxed);
    let sizes: Vec<u64> = args.iter().filter_map(|a| a.parse().ok()).collect();
    let sizes = if sizes.is_empty() {
        vec![20_000]
    } else {
        sizes
    };
    for plain in [false, true] {
        if all || args.iter().any(|a| a == "files") {
            files_case(plain);
        }
        if all || args.iter().any(|a| a == "db") {
            for &n in &sizes {
                db_case(n, plain);
            }
        }
    }
}
