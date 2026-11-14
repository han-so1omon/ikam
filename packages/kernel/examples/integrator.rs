//! E262: views kept current by a background integrator.
//! `cargo run --release --example integrator -- [N]`
//!
//! E260 found that keeping views current after every append is most of a
//! session of single appends (37 s against 4.0 s caught up once, at
//! 100,000 rows). Here the same view (E260's per-id sum, incremental) is
//! kept current three ways while appends arrive:
//!
//! - `inline`: `views_update` after every append, on the writer's thread.
//! - `end`: once, after the session.
//! - `background`: an `Integrator` (src/integrate.rs) on a sibling handle,
//!   folding whenever the views lag by `min_events`, with and without a
//!   lag cap (`max_lag`).
//!
//! Workload: a generated table of N rows (default 100,000) on an FsStore
//! with deferred sync (E227), `plain_db_events` (E232) and `needed_index`
//! (E129), shared between the writer and the worker (`SharedStore`).
//! Sessions: 200 single appends, and 2,000 events in batches of 100; a
//! commit after each append, as in E260. Each case starts from a copy of
//! the same loaded store, after one warm-up append.
//!
//! A hot operation is what the writer waits for: append, commit, and the
//! view update (`inline`) or `advance` (`background`). `final` is what a
//! reader that must see the last write then waits: the catch-up for
//! `end`, `wait` for `background`. Lag is sampled after each hot
//! operation. Every case's last state is checked against `views_build`.
//!
//! Prints one JSON line per case. Fixtures are synthetic.

mod db_common;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use db_common::{Lcg, row};
use ikam_kernel::integrate::{Integrator, Policy};
use ikam_kernel::views::{Mode, View, ViewGraph, ViewOp, ViewState};
use ikam_kernel::{Db, Event, FsStore, Repo, SharedStore};

type R = Repo<SharedStore<FsStore>>;

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

fn events(rng: &mut Lcg, n: u64, from: usize, count: usize) -> Vec<Event> {
    (from..from + count)
        .map(|j| {
            let k = rng.next() % n;
            Event::upsert(
                &format!("t/{k:09}"),
                row(rng, k).as_bytes(),
                &format!("edit-{j}"),
            )
        })
        .collect()
}

/// The loaded store: N rows, committed, views built.
fn base(dir: &Path, n: u64) -> (Db, ViewState) {
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
    let state = repo.views_build(&views(), &db).unwrap();
    // A ref move makes every waiting object durable and moves it to its
    // own path (E227), so the copies below hold the views' chunks too.
    repo.db_commit("main", &db, "views").unwrap();
    (db, state)
}

#[derive(Clone, Copy)]
enum How {
    Inline,
    End,
    Background(Policy),
}

fn case(src: &Path, n: u64, db0: Db, s0: &ViewState, batch: usize, how: How) {
    let dir = tempfile::tempdir().unwrap();
    copy_dir(src, dir.path());
    let mut repo = open(dir.path());
    let g = Arc::new(views());
    let mut rng = Lcg(7);
    // Warm-up: the writer's first append builds its indexes.
    let mut db = repo.db_append(&db0, &events(&mut rng, n, 0, 1)).unwrap();
    repo.db_commit("main", &db, "warm").unwrap();
    let mut state = repo.views_update(&g, s0, &db).unwrap();
    let total = if batch == 1 { 200 } else { 2_000 };
    let it = match how {
        How::Background(p) => Some(Integrator::start(&repo, g.clone(), state.clone(), p)),
        _ => None,
    };
    let (mut hot, mut lag) = (vec![], vec![]);
    let session = Instant::now();
    let mut i = 1;
    while i <= total {
        let evs = events(&mut rng, n, i, batch.min(total + 1 - i));
        i += evs.len();
        let t = Instant::now();
        db = repo.db_append(&db, &evs).unwrap();
        repo.db_commit("main", &db, "append").unwrap();
        match how {
            How::Inline => state = repo.views_update(&g, &state, &db).unwrap(),
            How::End => {}
            How::Background(_) => it.as_ref().unwrap().advance(&db).unwrap(),
        }
        hot.push(ms(t));
        lag.push(match (&it, how) {
            (Some(it), _) => it.lag() as f64,
            (None, How::Inline) => 0.0,
            _ => (db.kv_at - state.db.kv_at) as f64,
        });
    }
    let session_ms = ms(session);
    let t = Instant::now();
    let (passes, label) = match (it, how) {
        (Some(it), How::Background(p)) => {
            state = it.wait(&db).unwrap();
            let passes = it.passes();
            it.stop().unwrap();
            (
                passes,
                format!(
                    "background min_events={} max_lag={}",
                    p.min_events,
                    p.max_lag.map_or("none".into(), |m| m.to_string())
                ),
            )
        }
        (_, How::End) => {
            state = repo.views_update(&g, &state, &db).unwrap();
            (1, "end".into())
        }
        _ => (hot.len() as u64, "inline".into()),
    };
    let final_ms = ms(t);
    let full = repo.views_build(&g, &db).unwrap();
    assert_eq!(state.db, db);
    assert_eq!(
        repo.view_rows(&g, &state, "per_id").unwrap(),
        repo.view_rows(&g, &full, "per_id").unwrap(),
        "views equal a full build"
    );
    println!(
        "{{\"rows\":{n},\"batch\":{batch},\"events\":{total},\"views\":\"{label}\",\"session_ms\":{session_ms:.0},\"hot_p50\":{:.2},\"hot_p99\":{:.2},\"final_ms\":{final_ms:.1},\"lag_p50\":{:.0},\"lag_p99\":{:.0},\"lag_max\":{:.0},\"passes\":{passes}}}",
        pct(&mut hot, 0.5),
        pct(&mut hot, 0.99),
        pct(&mut lag, 0.5),
        pct(&mut lag, 0.99),
        pct(&mut lag, 1.0),
    );
}

fn main() {
    let n: u64 = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(100_000);
    let src = tempfile::tempdir().unwrap();
    let t = Instant::now();
    let (db, state) = base(src.path(), n);
    eprintln!("loaded {n} rows in {:.0} ms", ms(t));
    let bg = |min_events, max_lag| {
        How::Background(Policy {
            min_events,
            max_lag,
        })
    };
    for (batch, cases) in [
        (
            1,
            vec![
                How::Inline,
                How::End,
                bg(1, None),
                bg(50, None),
                bg(1, Some(20)),
            ],
        ),
        (
            100,
            vec![How::Inline, How::End, bg(1, None), bg(1, Some(200))],
        ),
    ] {
        for how in cases {
            case(src.path(), n, db, &state, batch, how);
        }
    }
}
