//! Laws of the background integrator (E262): every state it writes equals
//! a full build at the same database; `wait` sees its own write; the lag
//! cap holds; siblings share the needed index, so a writer with
//! `needed_index` never drops what the worker's encodings need.

use std::sync::Arc;

use ikam_kernel::integrate::{Integrator, Policy};
use ikam_kernel::views::{Mode, View, ViewGraph, ViewOp};
use ikam_kernel::{Db, Event, FsStore, MemStore, Repo, SharedStore, Store};

fn field(v: &[u8], j: usize) -> &str {
    std::str::from_utf8(v)
        .unwrap()
        .split(',')
        .nth(j)
        .unwrap_or("")
}

fn per_group(_: &str, v: &[u8]) -> Option<(String, i64)> {
    Some((field(v, 0).to_string(), field(v, 1).parse().ok()?))
}

fn upper(k: &str, v: &[u8]) -> Option<(String, Vec<u8>)> {
    Some((k.to_string(), v.to_ascii_uppercase()))
}

fn graph() -> ViewGraph {
    ViewGraph::new(vec![
        View {
            name: "sums".into(),
            input: "kv".into(),
            mode: Mode::Incremental,
            op: ViewOp::GroupSum(per_group),
            check: None,
        },
        View {
            name: "loud".into(),
            input: "kv".into(),
            mode: Mode::Stored,
            op: ViewOp::Map(upper),
            check: None,
        },
    ])
    .unwrap()
}

fn event(i: u64, step: u64) -> Event {
    Event::upsert(
        &format!("t/{:04}", i % 300),
        format!("g{},{}", i % 7, i * 3 + step).as_bytes(),
        &format!("e-{step}-{i}"),
    )
}

fn load<S: Store>(repo: &mut Repo<S>) -> Db {
    let events: Vec<Event> = (0..500).map(|i| event(i, 0)).collect();
    let empty = repo.db_empty().unwrap();
    repo.db_append(&empty, &events).unwrap()
}

fn same<S: Store>(repo: &mut Repo<S>, g: &ViewGraph, state: &ikam_kernel::views::ViewState) {
    let full = repo.views_build(g, &state.db).unwrap();
    for v in ["sums", "loud"] {
        assert_eq!(
            repo.view_rows(g, state, v).unwrap(),
            repo.view_rows(g, &full, v).unwrap(),
            "{v} equals a full build at the same position"
        );
    }
}

#[test]
fn every_state_equals_a_full_build_and_wait_sees_its_own_write() {
    let mut repo = Repo::new(SharedStore::new(MemStore::default()));
    let g = Arc::new(graph());
    let mut db = load(&mut repo);
    let state = repo.views_build(&g, &db).unwrap();
    let it = Integrator::start(&repo, g.clone(), state, Policy::default());
    let mut seen = Vec::new();
    for step in 1..=40 {
        let events: Vec<Event> = (0..5).map(|i| event(step * 11 + i, step)).collect();
        db = repo.db_append(&db, &events).unwrap();
        it.advance(&db).unwrap();
        if step % 8 == 0 {
            let s = it.wait(&db).unwrap();
            assert_eq!(s.db, db, "wait returns the views at its own write");
            seen.push(s);
        } else {
            seen.push(it.current());
        }
    }
    let last = it.stop().unwrap();
    assert_eq!(last.db, db);
    for s in seen.iter().chain([&last]) {
        same(&mut repo, &g, s);
    }
    assert!(repo.fsck().unwrap().is_empty());
}

#[test]
fn min_events_batches_passes_and_stop_catches_up() {
    let mut repo = Repo::new(SharedStore::new(MemStore::default()));
    let g = Arc::new(graph());
    let mut db = load(&mut repo);
    let state = repo.views_build(&g, &db).unwrap();
    let policy = Policy {
        min_events: 1_000_000,
        max_lag: None,
    };
    let it = Integrator::start(&repo, g.clone(), state.clone(), policy);
    for step in 1..=10 {
        db = repo.db_append(&db, &[event(step, step)]).unwrap();
        it.advance(&db).unwrap();
    }
    assert_eq!(it.passes(), 0, "below min_events nothing folds");
    assert_eq!(it.current(), state);
    assert_eq!(it.lag(), 10);
    let last = it.stop().unwrap();
    assert_eq!(last.db, db, "stop catches up");
    same(&mut repo, &g, &last);
}

#[test]
fn the_lag_cap_holds_after_every_advance() {
    let mut repo = Repo::new(SharedStore::new(MemStore::default()));
    let g = Arc::new(graph());
    let mut db = load(&mut repo);
    let state = repo.views_build(&g, &db).unwrap();
    let policy = Policy {
        min_events: 1,
        max_lag: Some(6),
    };
    let it = Integrator::start(&repo, g.clone(), state, policy);
    for step in 1..=30 {
        let events: Vec<Event> = (0..3).map(|i| event(step * 5 + i, step)).collect();
        db = repo.db_append(&db, &events).unwrap();
        it.advance(&db).unwrap();
        assert!(it.lag() <= 6, "lag {} after advance", it.lag());
    }
    same(&mut repo, &g, &it.stop().unwrap());
}

#[test]
fn a_worker_on_an_fs_store_with_the_opt_ins_stays_consistent() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(SharedStore::new(
        FsStore::open_deferred(dir.path()).unwrap(),
    ));
    repo.plain_db_events = true;
    repo.needed_index = true;
    let g = Arc::new(graph());
    let mut db = load(&mut repo);
    let state = repo.views_build(&g, &db).unwrap();
    let it = Integrator::start(&repo, g.clone(), state, Policy::default());
    for step in 1..=60 {
        let events: Vec<Event> = (0..4).map(|i| event(step * 7 + i, step)).collect();
        db = repo.db_append(&db, &events).unwrap();
        repo.db_commit("main", &db, "append").unwrap();
        it.advance(&db).unwrap();
    }
    let last = it.stop().unwrap();
    same(&mut repo, &g, &last);
    assert_eq!(repo.db_rebuild(&db.log).unwrap(), db);
    assert!(repo.fsck_deep().unwrap().is_empty());
}

#[test]
fn siblings_share_the_needed_index() {
    let mut a = Repo::new(SharedStore::new(MemStore::default()));
    a.needed_index = true;
    let b = a.sibling();
    assert!(b.needed_index, "a sibling keeps the option");
    let db = load(&mut a);
    // A write through either handle is seen by the other's index.
    let mut b = b;
    let more = b.db_append(&db, &[event(1, 1)]).unwrap();
    assert!(a.db_append(&more, &[event(2, 2)]).is_ok());
    assert!(a.fsck_deep().unwrap().is_empty());
}
