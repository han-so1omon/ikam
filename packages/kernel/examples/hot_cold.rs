//! E260: where time goes in each layer, today; E267: the same with what
//! E261 to E268 kept.
//! `cargo run --release --example hot_cold -- [files|db] [default|opt|kept] [N...]`
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
//! The `kept` setting (E267) adds, to the opt-ins, what E261 to E268
//! kept as options: `Repo::plain_hot_writes` (E261); for the database,
//! views kept by a background integrator (E262, `min_events` 50) through
//! `hot::HotRefs` on `main` (E264), on a `SharedStore`; and, as the cold
//! work, the incremental pass (`repack_incremental`, E268) in place of the
//! full repack, so it runs at every size. For the database, one pass runs
//! after the load, so the pass after the session sees only the session,
//! as it would in steady use. Hot latency (`hot_p50`) is what a caller
//! waits for per batch in every setting: the append, the commit and, with
//! views kept each batch, the view update; with `kept`, the hand-over to
//! the integrator.
//!
//! Prints one JSON line per case. Fixtures are synthetic.

mod db_common;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use db_common::{Lcg, row};
use ikam_kernel::hot::HotRefs;
use ikam_kernel::integrate::Policy;
use ikam_kernel::views::{Mode, View, ViewGraph, ViewOp};
use ikam_kernel::{Db, Edit, Event, FsStore, Id, Repo, SharedStore, Store};

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

/// Which repack the cold work runs.
#[derive(Clone, Copy, PartialEq)]
enum Pass {
    Full,
    Incremental,
    Skip,
}

/// The cold work after a session: repack (full or incremental), gc, fsck.
/// Bytes on disk before and after.
fn cold<S: Store + Sync>(repo: &mut Repo<S>, dir: &Path, pass: Pass) -> String {
    let before = dir_bytes(dir);
    let t = Instant::now();
    let mut new = 0;
    match pass {
        Pass::Full => {
            repo.repack().unwrap();
        }
        Pass::Incremental => new = repo.repack_incremental().unwrap().new,
        Pass::Skip => {}
    }
    let repack_ms = if pass == Pass::Skip { -1.0 } else { ms(t) };
    let name = match pass {
        Pass::Full => "full",
        Pass::Incremental => "incremental",
        Pass::Skip => "none",
    };
    let t = Instant::now();
    let freed = repo.gc().unwrap();
    let gc_ms = ms(t);
    let t = Instant::now();
    assert!(repo.fsck().unwrap().is_empty(), "fsck");
    let fsck_ms = ms(t);
    format!(
        "\"disk_before\":{before},\"disk_after\":{},\"repack\":\"{name}\",\"repack_new\":{new},\"repack_ms\":{repack_ms:.0},\"gc_ms\":{gc_ms:.0},\"gc_freed\":{freed},\"fsck_ms\":{fsck_ms:.0}",
        dir_bytes(dir)
    )
}

/// Today's defaults, today's opt-ins, or those with what E261 to E268
/// kept.
#[derive(Clone, Copy, PartialEq)]
enum Setting {
    Default,
    Opt,
    Kept,
}

impl Setting {
    fn name(self) -> &'static str {
        match self {
            Setting::Default => "default",
            Setting::Opt => "opt",
            Setting::Kept => "kept",
        }
    }
}

fn files_case(setting: Setting, every: usize) {
    let deferred = setting != Setting::Default;
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(open(dir.path(), deferred));
    repo.plain_hot_writes = setting == Setting::Kept;
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
    let pass = match setting {
        Setting::Kept => Pass::Incremental,
        _ => Pass::Full,
    };
    let disk = cold(&mut repo, dir.path(), pass);
    println!(
        "{{\"workload\":\"files\",\"setting\":\"{}\",\"deferred_sync\":{deferred},\"commit_every\":{every},\"edits\":{EDITS},\"session_ms\":{session_ms:.0},\"edit_p50\":{:.2},\"edit_p99\":{:.2},\"put_p50\":{:.2},\"tree_p50\":{:.2},\"commit_p50\":{:.2},\"commit_p99\":{:.2},\"hot_bytes_written\":{hot_written},{disk}}}",
        setting.name(),
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

fn load<S: Store>(repo: &mut Repo<S>, n: u64, rng: &mut Lcg) -> Db {
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
    let (mut append, mut commit, mut view, mut hot) = (vec![], vec![], vec![], vec![]);
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
        let t0 = Instant::now();
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
        hot.push(ms(t0));
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
        let pass = if n <= 20_000 { Pass::Full } else { Pass::Skip };
        format!(",{}", cold(&mut repo, dir.path(), pass))
    } else {
        String::new()
    };
    let batches = append.len();
    println!(
        "{{\"workload\":\"db\",\"rows\":{n},\"setting\":\"{}\",\"opt_ins\":{opt_in},\"batch\":{batch},\"views\":\"{}\",\"batches\":{batches},\"session_ms\":{session_ms:.0},\"hot_p50\":{:.2},\"hot_p99\":{:.2},\"append_p50\":{:.2},\"append_p99\":{:.2},\"commit_p50\":{:.2},\"commit_p99\":{:.2},\"view_p50\":{:.2},\"view_build_ms\":{build_ms:.0},\"view_catch_up_ms\":{catch_up_ms:.1},\"hot_bytes_written\":{hot_written}{disk}}}",
        if opt_in { "opt" } else { "default" },
        if eager_views {
            "each batch"
        } else {
            "once at end"
        },
        pct(&mut hot, 0.5),
        pct(&mut hot, 0.99),
        pct(&mut append, 0.5),
        pct(&mut append, 0.99),
        pct(&mut commit, 0.5),
        pct(&mut commit, 0.99),
        pct(&mut view, 0.5),
    );
}

/// E267: the database with what E261 to E268 kept. Views are kept by a
/// background integrator through `HotRefs`; the cold work is the
/// incremental pass, at every size, after the largest batches.
fn db_kept_case(n: u64, batch: usize) {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(SharedStore::new(
        FsStore::open_deferred(dir.path()).unwrap(),
    ));
    repo.plain_db_events = true;
    repo.needed_index = true;
    repo.plain_hot_writes = true;
    let mut rng = Lcg(42);
    let db = load(&mut repo, n, &mut rng);
    let cold_after = batch == 2_000;
    // A first pass, so the pass after the session sees only the session.
    let load_pass_ms = if cold_after {
        let t = Instant::now();
        repo.repack_incremental().unwrap();
        ms(t)
    } else {
        -1.0
    };
    let g = Arc::new(views());
    let t = Instant::now();
    let state = repo.views_build(&g, &db).unwrap();
    let build_ms = ms(t);
    let mut refs = HotRefs::new(
        g.clone(),
        Policy {
            min_events: 50,
            max_lag: None,
        },
    );
    refs.open(&mut repo, "main", Some(state)).unwrap();
    let written = repo.bytes_written;
    let (mut hot, mut lag) = (vec![], vec![]);
    let session = Instant::now();
    let mut i = 0;
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
        refs.append(&mut repo, "main", &events, "append").unwrap();
        hot.push(ms(t));
        lag.push(refs.lag("main").unwrap() as f64);
    }
    let session_ms = ms(session);
    let hot_written = repo.bytes_written - written;
    let db = refs.db("main").unwrap();
    let t = Instant::now();
    let state = refs.wait("main").unwrap();
    let catch_up_ms = ms(t);
    let full = repo.views_build(&g, &db).unwrap();
    assert_eq!(
        repo.view_rows(&g, &state, "per_id").unwrap(),
        repo.view_rows(&g, &full, "per_id").unwrap(),
        "background views equal a full build"
    );
    assert_eq!(
        repo.db_rebuild(&db.log).unwrap(),
        db,
        "views equal a rebuild"
    );
    // gc and the pass only while no integrator runs.
    refs.close("main").unwrap();
    let disk = if cold_after {
        format!(
            ",\"load_pass_ms\":{load_pass_ms:.0},{}",
            cold(&mut repo, dir.path(), Pass::Incremental)
        )
    } else {
        String::new()
    };
    let batches = hot.len();
    println!(
        "{{\"workload\":\"db\",\"rows\":{n},\"setting\":\"kept\",\"batch\":{batch},\"views\":\"background\",\"batches\":{batches},\"session_ms\":{session_ms:.0},\"hot_p50\":{:.2},\"hot_p99\":{:.2},\"lag_p50\":{:.0},\"lag_p99\":{:.0},\"view_build_ms\":{build_ms:.0},\"view_catch_up_ms\":{catch_up_ms:.1},\"hot_bytes_written\":{hot_written}{disk}}}",
        pct(&mut hot, 0.5),
        pct(&mut hot, 0.99),
        pct(&mut lag, 0.5),
        pct(&mut lag, 0.99),
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
    let named: Vec<Setting> = [Setting::Default, Setting::Opt, Setting::Kept]
        .into_iter()
        .filter(|s| args.iter().any(|a| a == s.name()))
        .collect();
    let settings = if named.is_empty() {
        vec![Setting::Default, Setting::Opt, Setting::Kept]
    } else {
        named
    };
    if all || args.iter().any(|a| a == "files") {
        for &setting in &settings {
            for every in [1, 50] {
                files_case(setting, every);
            }
        }
    }
    if all || args.iter().any(|a| a == "db") {
        for &n in &sizes {
            for &setting in &settings {
                for batch in [1, 100, 2_000] {
                    match setting {
                        Setting::Kept => db_kept_case(n, batch),
                        s => {
                            for eager in [true, false] {
                                db_case(n, s == Setting::Opt, batch, eager);
                            }
                        }
                    }
                }
            }
        }
    }
}
