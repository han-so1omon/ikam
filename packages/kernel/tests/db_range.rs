//! E240: range drops in graphs and the database, and stable row ids.

use ikam_kernel::{Arg, Db, Event, Id, MemStore, Node, Repo, Target};

fn node(label: &str) -> Node {
    Node {
        label: label.into(),
        target: Target::Content(Arg::Whole(Id::of_content(label.as_bytes()))),
        edges: Vec::new(),
    }
}

/// A small deterministic generator, so the cases are the same every run.
struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

#[test]
fn drop_range_equals_put_graph_of_the_rest() {
    let mut rng = Lcg(7);
    for &n in &[0usize, 1, 50, 900, 6000, 30000] {
        let labels: Vec<String> = (0..n).map(|i| format!("k/{:06}", i * 3)).collect();
        let mut repo = Repo::new(MemStore::default());
        let root = repo
            .put_graph(labels.iter().map(|l| node(l)).collect())
            .unwrap();
        for _ in 0..12 {
            let a = rng.next() as usize % (n * 3 + 2);
            let len = rng.next() as usize % (n * 3 + 2);
            let lo = format!("k/{:06}", a);
            let hi = format!("k/{:06}", a + len);
            for hi in [Some(hi.as_str()), None] {
                let rest: Vec<Node> = labels
                    .iter()
                    .filter(|l| !(l.as_str() >= lo.as_str() && hi.is_none_or(|h| l.as_str() < h)))
                    .map(|l| node(l))
                    .collect();
                let want = repo.put_graph(rest.clone()).unwrap();
                let got = repo.graph_drop_range(&root, &lo, hi).unwrap();
                assert_eq!(got, want, "n={n} lo={lo} hi={hi:?}");
                let inside = repo.graph_in_range(&root, &lo, hi).unwrap();
                assert_eq!(inside.len() + rest.len(), n);
            }
        }
        // Ranges outside every label, and around all of them.
        assert_eq!(repo.graph_drop_range(&root, "z", None).unwrap(), root);
        assert_eq!(repo.graph_drop_range(&root, "a", Some("b")).unwrap(), root);
        let empty = repo.put_graph(Vec::new()).unwrap();
        assert_eq!(repo.graph_drop_range(&root, "", None).unwrap(), empty);
    }
}

fn base(repo: &mut Repo<MemStore>, rows: usize) -> Db {
    let mut events = Vec::new();
    for t in ["a", "b", "c"] {
        for i in 0..rows {
            let k = format!("{t}/{i:05}");
            events.push(Event::upsert(
                &k,
                format!("v{i}").as_bytes(),
                &format!("w-{k}"),
            ));
        }
        events.push(Event::upsert(t, b"top", &format!("w-{t}")));
    }
    let empty = repo.db_empty().unwrap();
    repo.db_append(&empty, &events).unwrap()
}

fn both_ways(rows: usize, batch: &[Event]) -> Db {
    let mut roots = Vec::new();
    for fast in [false, true] {
        let mut repo = Repo::new(MemStore::default());
        repo.db_range_drop = fast;
        let db = base(&mut repo, rows);
        let after = repo.db_append(&db, batch).unwrap();
        assert_eq!(repo.db_rebuild(&after.log).unwrap(), after, "fast={fast}");
        roots.push(after);
    }
    assert_eq!(roots[0], roots[1]);
    roots[0]
}

#[test]
fn subtree_remove_is_the_same_with_range_drop() {
    for rows in [3, 400, 5000] {
        both_ways(rows, &[Event::remove("b", true, "rm-b")]);
        both_ways(
            rows,
            &[
                Event::upsert("b/new", b"before", "u1"),
                Event::remove("b", true, "rm-b"),
                Event::upsert("b/after", b"kept", "u2"),
                Event::remove("a", true, "rm-a"),
            ],
        );
    }
}

#[test]
fn remove_range_drops_keys_in_range() {
    let rows = 2000;
    let batch = [
        Event::upsert("b/00010", b"rewritten first", "u0"),
        Event::remove_range("b/00010", "b/01500", "rr"),
        Event::upsert("b/00020", b"after", "u1"),
    ];
    let db = both_ways(rows, &batch);
    let mut repo = Repo::new(MemStore::default());
    let start = base(&mut repo, rows);
    let db2 = repo.db_append(&start, &batch).unwrap();
    assert_eq!(db, db2);
    assert_eq!(repo.db_get(&db2, "b/00009").unwrap().unwrap(), b"v9");
    assert_eq!(repo.db_get(&db2, "b/00010").unwrap(), None);
    assert_eq!(repo.db_get(&db2, "b/00020").unwrap().unwrap(), b"after");
    assert_eq!(repo.db_get(&db2, "b/01499").unwrap(), None);
    assert_eq!(repo.db_get(&db2, "b/01500").unwrap().unwrap(), b"v1500");
    // Events round-trip, and an empty or reversed range is refused.
    let e = Event::remove_range("x/1", "x/9", "i");
    assert_eq!(Event::decode(&e.encode()).unwrap().0, e);
    let bad = Event::remove_range("x/9", "x/1", "j");
    assert!(repo.db_append(&db2, std::slice::from_ref(&bad)).is_err());
    assert!(Event::decode(&bad.encode()).is_err());
}

#[test]
fn row_ids_are_stable_and_retries_keep_them() {
    let mut repo = Repo::new(MemStore::default());
    let db = repo.db_empty().unwrap();
    let db = repo
        .db_append(&db, &[Event::upsert("other", b"x", "o")])
        .unwrap();
    let rows: Vec<(&[u8], &str)> = vec![(b"r1", "i1"), (b"r2", "i2"), (b"r3", "i3")];
    let (db, keys) = repo.db_insert(&db, "t", &rows).unwrap();
    assert_eq!(
        keys,
        vec![
            Db::row_key("t", 2),
            Db::row_key("t", 3),
            Db::row_key("t", 4)
        ]
    );
    // A retry, and a retry mixed with a new row.
    let (db2, again) = repo.db_insert(&db, "t", &rows[..2]).unwrap();
    assert_eq!((db2, again), (db, keys[..2].to_vec()));
    let (db3, k3) = repo
        .db_insert(&db, "t", &[(b"r1", "i1"), (b"r4", "i4")])
        .unwrap();
    assert_eq!(k3, vec![keys[0].clone(), Db::row_key("t", 5)]);
    // Ids are not reused after a range of them is removed.
    let cut = Event::remove_range(&Db::row_key("t", 0), &Db::row_key("t", 4), "cut");
    let db4 = repo.db_append(&db3, &[cut]).unwrap();
    let (_, k5) = repo.db_insert(&db4, "t", &[(b"r5", "i5")]).unwrap();
    assert_eq!(k5, vec![Db::row_key("t", 7)]);
    let left: Vec<String> = repo
        .db_scan(&db4)
        .unwrap()
        .into_iter()
        .map(|p| p.0)
        .collect();
    assert_eq!(
        left,
        vec![
            "other".to_string(),
            Db::row_key("t", 4),
            Db::row_key("t", 5)
        ]
    );
}
