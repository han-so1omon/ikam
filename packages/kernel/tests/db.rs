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

/// The same law on a database spread over many chunks: random batches of
/// upserts, removes and subtree removes (some spanning chunk boundaries)
/// edited in place give, byte for byte, the roots a full rebuild gives.
#[test]
fn in_place_views_equal_a_full_rebuild_at_scale() {
    let mut repo = Repo::new(MemStore::default());
    let mut db = repo.db_empty().unwrap();
    let mut s = 0x853c49e6748fea9bu64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut i = 0;
    for round in 0..40 {
        let batch: Vec<Event> = (0..1 + next() % 300)
            .map(|_| {
                i += 1;
                let r = next();
                let key = format!("chain:{}/run:{}/{}", r % 20, (r >> 8) % 50, (r >> 16) % 5);
                let idem = format!("e{i}");
                match r % 20 {
                    0 => rm(&format!("chain:{}", r % 20), true, &idem),
                    1 | 2 => rm(
                        &format!("chain:{}/run:{}", r % 20, (r >> 8) % 50),
                        true,
                        &idem,
                    ),
                    3..=5 => rm(&key, false, &idem),
                    _ => up(&key, &"x".repeat((r >> 24) as usize % 400), &idem),
                }
            })
            .collect();
        db = repo.db_append(&db, &batch).unwrap();
        assert_eq!(db.kv_at, i as u64);
        if round % 5 == 4 {
            assert_eq!(repo.db_rebuild(&db.log).unwrap(), db, "round {round}");
        }
    }
    assert_eq!(repo.db_rebuild(&db.log).unwrap(), db);
    assert!(
        repo.db_scan(&db).unwrap().len() > 100,
        "the view spans many chunks"
    );
}

/// Each view records its log position; a commit keeps it, a commit from
/// before positions were recorded reads as the log's end, and views behind
/// the log are refused rather than appended to.
#[test]
fn views_record_their_log_position() {
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    assert_eq!((empty.ids_at, empty.kv_at), (0, 0));
    let db = repo
        .db_append(&empty, &[up("a", "1", "e1"), up("b", "2", "e2")])
        .unwrap();
    assert_eq!(
        (db.ids_at, db.kv_at, repo.db_head(&db.log).unwrap()),
        (2, 2, 2)
    );
    let c = repo.db_commit("main", &db, "two").unwrap();
    assert_eq!(repo.db_at(&c).unwrap(), db);

    let graph = |name: &str, id| TreeEntry {
        name: name.into(),
        kind: Kind::Graph,
        id,
    };
    let views = repo
        .put(&Object::tree(vec![graph("ids", db.ids), graph("kv", db.kv)]).unwrap())
        .unwrap();
    let views = TreeEntry {
        name: "views".into(),
        kind: Kind::Tree,
        id: views,
    };
    let dir = repo
        .put(&Object::tree(vec![graph("log", db.log), views]).unwrap())
        .unwrap();
    let dir = TreeEntry {
        name: "db".into(),
        kind: Kind::Tree,
        id: dir,
    };
    let tree = repo.put(&Object::tree(vec![dir]).unwrap()).unwrap();
    let old = repo.commit("old", tree, "no positions").unwrap();
    assert_eq!(repo.db_at(&old).unwrap(), db);

    let behind = Db { kv_at: 1, ..db };
    assert!(repo.db_append(&behind, &[up("c", "3", "e3")]).is_err());
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
    for &(n, batch) in &[
        (1000usize, 1usize),
        (1000, 100),
        (10_000, 1000),
        (10_000, 1),
    ] {
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

/// Two strings whose 8-byte label keys are the same (tests/graphs.rs).
const CLASH: [&str; 2] = ["ke32051949435c933", "kb4a1e63c25ad68d4"];

/// Keys and idempotency keys are chosen by clients, so two of them may
/// share a label key (a client can find such a pair in minutes). Both are
/// stored and read back exactly, and the views still equal a full rebuild.
#[test]
fn clashing_label_keys_are_stored_and_rebuilt() {
    let mut repo = Repo::new(MemStore::default());
    let mut db = repo.db_empty().unwrap();
    for i in 0..400 {
        let key = format!("k{:016x}", (i as u64) * (u64::MAX / 400) + 1);
        db = repo.db_append(&db, &[up(&key, "filler", &key)]).unwrap();
    }
    // As keys, and as idempotency keys, in separate appends and together.
    db = repo
        .db_append(&db, &[up(CLASH[0], "first", CLASH[0])])
        .unwrap();
    db = repo
        .db_append(&db, &[up(CLASH[1], "second", CLASH[1])])
        .unwrap();
    assert_eq!(repo.db_get(&db, CLASH[0]).unwrap(), Some(b"first".to_vec()));
    assert_eq!(
        repo.db_get(&db, CLASH[1]).unwrap(),
        Some(b"second".to_vec())
    );
    assert_eq!(repo.db_rebuild(&db.log).unwrap(), db);
    // A retry under either idempotency key is still a no-op.
    assert_eq!(
        repo.db_append(&db, &[up(CLASH[1], "second", CLASH[1])])
            .unwrap(),
        db
    );
    db = repo
        .db_append(
            &db,
            &[rm(CLASH[0], false, "r0"), up(CLASH[1], "third", "u1")],
        )
        .unwrap();
    assert_eq!(repo.db_get(&db, CLASH[0]).unwrap(), None);
    assert_eq!(repo.db_get(&db, CLASH[1]).unwrap(), Some(b"third".to_vec()));
    assert_eq!(repo.db_rebuild(&db.log).unwrap(), db);
}

/// E229: a range or prefix read returns exactly the keys of a full scan that
/// fall in it, in order or reversed, cut at the limit, over a view that
/// spans many chunks and levels.
#[test]
fn ranges_and_prefixes_equal_a_filtered_scan() {
    let mut repo = Repo::new(MemStore::default());
    let mut s = 0x2545f4914f6cdd1du64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let events: Vec<Event> = (0..6000)
        .map(|i| {
            let r = next();
            let key = format!(
                "t/items/k{:03}/2{:03}-{:02}",
                r % 40,
                (r >> 8) % 30,
                (r >> 16) % 12
            );
            up(&key, &"v".repeat((r >> 24) as usize % 60), &format!("e{i}"))
        })
        .collect();
    let empty = repo.db_empty().unwrap();
    let db = repo.db_append(&empty, &events).unwrap();
    let all = scan(&repo, &db);
    assert!(all.len() > 3000);
    let text = |v: Vec<(String, Vec<u8>)>| -> Vec<(String, String)> {
        v.into_iter()
            .map(|(k, v)| (k, String::from_utf8(v).unwrap()))
            .collect()
    };
    let mut cases: Vec<(String, Option<String>)> = vec![
        (String::new(), None),
        ("t/items/k007/".into(), Some("t/items/k008/".into())),
        (
            "t/items/k007/2010".into(),
            Some("t/items/k031/2005-03".into()),
        ),
        ("t/items/k039/2029-11".into(), None),
        ("u".into(), None),
        ("t/items/k010/".into(), Some("t/items/k010/".into())),
    ];
    for _ in 0..40 {
        let (a, b) = (
            &all[next() as usize % all.len()].0,
            &all[next() as usize % all.len()].0,
        );
        cases.push((a.min(b).clone(), Some(a.max(b).clone())));
    }
    for (start, end) in &cases {
        let want: Vec<_> = all
            .iter()
            .filter(|(k, _)| k >= start && end.as_ref().is_none_or(|e| k < e))
            .cloned()
            .collect();
        for limit in [0, 1, 7, 500, usize::MAX] {
            let got = text(
                repo.db_range(&db, start, end.as_deref(), limit, false)
                    .unwrap(),
            );
            let cut: Vec<_> = want.iter().take(limit).cloned().collect();
            assert_eq!(got, cut, "{start:?}..{end:?} limit {limit}");
            let got = text(
                repo.db_range(&db, start, end.as_deref(), limit, true)
                    .unwrap(),
            );
            let cut: Vec<_> = want.iter().rev().take(limit).cloned().collect();
            assert_eq!(got, cut, "reverse {start:?}..{end:?} limit {limit}");
        }
    }
    for p in ["t/items/k012/", "t/items/k012/2003", "t/items/", "t/", "x/"] {
        let want: Vec<_> = all
            .iter()
            .filter(|(k, _)| k.starts_with(p))
            .cloned()
            .collect();
        assert_eq!(
            text(repo.db_prefix(&db, p, usize::MAX, false).unwrap()),
            want,
            "{p}"
        );
        let last: Vec<_> = want.iter().rev().take(1).cloned().collect();
        assert_eq!(
            text(repo.db_prefix(&db, p, 1, true).unwrap()),
            last,
            "last under {p}"
        );
    }
}

#[test]
fn prefix_end_is_the_first_string_after_the_prefix() {
    use ikam_kernel::prefix_end;
    assert_eq!(prefix_end("a/b/").as_deref(), Some("a/b0"));
    assert_eq!(prefix_end("az").as_deref(), Some("a{"));
    assert_eq!(prefix_end(""), None);
    assert_eq!(prefix_end("\u{10FFFF}"), None);
    assert_eq!(prefix_end("a\u{10FFFF}").as_deref(), Some("b"));
    assert_eq!(prefix_end("\u{D7FF}").as_deref(), Some("\u{E000}"));
}

#[test]
fn counts_equal_the_length_of_the_range() {
    let events: Vec<Event> = (0..2000)
        .map(|i| up(&format!("t/x/{:02}/{i:05}", i % 37), "v", &format!("e{i}")))
        .collect();
    let (repo, db) = fold(&events);
    for (s, e) in [
        ("t/x/03/", Some("t/x/04/")),
        ("", None),
        ("t/x/36/", None),
        ("u", None),
    ] {
        assert_eq!(
            repo.db_count(&db, s, e).unwrap(),
            repo.db_range(&db, s, e, usize::MAX, false).unwrap().len()
        );
    }
}

/// E232: a reading handle on a shared store reads what the writer derived
/// after the reader started, because siblings share the ledger.
#[test]
fn sibling_reads_what_the_writer_derives_later() {
    use ikam_kernel::SharedStore;
    let mut w = Repo::new(SharedStore::new(MemStore::default()));
    let row = |i: usize| {
        Event::upsert(
            &format!("t/rows/{:06}", i),
            format!("[{i},\"some repeated text to slice against\",{}]", i * 7).as_bytes(),
            &format!("i{i}"),
        )
    };
    let mut db = w.db_empty().unwrap();
    for b in 0..4 {
        let batch: Vec<Event> = (b * 1500..(b + 1) * 1500).map(row).collect();
        db = w.db_append(&db, &batch).unwrap();
    }
    let c = w.db_commit("db/t", &db, "first").unwrap();
    let mut r = w.sibling();
    let at_c = r.db_at(&c).unwrap();
    assert_eq!(r.db_scan(&at_c).unwrap().len(), 6000);
    // More versions of the same chunks, written after the reader read.
    for i in (0..6000).step_by(97) {
        db = w
            .db_append(
                &db,
                &[Event::upsert(
                    &format!("t/rows/{:06}", i),
                    b"[0]",
                    &format!("u{i}"),
                )],
            )
            .unwrap();
    }
    let c2 = w.db_commit("db/t", &db, "second").unwrap();
    let mine = w.db_scan(&db).unwrap();
    let at_c2 = r.db_at(&c2).unwrap();
    let theirs = r.db_scan(&at_c2).unwrap();
    assert_eq!(mine, theirs);
}

/// E233: counts kept by appends equal counts from scratch after every kind
/// of write (new keys, overwrites, removes, subtree removes, retries), each
/// prefix's kept count equals the count of its range, a commit keeps the
/// view, and a database that keeps none commits as before.
#[test]
fn kept_counts_equal_counts_from_scratch() {
    let mut repo = Repo::new(MemStore::default());
    let plain = repo.db_empty().unwrap();
    let mut db = repo.db_keep_counts(&plain, 2).unwrap();
    let mut s = 0x9e3779b97f4a7c15u64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut i = 0;
    let mut batches = Vec::new();
    for round in 0..30 {
        let batch: Vec<Event> = (0..1 + next() % 200)
            .map(|_| {
                i += 1;
                let r = next();
                let key = format!("t:{}/s:{}/{}", r % 6, (r >> 8) % 40, (r >> 16) % 7);
                let idem = format!("e{i}");
                match r % 100 {
                    0 => rm(&format!("t:{}", r % 6), true, &idem),
                    1..=6 => rm(&format!("t:{}/s:{}", r % 6, (r >> 8) % 40), true, &idem),
                    7..=20 => rm(&key, false, &idem),
                    21..=23 => up(&format!("t:{}", r % 6), "top", &idem),
                    _ => up(&key, &"x".repeat((r >> 24) as usize % 50), &idem),
                }
            })
            .collect();
        db = repo.db_append(&db, &batch).unwrap();
        if round % 7 == 3 {
            // A retry changes nothing, counts included.
            assert_eq!(repo.db_append(&db, &batch).unwrap(), db);
        }
        let c = db.counts.unwrap();
        assert_eq!(c.at, db.kv_at);
        assert_eq!(
            repo.db_counts_rebuild(&db.kv, 2).unwrap(),
            c.root,
            "round {round}"
        );
        batches.push(batch);
    }
    let keys: Vec<String> = repo
        .db_scan(&db)
        .unwrap()
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    assert!(keys.len() > 100);
    let mut prefixes = vec![
        "t:0/".to_string(),
        "t:9/".into(),
        "t:1/s:3/".into(),
        "t:".into(),
    ];
    prefixes.extend((0..6).map(|t| format!("t:{t}/")));
    prefixes.extend((0..40).map(|n| format!("t:2/s:{n}/")));
    prefixes.push("t:2/s:1/3/".into()); // deeper than the view: a range count
    for p in &prefixes {
        let end = ikam_kernel::prefix_end(p);
        let want = repo.db_count(&db, p, end.as_deref()).unwrap();
        assert_eq!(repo.db_count_prefix(&db, p).unwrap(), want, "{p}");
        assert_eq!(
            want,
            keys.iter().filter(|k| k.starts_with(p.as_str())).count()
        );
    }
    // A commit keeps the view; turning it on later builds the same view.
    let c = repo.db_commit("db/t", &db, "kept").unwrap();
    assert_eq!(repo.db_at(&c).unwrap(), db);
    let mut other = repo.db_empty().unwrap();
    for b in &batches {
        other = repo.db_append(&other, b).unwrap();
    }
    assert_eq!(other.counts, None);
    let later = repo.db_keep_counts(&other, 2).unwrap();
    assert_eq!(later, db);
    // Without the view the commit has the entries it always had.
    let c2 = repo.db_commit("db/u", &other, "plain").unwrap();
    let back = repo.db_at(&c2).unwrap();
    assert_eq!(back.counts, None);
    assert_eq!(repo.db_keep_counts(&back, 0).unwrap(), back);
}
