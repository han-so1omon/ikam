//! Laws of the event log and its views (src/db.rs). The first five port the
//! April fold tests (packages/modelado/tests/test_graph_edge_event_folding.py
//! at b1ce98f^) to keys and events.

use ikam_kernel::{Db, Error, Event, Kind, MemStore, Object, Repo, Store, TreeEntry};

fn up(key: &str, value: &str, idem: &str) -> Event {
    Event::upsert(key, value.as_bytes(), idem)
}

fn rm(key: &str, subtree: bool, idem: &str) -> Event {
    Event::remove(key, subtree, idem)
}

fn fold(events: &[Event]) -> (Repo<MemStore>, Db) {
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let db = repo.db_append(&empty, events).unwrap();
    (repo, db)
}

fn scan(repo: &Repo<MemStore>, db: &Db) -> Vec<(String, String)> {
    repo.db_scan(db)
        .unwrap()
        .into_iter()
        .map(|(k, v)| (k, String::from_utf8(v).unwrap()))
        .collect()
}

fn kv(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// April: last write wins per identity.
#[test]
fn last_write_wins_per_key() {
    let (repo, db) = fold(&[up("a/b", "1", "e1"), up("a/b", "2", "e2")]);
    assert_eq!(scan(&repo, &db), kv(&[("a/b", "2")]));
    assert_eq!(
        repo.db_events(&db).unwrap().len(),
        2,
        "both writes are logged"
    );
}

/// April: a delete removes the key even though its idempotency key differs
/// from the upsert's.
#[test]
fn remove_wins_whatever_its_idempotency_key() {
    let (repo, db) = fold(&[up("d9", "x", "idem-upsert"), rm("d9", false, "idem-delete")]);
    assert!(scan(&repo, &db).is_empty());
}

/// April: keys are independent.
#[test]
fn keys_are_independent() {
    let (repo, db) = fold(&[
        up("a/b", "1", "e1"),
        up("a/c", "2", "e2"),
        rm("a/b", false, "e3"),
    ]);
    assert_eq!(scan(&repo, &db), kv(&[("a/c", "2")]));
}

/// April: a subtree remove drops the key and every key below it, and
/// nothing else (`a/b2` is not below `a/b`).
#[test]
fn subtree_remove_drops_descendants() {
    let events = [
        up("claims/0", "claim", "e1"),
        up("claims/0/evidence", "evidence", "e2"),
        up("claims/0/evidence/1", "more", "e3"),
        up("claims/00", "sibling", "e4"),
        rm("claims/0", true, "e5"),
    ];
    let (repo, db) = fold(&events);
    assert_eq!(scan(&repo, &db), kv(&[("claims/00", "sibling")]));
    let (repo, db) = fold(&[
        up("a/b", "1", "e1"),
        up("a/b/c", "2", "e2"),
        rm("a/b", false, "e3"),
    ]);
    assert_eq!(
        scan(&repo, &db),
        kv(&[("a/b/c", "2")]),
        "a plain remove keeps descendants"
    );
}

/// April: identity includes environment qualifiers; here they are part of
/// the key, so the same path in two environments is two keys.
#[test]
fn qualifiers_in_the_key_keep_environments_apart() {
    let (repo, db) = fold(&[
        up("env:dev/rel-1", "dev", "e1"),
        up("env:staging/rel-1", "staging", "e2"),
        rm("env:dev/rel-1", false, "e3"),
    ]);
    assert_eq!(scan(&repo, &db), kv(&[("env:staging/rel-1", "staging")]));
}

/// April's order-independent receipt, as a law of the views: a batch of
/// writes to distinct keys gives the same kv view in any order.
#[test]
fn writes_to_distinct_keys_commute() {
    let batch: Vec<Event> = (0..200)
        .map(|i| up(&format!("k/{i:03}"), &format!("v{i}"), &format!("e{i}")))
        .collect();
    let (_, a) = fold(&batch);
    let mut reversed = batch.clone();
    reversed.reverse();
    let (_, b) = fold(&reversed);
    assert_eq!(a.kv, b.kv);
    assert_ne!(a.log, b.log, "the log still records the order");
}

/// Every value reads back byte for byte, including empty and non-UTF-8
/// values, and every event reads back as written.
#[test]
fn values_and_events_read_back_exactly() {
    let values: Vec<Vec<u8>> = vec![
        Vec::new(),
        vec![0, 255, 0, 10, 13],
        (0..=255u8).cycle().take(70_000).collect(),
        "naïve café\r\n".as_bytes().to_vec(),
    ];
    let events: Vec<Event> = values
        .iter()
        .enumerate()
        .map(|(i, v)| Event::upsert(&format!("v/{i}"), v, &format!("e{i}")))
        .collect();
    let (repo, db) = fold(&events);
    for (i, v) in values.iter().enumerate() {
        assert_eq!(
            repo.db_get(&db, &format!("v/{i}")).unwrap().as_ref(),
            Some(v)
        );
    }
    assert_eq!(repo.db_get(&db, "v/9").unwrap(), None);
    let logged: Vec<Event> = repo
        .db_events(&db)
        .unwrap()
        .into_iter()
        .map(|(_, e)| e)
        .collect();
    assert_eq!(logged, events);
    for e in &events {
        assert_eq!(Event::decode(&e.encode()).unwrap().0, *e);
    }
}

/// Decoding is strict: trailing bytes, a bad op, an empty key segment or a
/// missing idempotency key are errors.
#[test]
fn events_decode_strictly() {
    let mut bytes = rm("a", true, "e").encode();
    bytes.push(0);
    assert!(Event::decode(&bytes).is_err());
    let mut bytes = rm("a", true, "e").encode();
    *bytes.last_mut().unwrap() = 2;
    assert!(Event::decode(&bytes).is_err());
    assert!(Event::decode(&up("a//b", "", "e").encode()).is_err());
    assert!(Event::decode(&up("a", "", "").encode()).is_err());
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    assert!(repo.db_append(&empty, &[up("a/", "x", "e")]).is_err());
}

/// A retried append changes no root; reusing an idempotency key for a
/// different event is refused and appends nothing.
#[test]
fn retried_appends_change_nothing() {
    let first = [up("a", "1", "e1"), up("b", "2", "e2")];
    let (mut repo, db) = fold(&first);
    assert_eq!(repo.db_append(&db, &first).unwrap(), db);
    assert_eq!(repo.db_append(&db, &[first[1].clone()]).unwrap(), db);
    let again = repo
        .db_append(&db, &[up("a", "1", "e1"), up("a", "3", "e3")])
        .unwrap();
    assert_eq!(repo.db_events(&again).unwrap().len(), 3);
    match repo.db_append(&again, &[up("c", "4", "e4"), up("a", "changed", "e1")]) {
        Err(Error::IdempotencyConflict(k)) => assert_eq!(k, "e1"),
        other => panic!("expected a conflict, got {other:?}"),
    }
    let dup = repo
        .db_append(&db, &[up("c", "4", "e9"), up("c", "4", "e9")])
        .unwrap();
    assert_eq!(
        repo.db_events(&dup).unwrap().len(),
        3,
        "a retry within one batch is skipped"
    );
}

/// Views are projections: folding the log from scratch gives the same
/// roots as views maintained append by append, at every step.
#[test]
fn rebuilt_views_equal_maintained_views() {
    let mut repo = Repo::new(MemStore::default());
    let mut db = repo.db_empty().unwrap();
    let mut s = 0x2545f4914f6cdd1du64;
    for i in 0..300 {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        let key = format!("p/{}/{}", s % 7, (s >> 8) % 11);
        let e = match s % 5 {
            0 => rm(&key, false, &format!("e{i}")),
            1 => rm(&format!("p/{}", s % 7), true, &format!("e{i}")),
            _ => up(&key, &format!("{s:x}"), &format!("e{i}")),
        };
        db = repo.db_append(&db, &[e]).unwrap();
        if i % 50 == 49 {
            assert_eq!(repo.db_rebuild(&db.log).unwrap(), db);
        }
    }
    assert_eq!(repo.db_rebuild(&db.log).unwrap(), db);
}

/// Commits keep every version: reads at an older commit see that version,
/// files beside `db` are kept, and gc, repack and fsck leave both intact.
#[test]
fn older_commits_read_their_own_version() {
    let mut repo = Repo::new(MemStore::default());
    let file = repo.put_content(b"a file beside the database").unwrap().id;
    let entry = TreeEntry {
        name: "notes.txt".into(),
        kind: Kind::File,
        id: file,
    };
    let tree = repo.put(&Object::tree(vec![entry]).unwrap()).unwrap();
    repo.commit("main", tree, "files").unwrap();

    let empty = repo.db_empty().unwrap();
    let v1 = repo
        .db_append(&empty, &[up("run/1", "first report", "r1")])
        .unwrap();
    let c1 = repo.db_commit("main", &v1, "run 1").unwrap();
    let v2 = repo
        .db_append(
            &v1,
            &[
                up("run/1", "corrected report", "r1b"),
                up("run/2", "second", "r2"),
            ],
        )
        .unwrap();
    let c2 = repo.db_commit("main", &v2, "run 2").unwrap();
    let v3 = repo.db_append(&v2, &[rm("run", true, "clear")]).unwrap();
    repo.db_commit("main", &v3, "clear").unwrap();

    repo.gc().unwrap();
    repo.repack().unwrap();
    assert!(repo.fsck().unwrap().is_empty());

    let at1 = repo.db_at(&c1).unwrap();
    assert_eq!(at1, v1);
    assert_eq!(
        repo.db_get(&at1, "run/1").unwrap().unwrap(),
        b"first report"
    );
    let at2 = repo.db_at(&c2).unwrap();
    assert_eq!(
        scan(&repo, &at2),
        kv(&[("run/1", "corrected report"), ("run/2", "second")])
    );
    let head = repo.resolve("main").unwrap();
    let now = repo.db_at(&head).unwrap();
    assert!(scan(&repo, &now).is_empty());
    assert_eq!(repo.db_events(&now).unwrap().len(), 4);
    assert_eq!(
        repo.read_content(&file).unwrap(),
        b"a file beside the database"
    );
    assert_eq!(repo.db_rebuild(&now.log).unwrap(), now);
}

/// A commit with no `db` entry reads as an empty database.
#[test]
fn commits_without_a_database_read_as_empty() {
    let mut repo = Repo::new(MemStore::default());
    let tree = repo.put(&Object::tree(Vec::new()).unwrap()).unwrap();
    let c = repo.commit("main", tree, "").unwrap();
    let db = repo.db_at(&c).unwrap();
    assert_eq!(db, repo.db_empty().unwrap());
    assert!(repo.db_events(&db).unwrap().is_empty());
    assert!(repo.store().get_ref("main").unwrap().is_some());
}

/// Append cost, printed (run with `--ignored --nocapture`): every append
/// rebuilds the three graphs, so cost grows with the log.
#[test]
#[ignore]
fn append_cost() {
    use std::time::Instant;
    for &(n, batch) in &[(1000usize, 1usize), (1000, 100), (10_000, 1000)] {
        let mut repo = Repo::new(MemStore::default());
        let mut db = repo.db_empty().unwrap();
        let start = Instant::now();
        let mut last = 0.0;
        for b in 0..n / batch {
            let t = Instant::now();
            let events: Vec<Event> = (0..batch)
                .map(|j| {
                    let i = b * batch + j;
                    up(
                        &format!("runs/{:05}", i % 500),
                        &format!("report {i} {}", "x".repeat(200)),
                        &format!("e{i}"),
                    )
                })
                .collect();
            db = repo.db_append(&db, &events).unwrap();
            last = t.elapsed().as_secs_f64();
        }
        let ids = repo.store().ids().unwrap();
        let bytes: usize = ids
            .iter()
            .map(|id| repo.store().read(id).unwrap().len())
            .sum();
        println!(
            "{n} events in batches of {batch}: {:.2} s total, last batch {:.1} ms, {} objects, {} KB stored",
            start.elapsed().as_secs_f64(),
            last * 1000.0,
            ids.len(),
            bytes / 1024
        );
    }
}
