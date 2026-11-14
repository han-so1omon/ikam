//! E260: where time goes in each layer, today.
//! `cargo run --release --example hot_cold -- [files|db] [default|opt] [N...]`
//!
//! The hot layer is what a caller waits for: storing an edit, editing the
//! tree or appending to the log, and the commit that moves the ref. The
//! cold layer is the work that may lag: repack, garbage collection, fsck
//! and bringing derived views up to date. This harness runs both with
//! today's defaults and with today's opt-ins for the hot path
//! (`FsStore::open_deferred`, E227; `Repo::plain_db_events`, E232;
//! `Repo::needed_index`, E129, for the database), and
//! times the cold work after the session, so the two layers are measured
//! together for the first time. No kernel change.
//!
//! - `files` (W1): 200 generated files (half markdown, half CSV), then
//!   1,000 edits, each a changed line, committed after every edit or
//!   every 50.
//! - `db` (W2): a generated table of N rows (default 100,000), then 2,000
//!   appends in batches of 100 and 2,000 (200 single appends), committed
//!   after each batch;
//!   a view graph (E242) kept current after each batch or caught up once
//!   at the end. gc and fsck run once per store setting, after the
//!   2,000-event batches; repack too, at 20,000 rows or fewer (on a
//!   100,000-row store, 445 MB on disk, it ran 12 minutes without
//!   finishing).
//!
//! Prints one JSON line per case. Fixtures are synthetic.

mod db_common;

use std::path::Path;
use std::time::Instant;

use db_common::{Lcg, row};
use ikam_kernel::views::{Mode, View, ViewGraph, ViewOp};
use ikam_kernel::{Db, Edit, Event, FsStore, Id, Repo};

const FILES: usize = 200;
const EDITS: usize = 1_000;
const APPENDS: usize = 2_000;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
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

fn open(dir: &Path, deferred: bool) -> FsStore {
    match deferred {
        true => FsStore::open_deferred(dir).unwrap(),
        false => FsStore::open(dir).unwrap(),
    }
}

/// A generated file: markdown paragraphs or CSV rows.
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

/// The cold work after a session: repack, gc, fsck. Bytes on disk before
/// and after.
fn cold(repo: &mut Repo<FsStore>, dir: &Path, repack: bool) -> String {
    let before = dir_bytes(dir);
    let t = Instant::now();
    if repack {
        repo.repack().unwrap();
    }
    let repack_ms = if repack { ms(t) } else { -1.0 };
    let t = Instant::now();
    let freed = repo.gc().unwrap();
    let gc_ms = ms(t);
    let t = Instant::now();
    assert!(repo.fsck().unwrap().is_empty(), "fsck");
    let fsck_ms = ms(t);
    format!(
        "\"disk_before\":{before},\"disk_after\":{},\"repack_ms\":{repack_ms:.0},\"gc_ms\":{gc_ms:.0},\"gc_freed\":{freed},\"fsck_ms\":{fsck_ms:.0}",
        dir_bytes(dir)
    )
}

fn files_case(deferred: bool, every: usize) {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(open(dir.path(), deferred));
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
    let written = repo.bytes_written;
    let (mut put, mut tree, mut commit, mut edit) = (vec![], vec![], vec![], vec![]);
    let session = Instant::now();
    for i in 0..EDITS {
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
        let id: Id = repo
            .put_content(docs[f].1.join("\n").as_bytes())
            .unwrap()
            .id;
        put.push(ms(t));
        let t2 = Instant::now();
        work = repo
            .edit_tree(
                Some(work),
                &[Edit::Put {
                    path: docs[f].0.clone(),
                    id,
                }],
            )
            .unwrap();
        tree.push(ms(t2));
        if (i + 1).is_multiple_of(every) {
            let t3 = Instant::now();
            repo.commit("main", work, "edit").unwrap();
            commit.push(ms(t3));
        }
        edit.push(ms(t));
    }
    let session_ms = ms(session);
    let hot_written = repo.bytes_written - written;
    let disk = cold(&mut repo, dir.path(), true);
    println!(
        "{{\"workload\":\"files\",\"deferred_sync\":{deferred},\"commit_every\":{every},\"edits\":{EDITS},\"session_ms\":{session_ms:.0},\"edit_p50\":{:.2},\"edit_p99\":{:.2},\"put_p50\":{:.2},\"tree_p50\":{:.2},\"commit_p50\":{:.2},\"commit_p99\":{:.2},\"hot_bytes_written\":{hot_written},{disk}}}",
        pct(&mut edit, 0.5),
        pct(&mut edit, 0.99),
        pct(&mut put, 0.5),
        pct(&mut tree, 0.5),
        pct(&mut commit, 0.5),
        pct(&mut commit, 0.99),
    );
}

fn field(v: &[u8], j: usize) -> &str {
    std::str::from_utf8(v)
        .unwrap()
        .split(',')
        .nth(j)
        .unwrap_or("")
}

fn per_id(_: &str, v: &[u8]) -> Option<(String, i64)> {
    Some((field(v, 1).to_string(), field(v, 3).parse().ok()?))
}

fn views() -> ViewGraph {
    ViewGraph::new(vec![View {
        name: "per_id".into(),
        input: "kv".into(),
        mode: Mode::Incremental,
        op: ViewOp::GroupSum(per_id),
        check: None,
    }])
    .unwrap()
}

fn load(repo: &mut Repo<FsStore>, n: u64, rng: &mut Lcg) -> Db {
    let mut db = repo.db_empty().unwrap();
    let mut start = 0;
    while start < n {
        let end = (start + 10_000).min(n);
        let events: Vec<Event> = (start..end)
            .map(|i| {
                Event::upsert(
                    &format!("t/{i:09}"),
                    row(rng, i).as_bytes(),
                    &format!("load-{i}"),
                )
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
        start = end;
    }
    repo.db_commit("main", &db, "load").unwrap();
    db
}

fn db_case(n: u64, opt_in: bool, batch: usize, eager_views: bool) {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(open(dir.path(), opt_in));
    repo.plain_db_events = opt_in;
    repo.needed_index = opt_in;
    let mut rng = Lcg(42);
    let mut db = load(&mut repo, n, &mut rng);
    let g = views();
    let t = Instant::now();
    let mut state = repo.views_build(&g, &db).unwrap();
    let build_ms = ms(t);
    let written = repo.bytes_written;
    let (mut append, mut commit, mut view) = (vec![], vec![], vec![]);
    let session = Instant::now();
    let mut i = 0;
    // Single appends stop at 200: each waits on a commit.
    let total = if batch == 1 { 200 } else { APPENDS };
    while i < total {
        let events: Vec<Event> = (i..(i + batch).min(total))
            .map(|j| {
                let k = rng.next() % n;
                Event::upsert(
                    &format!("t/{k:09}"),
                    row(&mut rng, k).as_bytes(),
                    &format!("edit-{j}"),
                )
            })
            .collect();
        i += events.len();
        let t = Instant::now();
        db = repo.db_append(&db, &events).unwrap();
        append.push(ms(t));
        let t = Instant::now();
        repo.db_commit("main", &db, "append").unwrap();
        commit.push(ms(t));
        if eager_views {
            let t = Instant::now();
            state = repo.views_update(&g, &state, &db).unwrap();
            view.push(ms(t));
        }
    }
    let session_ms = ms(session);
    let hot_written = repo.bytes_written - written;
    // The cold layer: views caught up once (lazy) and checked against a
    // full build, then repack, gc and fsck.
    let t = Instant::now();
    state = repo.views_update(&g, &state, &db).unwrap();
    let catch_up_ms = ms(t);
    let full = repo.views_build(&g, &db).unwrap();
    assert_eq!(
        repo.view_rows(&g, &state, "per_id").unwrap(),
        repo.view_rows(&g, &full, "per_id").unwrap(),
        "incremental view equals a full build"
    );
    assert_eq!(
        repo.db_rebuild(&db.log).unwrap(),
        db,
        "views equal a rebuild"
    );
    // The cold work runs once per store setting, after the largest
    // batches with views caught up at the end; repack only on small
    // stores (repack_ms -1 when skipped).
    let disk = if batch == 2_000 && !eager_views {
        format!(",{}", cold(&mut repo, dir.path(), n <= 20_000))
    } else {
        String::new()
    };
    let batches = append.len();
    println!(
        "{{\"workload\":\"db\",\"rows\":{n},\"opt_ins\":{opt_in},\"batch\":{batch},\"views\":\"{}\",\"batches\":{batches},\"session_ms\":{session_ms:.0},\"append_p50\":{:.2},\"append_p99\":{:.2},\"commit_p50\":{:.2},\"commit_p99\":{:.2},\"view_p50\":{:.2},\"view_build_ms\":{build_ms:.0},\"view_catch_up_ms\":{catch_up_ms:.1},\"hot_bytes_written\":{hot_written}{disk}}}",
        if eager_views {
            "each batch"
        } else {
            "once at end"
        },
        pct(&mut append, 0.5),
        pct(&mut append, 0.99),
        pct(&mut commit, 0.5),
        pct(&mut commit, 0.99),
        pct(&mut view, 0.5),
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let all = args.iter().all(|a| a != "files" && a != "db");
    let sizes: Vec<u64> = args.iter().filter_map(|a| a.parse().ok()).collect();
    let sizes = if sizes.is_empty() {
        vec![100_000]
    } else {
        sizes
    };
    if all || args.iter().any(|a| a == "files") {
        for deferred in [false, true] {
            for every in [1, 50] {
                files_case(deferred, every);
            }
        }
    }
    if all || args.iter().any(|a| a == "db") {
        for &n in &sizes {
            let only = |w: &str| args.iter().any(|a| a == w);
            let settings: Vec<bool> = [false, true]
                .into_iter()
                .filter(|&o| !(only("opt") && !o) && !(only("default") && o))
                .collect();
            for &opt_in in &settings {
                for batch in [1, 100, 2_000] {
                    for eager in [true, false] {
                        db_case(n, opt_in, batch, eager);
                    }
                }
            }
        }
    }
}
