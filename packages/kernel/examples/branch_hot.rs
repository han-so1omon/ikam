//! E264: hot state per ref. `cargo run --release --example branch_hot -- [N]`
//!
//! A generated table of N rows (default 100,000) on `main`, with E260's
//! per-id sum view (incremental), on an FsStore with deferred sync
//! (E227), `plain_db_events` (E232) and `needed_index` (E129), shared
//! between the writer and the integrators (`SharedStore`). After the load,
//! one cold pass (`repack_incremental`, E268), so later passes see only
//! what changed. Each case starts from a copy of that store. Every ref's
//! hot state is kept by `hot::HotRefs` (src/hot.rs): its newest database
//! and an E262 integrator for its views (`min_events` 50).
//!
//! - `fork`: 20 forks of `main`: time each, and the objects and bytes a
//!   fork writes; against one `views_build`, what a branch that built its
//!   own views would pay first.
//! - `merge lagging` (variant b) and `merge current` (variant a), W3
//!   without conflicts: `side` forked from `main`, then 500 single upserts
//!   on each side in turn (main writes even rows, side odd ones), each
//!   committed to its own ref; hot latency (append, commit, hand-over) and
//!   lag per side. Then `side` merged into `main` at once (b) or after
//!   both sides' views caught up (a), and `main`'s views caught up. The
//!   two cases must give the same merged database and view rows; each is
//!   checked against `views_build` of the merged database. Case b then
//!   runs the cold passes: one after the session, then a second branch
//!   `side2` forked from the merged `main` with 500 upserts of its own and
//!   another pass. Stored bytes after each.
//! - `linear`: the same 1,000 upserts on `main` alone, then a pass: the
//!   bytes a branch costs over writing on one ref.
//! - `conflicts`: W3 with 10% of `side`'s upserts on rows `main` also
//!   writes: the merge, while views lag, must report exactly those rows
//!   and move nothing.
//!
//! Prints one JSON line per step. Fixtures are synthetic.

mod db_common;

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use db_common::{Lcg, row};
use ikam_kernel::hot::HotRefs;
use ikam_kernel::integrate::Policy;
use ikam_kernel::merge::{Conflict, Merged};
use ikam_kernel::views::{Mode, View, ViewGraph, ViewOp};
use ikam_kernel::{Event, FsStore, Repo, SharedStore, Store};

type R = Repo<SharedStore<FsStore>>;

const EDITS: usize = 500;

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

fn graph() -> Arc<ViewGraph> {
    Arc::new(
        ViewGraph::new(vec![View {
            name: "per_id".into(),
            input: "kv".into(),
            mode: Mode::Incremental,
            op: ViewOp::GroupSum(per_id),
            check: None,
        }])
        .unwrap(),
    )
}

fn open(dir: &Path) -> R {
    let mut repo = Repo::new(SharedStore::new(FsStore::open_deferred(dir).unwrap()));
    repo.plain_db_events = true;
    repo.needed_index = true;
    repo
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

fn stored(repo: &R) -> (usize, usize) {
    let s = repo.store();
    let ids = s.ids().unwrap();
    let bytes = ids.iter().map(|id| s.read(id).unwrap().len()).sum();
    (ids.len(), bytes)
}

/// Views fold once they lag by 50 events (E262's `min_events` 50), so
/// they lag at a merge.
fn policy() -> Policy {
    Policy {
        min_events: 50,
        max_lag: None,
    }
}

fn upsert(rng: &mut Lcg, k: u64, tag: &str) -> Event {
    Event::upsert(
        &format!("t/{k:09}"),
        row(rng, k).as_bytes(),
        &format!("{tag}-{k}-{}", rng.next()),
    )
}

/// The loaded store: N rows, committed, then a first cold pass.
fn base(dir: &Path, n: u64) {
    let mut repo = open(dir);
    let mut rng = Lcg(42);
    let mut db = repo.db_empty().unwrap();
    let mut start = 0;
    while start < n {
        let end = (start + 10_000).min(n);
        let evs: Vec<Event> = (start..end)
            .map(|i| {
                Event::upsert(
                    &format!("t/{i:09}"),
                    row(&mut rng, i).as_bytes(),
                    &format!("load-{i}"),
                )
            })
            .collect();
        db = repo.db_append(&db, &evs).unwrap();
        start = end;
    }
    repo.db_commit("main", &db, "load").unwrap();
    let t = Instant::now();
    let pass = repo.repack_incremental().unwrap();
    let (objects, bytes) = stored(&repo);
    println!(
        "{{\"rows\":{n},\"step\":\"load and first pass\",\"pass_ms\":{:.0},\"new\":{},\"objects\":{objects},\"stored\":{bytes}}}",
        ms(t),
        pass.new
    );
}

struct Session {
    hot: [Vec<f64>; 2],
    lag: [Vec<f64>; 2],
}

/// `edits` single upserts on each of `sides` in turn; side 0 writes even
/// rows, side 1 odd ones, except that `planted` of side 1's upserts go to
/// rows side 0 also writes.
fn session(
    repo: &mut R,
    refs: &mut HotRefs,
    n: u64,
    sides: &[&str],
    edits: usize,
    planted: &mut BTreeSet<String>,
    plant: usize,
) -> Session {
    let mut rng = Lcg(7);
    let mut s = Session {
        hot: [vec![], vec![]],
        lag: [vec![], vec![]],
    };
    let mut written: Vec<u64> = Vec::new();
    for i in 0..edits {
        for (side, name) in sides.iter().enumerate() {
            let k = if side == 1 && i % 10 == 0 && planted.len() < plant {
                let k = written[(rng.next() as usize) % written.len()];
                planted.insert(format!("t/{k:09}"));
                k
            } else {
                (rng.next() % (n / 2)) * 2 + side as u64
            };
            if side == 0 {
                written.push(k);
            }
            let e = upsert(&mut rng, k, name);
            let t = Instant::now();
            refs.append(repo, name, &[e], "edit").unwrap();
            s.hot[side].push(ms(t));
            s.lag[side].push(refs.lag(name).unwrap() as f64);
        }
    }
    s
}

fn report(label: &str, sides: &[&str], s: &mut Session) {
    for (i, name) in sides.iter().enumerate() {
        println!(
            "{{\"step\":\"{label}\",\"ref\":\"{name}\",\"edits\":{},\"hot_p50\":{:.2},\"hot_p99\":{:.2},\"hot_max\":{:.1},\"lag_p50\":{:.0},\"lag_p99\":{:.0}}}",
            s.hot[i].len(),
            pct(&mut s.hot[i], 0.5),
            pct(&mut s.hot[i], 0.99),
            pct(&mut s.hot[i], 1.0),
            pct(&mut s.lag[i], 0.5),
            pct(&mut s.lag[i], 0.99),
        );
    }
}

fn check_views(repo: &mut R, g: &ViewGraph, refs: &HotRefs, name: &str) -> Vec<(String, Vec<u8>)> {
    let db = refs.db(name).unwrap();
    let state = refs.wait(name).unwrap();
    let full = repo.views_build(g, &db).unwrap();
    let rows = repo.view_rows(g, &state, "per_id").unwrap();
    assert_eq!(
        rows,
        repo.view_rows(g, &full, "per_id").unwrap(),
        "{name}: views equal a full build"
    );
    rows.into_iter().collect()
}

fn close_all(refs: &mut HotRefs) {
    for name in refs.names() {
        refs.close(&name).unwrap();
    }
}

fn pass(repo: &mut R, label: &str) {
    let t = Instant::now();
    let p = repo.repack_incremental().unwrap();
    let pass_ms = ms(t);
    let (objects, bytes) = stored(repo);
    println!(
        "{{\"step\":\"{label}\",\"pass_ms\":{pass_ms:.0},\"new\":{},\"objects\":{objects},\"stored\":{bytes}}}",
        p.new
    );
}

fn fork_case(src: &Path) {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(src, dir.path());
    let mut repo = open(dir.path());
    let g = graph();
    let mut refs = HotRefs::new(g.clone(), policy());
    let t = Instant::now();
    refs.open(&mut repo, "main", None).unwrap();
    let build_ms = ms(t);
    let before = stored(&repo);
    let mut times = Vec::new();
    for i in 0..20 {
        let t = Instant::now();
        refs.fork(&mut repo, "main", &format!("b{i:02}")).unwrap();
        times.push(ms(t));
    }
    let after = stored(&repo);
    let views = refs.views("main").unwrap();
    for i in 0..20 {
        assert_eq!(refs.views(&format!("b{i:02}")).unwrap(), views);
    }
    close_all(&mut refs);
    println!(
        "{{\"step\":\"fork\",\"forks\":20,\"fork_p50_ms\":{:.3},\"fork_max_ms\":{:.3},\"objects_written\":{},\"bytes_written\":{},\"views_build_ms\":{build_ms:.0}}}",
        pct(&mut times, 0.5),
        pct(&mut times, 1.0),
        after.0 - before.0,
        after.1 - before.1,
    );
}

/// W3 without conflicts and a merge; returns the merged database and
/// view rows.
fn merge_case(src: &Path, n: u64, wait: bool) -> (ikam_kernel::Db, Vec<(String, Vec<u8>)>) {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(src, dir.path());
    let mut repo = open(dir.path());
    let g = graph();
    let mut refs = HotRefs::new(g.clone(), policy());
    refs.open(&mut repo, "main", None).unwrap();
    refs.fork(&mut repo, "main", "side").unwrap();
    let label = if wait {
        "merge current"
    } else {
        "merge lagging"
    };
    let sides = ["main", "side"];
    let mut s = session(
        &mut repo,
        &mut refs,
        n,
        &sides,
        EDITS,
        &mut BTreeSet::new(),
        0,
    );
    let lag_at_merge = refs.lag("main").unwrap() + refs.lag("side").unwrap();
    report(&format!("{label}: session"), &sides, &mut s);
    let t = Instant::now();
    if wait {
        refs.wait("main").unwrap();
        refs.wait("side").unwrap();
    }
    let wait_ms = ms(t);
    let t = Instant::now();
    let m = refs.merge(&mut repo, "main", "side", "merge").unwrap();
    let merge_ms = ms(t);
    assert!(matches!(m, Merged::Commit(_)), "{m:?}");
    let t = Instant::now();
    refs.wait("main").unwrap();
    let catch_up_ms = ms(t);
    let (before_objects, before_bytes) = stored(&repo);
    let rows = check_views(&mut repo, &g, &refs, "main");
    let db = refs.db("main").unwrap();
    println!(
        "{{\"step\":\"{label}\",\"lag_at_merge\":{lag_at_merge},\"wait_before_ms\":{wait_ms:.1},\"merge_ms\":{merge_ms:.1},\"views_after_ms\":{catch_up_ms:.1},\"objects_after\":{before_objects},\"stored_after\":{before_bytes}}}"
    );
    if !wait {
        close_all(&mut refs);
        pass(&mut repo, "pass after the branch session and merge");
        refs.open(&mut repo, "main", None).unwrap();
        refs.fork(&mut repo, "main", "side2").unwrap();
        let mut s = session(
            &mut repo,
            &mut refs,
            n,
            &["side2"],
            EDITS,
            &mut BTreeSet::new(),
            0,
        );
        report("second branch: session", &["side2"], &mut s);
        check_views(&mut repo, &g, &refs, "side2");
        close_all(&mut refs);
        pass(&mut repo, "pass after the second branch");
        assert!(repo.fsck().unwrap().is_empty(), "fsck");
        let main = repo.resolve("main").unwrap();
        let mdb = repo.db_at(&main).unwrap();
        assert_eq!(mdb, db);
        assert_eq!(
            repo.db_rebuild(&db.log).unwrap().kv,
            db.kv,
            "main rebuilds the same"
        );
        let s2 = repo.resolve("side2").unwrap();
        let sdb = repo.db_at(&s2).unwrap();
        assert_eq!(
            repo.db_rebuild(&sdb.log).unwrap().kv,
            sdb.kv,
            "side2 rebuilds the same"
        );
    } else {
        close_all(&mut refs);
    }
    (db, rows)
}

fn linear_case(src: &Path, n: u64) {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(src, dir.path());
    let mut repo = open(dir.path());
    let mut refs = HotRefs::new(graph(), policy());
    refs.open(&mut repo, "main", None).unwrap();
    // The same events as the two-ref session, all on main.
    let mut rng = Lcg(7);
    let mut hot = Vec::new();
    for _ in 0..EDITS {
        for side in 0..2u64 {
            let k = (rng.next() % (n / 2)) * 2 + side;
            let tag = if side == 0 { "main" } else { "side" };
            let e = upsert(&mut rng, k, tag);
            let t = Instant::now();
            refs.append(&mut repo, "main", &[e], "edit").unwrap();
            hot.push(ms(t));
        }
    }
    close_all(&mut refs);
    let (objects, bytes) = stored(&repo);
    println!(
        "{{\"step\":\"linear: session\",\"edits\":{},\"hot_p50\":{:.2},\"hot_p99\":{:.2},\"objects\":{objects},\"stored\":{bytes}}}",
        hot.len(),
        pct(&mut hot, 0.5),
        pct(&mut hot, 0.99)
    );
    pass(&mut repo, "linear: pass");
}

fn conflict_case(src: &Path, n: u64) {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(src, dir.path());
    let mut repo = open(dir.path());
    let mut refs = HotRefs::new(graph(), policy());
    refs.open(&mut repo, "main", None).unwrap();
    refs.fork(&mut repo, "main", "side").unwrap();
    let mut planted = BTreeSet::new();
    let sides = ["main", "side"];
    session(
        &mut repo,
        &mut refs,
        n,
        &sides,
        EDITS,
        &mut planted,
        EDITS / 10,
    );
    let head = repo.resolve("main").unwrap();
    let lag = refs.lag("main").unwrap() + refs.lag("side").unwrap();
    let t = Instant::now();
    let m = refs.merge(&mut repo, "main", "side", "merge").unwrap();
    let merge_ms = ms(t);
    let found: BTreeSet<String> = match m {
        Merged::Conflicts(c) => c
            .into_iter()
            .map(|c| match c {
                Conflict::Key(k) => k,
                other => panic!("{other:?}"),
            })
            .collect(),
        m => panic!("{m:?}"),
    };
    assert_eq!(repo.resolve("main").unwrap(), head, "main did not move");
    close_all(&mut refs);
    println!(
        "{{\"step\":\"conflicts\",\"lag_at_merge\":{lag},\"merge_ms\":{merge_ms:.1},\"planted\":{},\"found\":{},\"missed\":{},\"false\":{}}}",
        planted.len(),
        found.len(),
        planted.difference(&found).count(),
        found.difference(&planted).count()
    );
}

fn main() {
    let n: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(100_000);
    let src = tempfile::tempdir().unwrap();
    base(src.path(), n);
    fork_case(src.path());
    let (db_b, rows_b) = merge_case(src.path(), n, false);
    let (db_a, rows_a) = merge_case(src.path(), n, true);
    println!(
        "{{\"step\":\"variants agree\",\"same_db\":{},\"same_rows\":{}}}",
        db_a == db_b,
        rows_a == rows_b
    );
    assert_eq!(db_a, db_b);
    assert_eq!(rows_a, rows_b);
    linear_case(src.path(), n);
    conflict_case(src.path(), n);
}
