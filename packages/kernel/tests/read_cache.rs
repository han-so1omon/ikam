//! E294: `read_cache` keeps contents read across reads, rebuilt ones
//! included, and changes nothing else.

use ikam_kernel::{Db, Event, MemStore, Repo, Store};

/// The same database writes with and without the cache: a log of upserts
/// over keys that repeat, so the views' chunks are stored as slices of
/// earlier ones and every read rebuilds them.
fn build(cache: usize) -> (Repo<MemStore>, Vec<Db>) {
    let mut repo = Repo::new(MemStore::default());
    repo.read_cache = cache;
    let mut db = repo.db_empty().unwrap();
    let mut versions = Vec::new();
    for round in 0..40 {
        let events: Vec<Event> = (0..25)
            .map(|k| {
                Event::upsert(
                    &format!("item/{k:03}"),
                    format!("round {round} value {}", (k * 7 + round) % 13).as_bytes(),
                    &format!("e{round}-{k}"),
                )
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
        versions.push(db);
        for k in (0..25).step_by(5) {
            repo.db_get(&db, &format!("item/{k:03}")).unwrap();
        }
    }
    (repo, versions)
}

#[test]
fn same_bytes_same_reads() {
    let (plain, a) = build(0);
    let (cached, b) = build(64 << 20);
    assert_eq!(a, b, "the same database versions");
    assert_eq!(plain.bytes_written, cached.bytes_written);
    let mut ids_a = plain.store().ids().unwrap();
    let mut ids_b = cached.store().ids().unwrap();
    ids_a.sort();
    ids_b.sort();
    assert_eq!(ids_a, ids_b, "the same stored objects");
    for db in [a[0], a[19], a[39]] {
        assert_eq!(plain.db_scan(&db).unwrap(), cached.db_scan(&db).unwrap());
        // Read twice: the second comes from the cache.
        assert_eq!(cached.db_scan(&db).unwrap(), plain.db_scan(&db).unwrap());
    }
}

#[test]
fn a_tiny_cache_still_reads_right() {
    let (plain, a) = build(0);
    let (cached, b) = build(300);
    assert_eq!(a, b);
    assert_eq!(plain.bytes_written, cached.bytes_written);
    let last = a[39];
    assert_eq!(
        plain.db_scan(&last).unwrap(),
        cached.db_scan(&last).unwrap()
    );
}

#[test]
fn gc_empties_the_cache() {
    let mut repo = Repo::new(MemStore::default());
    repo.read_cache = 64 << 20;
    let kept = repo.put_content(b"kept by a ref").unwrap().id;
    let dropped = repo.put_content(b"referenced by nothing").unwrap().id;
    assert_eq!(
        repo.read_content(&dropped).unwrap(),
        b"referenced by nothing"
    );
    let tree = repo
        .put(
            &ikam_kernel::Object::tree(vec![ikam_kernel::TreeEntry {
                name: "f".into(),
                kind: ikam_kernel::Kind::File,
                id: kept,
            }])
            .unwrap(),
        )
        .unwrap();
    repo.commit("main", tree, "one file").unwrap();
    assert!(repo.gc().unwrap() > 0);
    assert!(
        repo.read_content(&dropped).is_err(),
        "gone, not served from the cache"
    );
    assert_eq!(repo.read_content(&kept).unwrap(), b"kept by a ref");
}
