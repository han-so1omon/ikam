//! Laws of hot state per ref (E264): a fork shares the source's state and
//! writes no objects; each ref's views equal a full build at its own
//! database; a merge while views lag (variant b) gives the database and
//! the view rows a merge with current views (variant a) gives; conflicts
//! are reported the same way and change nothing; one cold pass after
//! branching keeps every ref readable.

use std::sync::Arc;

use ikam_kernel::hot::HotRefs;
use ikam_kernel::integrate::Policy;
use ikam_kernel::merge::{Conflict, Merged};
use ikam_kernel::views::{Mode, View, ViewGraph, ViewOp};
use ikam_kernel::{Db, Event, MemStore, Repo, SharedStore, Store};

type R = Repo<SharedStore<MemStore>>;

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

fn graph() -> Arc<ViewGraph> {
    Arc::new(
        ViewGraph::new(vec![View {
            name: "sums".into(),
            input: "kv".into(),
            mode: Mode::Incremental,
            op: ViewOp::GroupSum(per_group),
            check: None,
        }])
        .unwrap(),
    )
}

fn event(key: u64, step: u64, side: &str) -> Event {
    Event::upsert(
        &format!("t/{key:04}"),
        format!("g{},{}", key % 7, key * 3 + step).as_bytes(),
        &format!("{side}-{step}-{key}"),
    )
}

fn repo() -> R {
    let mut repo = Repo::new(SharedStore::new(MemStore::default()));
    repo.needed_index = true;
    let events: Vec<Event> = (0..400).map(|i| event(i, 0, "load")).collect();
    let empty = repo.db_empty().unwrap();
    let db = repo.db_append(&empty, &events).unwrap();
    repo.db_commit("main", &db, "load").unwrap();
    repo
}

/// Lagging views: fold only every 50 events.
fn policy() -> Policy {
    Policy {
        min_events: 50,
        max_lag: None,
    }
}

fn rows(repo: &mut R, g: &ViewGraph, refs: &HotRefs, name: &str) {
    let db: Db = refs.db(name).unwrap();
    let state = refs.wait(name).unwrap();
    let full = repo.views_build(g, &db).unwrap();
    assert_eq!(
        repo.view_rows(g, &state, "sums").unwrap(),
        repo.view_rows(g, &full, "sums").unwrap(),
        "{name}: views equal a full build"
    );
}

/// Each side writes its own keys: main 0..200, side 200..400.
fn session(repo: &mut R, refs: &mut HotRefs, shared: u64) {
    for step in 1..=60 {
        let m = event(step % 200, step, "main");
        refs.append(repo, "main", &[m], "main edit").unwrap();
        let key = if step <= shared {
            step % 200
        } else {
            200 + step % 200
        };
        let s = event(key, step + 1000, "side");
        refs.append(repo, "side", &[s], "side edit").unwrap();
    }
}

#[test]
fn fork_shares_state_and_writes_nothing() {
    let g = graph();
    let mut repo = repo();
    let mut refs = HotRefs::new(g.clone(), policy());
    refs.open(&mut repo, "main", None).unwrap();
    let before = repo.store().ids().unwrap().len();
    refs.fork(&mut repo, "main", "side").unwrap();
    assert_eq!(repo.store().ids().unwrap().len(), before, "a fork is a ref");
    assert_eq!(refs.db("side").unwrap(), refs.db("main").unwrap());
    assert_eq!(repo.resolve("side").unwrap(), repo.resolve("main").unwrap());
    assert_eq!(refs.views("side").unwrap(), refs.views("main").unwrap());
    assert!(refs.fork(&mut repo, "main", "side").is_err());
    for name in refs.names() {
        refs.close(&name).unwrap();
    }
}

#[test]
fn each_ref_keeps_its_own_views() {
    let g = graph();
    let mut repo = repo();
    let mut refs = HotRefs::new(g.clone(), policy());
    refs.open(&mut repo, "main", None).unwrap();
    refs.fork(&mut repo, "main", "side").unwrap();
    session(&mut repo, &mut refs, 0);
    assert_ne!(refs.db("main").unwrap(), refs.db("side").unwrap());
    rows(&mut repo, &g, &refs, "main");
    rows(&mut repo, &g, &refs, "side");
    // The committed database at each ref is the ref's hot database.
    for name in ["main", "side"] {
        let head = repo.resolve(name).unwrap();
        assert_eq!(repo.db_at(&head).unwrap(), refs.db(name).unwrap());
    }
    for name in refs.names() {
        refs.close(&name).unwrap();
    }
}

/// The merged database, its view rows, and the lag at the merge.
type Outcome = (Db, Vec<(String, Vec<u8>)>, Option<u64>);

/// Run the session and merge side into main, waiting for views first
/// (variant a) or not (variant b).
fn merged(wait: bool) -> Outcome {
    let g = graph();
    let mut repo = repo();
    let mut refs = HotRefs::new(g.clone(), policy());
    refs.open(&mut repo, "main", None).unwrap();
    refs.fork(&mut repo, "main", "side").unwrap();
    session(&mut repo, &mut refs, 0);
    let lag = if wait {
        refs.wait("main").unwrap();
        refs.wait("side").unwrap();
        None
    } else {
        Some(refs.lag("main").unwrap() + refs.lag("side").unwrap())
    };
    let m = refs.merge(&mut repo, "main", "side", "merge").unwrap();
    assert!(matches!(m, Merged::Commit(_)), "{m:?}");
    rows(&mut repo, &g, &refs, "main");
    let db = refs.db("main").unwrap();
    let state = refs.wait("main").unwrap();
    let out = repo.view_rows(&g, &state, "sums").unwrap();
    for name in refs.names() {
        refs.close(&name).unwrap();
    }
    (db, out.into_iter().collect(), lag)
}

#[test]
fn merge_while_views_lag_equals_merge_with_current_views() {
    let (db_a, rows_a, _) = merged(true);
    let (db_b, rows_b, lag) = merged(false);
    assert!(lag.unwrap() > 0, "views lagged at the merge");
    assert_eq!(db_a, db_b, "same merged database");
    assert_eq!(rows_a, rows_b, "same view rows");
}

#[test]
fn conflicts_reported_while_views_lag_and_nothing_moves() {
    let g = graph();
    let mut repo = repo();
    let mut refs = HotRefs::new(g.clone(), policy());
    refs.open(&mut repo, "main", None).unwrap();
    refs.fork(&mut repo, "main", "side").unwrap();
    session(&mut repo, &mut refs, 6);
    let (head, db) = (repo.resolve("main").unwrap(), refs.db("main").unwrap());
    match refs.merge(&mut repo, "main", "side", "merge").unwrap() {
        Merged::Conflicts(c) => {
            let keys: Vec<Conflict> = (1..=6)
                .map(|k| Conflict::Key(format!("t/{k:04}")))
                .collect();
            assert_eq!(c, keys);
        }
        m => panic!("{m:?}"),
    }
    assert_eq!(repo.resolve("main").unwrap(), head);
    assert_eq!(refs.db("main").unwrap(), db);
    for name in refs.names() {
        refs.close(&name).unwrap();
    }
}

#[test]
fn fast_forward_hands_over_the_database() {
    let g = graph();
    let mut repo = repo();
    let mut refs = HotRefs::new(g.clone(), policy());
    refs.open(&mut repo, "main", None).unwrap();
    refs.fork(&mut repo, "main", "side").unwrap();
    for step in 1..=30 {
        refs.append(&mut repo, "side", &[event(step, step, "side")], "e")
            .unwrap();
    }
    let m = refs.merge(&mut repo, "main", "side", "merge").unwrap();
    assert!(matches!(m, Merged::FastForward(_)), "{m:?}");
    assert_eq!(refs.db("main").unwrap(), refs.db("side").unwrap());
    rows(&mut repo, &g, &refs, "main");
    for name in refs.names() {
        refs.close(&name).unwrap();
    }
}

#[test]
fn one_cold_pass_after_branching_keeps_every_ref() {
    let g = graph();
    let mut repo = repo();
    let mut refs = HotRefs::new(g.clone(), policy());
    refs.open(&mut repo, "main", None).unwrap();
    refs.fork(&mut repo, "main", "side").unwrap();
    session(&mut repo, &mut refs, 0);
    let dbs: Vec<(String, Db)> = refs
        .names()
        .into_iter()
        .map(|n| (n.clone(), refs.db(&n).unwrap()))
        .collect();
    let scans: Vec<_> = dbs.iter().map(|(_, d)| repo.db_scan(d).unwrap()).collect();
    for name in refs.names() {
        refs.close(&name).unwrap();
    }
    repo.repack_incremental().unwrap();
    repo.gc().unwrap();
    assert!(repo.fsck().unwrap().is_empty());
    for ((name, db), scan) in dbs.iter().zip(&scans) {
        assert_eq!(&repo.db_scan(db).unwrap(), scan, "{name}");
        assert_eq!(repo.db_rebuild(&db.log).unwrap().kv, db.kv, "{name}");
    }
}
