//! Every store backend keeps the same contract (E122): run the same laws
//! against each, including a reopen, a whole repository round trip and the
//! database layer. Engines behind cargo features are tested when enabled
//! (`cargo test --features stores --test backends`).

use std::path::Path;

use ikam_kernel::backends::PackStore;
use ikam_kernel::{Event, FsStore, Id, Repo, Store};

fn id(n: u8) -> Id {
    Id::from_bytes([n; 32])
}

/// The raw contract: write once, replace, read, prefix, delete, CAS refs.
fn contract<S: Store>(open: &dyn Fn(&Path) -> S) {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut s = open(dir.path());
        assert!(s.write(id(1), b"one").unwrap());
        assert!(!s.write(id(1), b"one").unwrap(), "second write is a no-op");
        assert!(s.write(id(2), b"two").unwrap());
        let mut near = [1u8; 32];
        near[31] = 9;
        assert!(s.write(Id::from_bytes(near), b"near").unwrap());
        assert_eq!(s.read(&id(1)).unwrap(), b"one");
        assert!(s.has(&id(2)) && !s.has(&id(3)));
        s.replace(id(2), b"TWO").unwrap();
        assert_eq!(s.read(&id(2)).unwrap(), b"TWO");
        assert_eq!(s.ids_with_prefix(&[1, 1]).unwrap().len(), 2);
        assert_eq!(s.ids().unwrap().len(), 3);
        assert_eq!(s.ids_with_prefix(&[]).unwrap().len(), 3);
        // E147: lengths without reads, after a replace and a delete too.
        assert_eq!(s.stored_len(&id(1)).unwrap(), 3);
        s.replace(id(2), b"TWO, longer").unwrap();
        assert_eq!(s.stored_len(&id(2)).unwrap(), 11);
        s.replace(id(2), b"TWO").unwrap();
        assert!(s.stored_len(&id(3)).is_err());
        s.delete(&Id::from_bytes(near)).unwrap();
        assert!(!s.has(&Id::from_bytes(near)));
        assert!(s.read(&Id::from_bytes(near)).is_err());
        assert!(s.stored_len(&Id::from_bytes(near)).is_err());

        assert_eq!(s.get_ref("heads/main").unwrap(), None);
        s.set_ref("heads/main", None, id(1)).unwrap();
        assert!(
            s.set_ref("heads/main", None, id(2)).is_err(),
            "CAS rejects a stale expectation"
        );
        s.set_ref("heads/main", Some(id(1)), id(2)).unwrap();
        assert!(s.set_ref("bad/.name", None, id(1)).is_err());
        assert_eq!(s.refs().unwrap(), vec![("heads/main".to_string(), id(2))]);
        // Written after the last ref move: must still survive a clean close.
        assert!(s.write(id(4), b"four").unwrap());
    }
    let s = open(dir.path());
    assert_eq!(s.read(&id(1)).unwrap(), b"one");
    assert_eq!(s.read(&id(2)).unwrap(), b"TWO");
    assert_eq!(s.read(&id(4)).unwrap(), b"four");
    assert_eq!(s.stored_len(&id(4)).unwrap(), 4);
    let mut ids = s.ids().unwrap();
    ids.sort();
    assert_eq!(ids, vec![id(1), id(2), id(4)]);
    assert_eq!(s.get_ref("heads/main").unwrap(), Some(id(2)));
}

/// A repository over the store: files and a database round trip byte-exact,
/// fsck is clean, and both survive a reopen.
fn repo_round_trip<S: Store>(open: &dyn Fn(&Path) -> S) {
    let dir = tempfile::tempdir().unwrap();
    let text: Vec<u8> = (0..40_000u32)
        .flat_map(|i| format!("line {i} of a test file\n").into_bytes())
        .collect();
    let (file, log) = {
        let mut repo = Repo::new(open(dir.path()));
        let put = repo.put_content(&text).unwrap();
        let empty = repo.db_empty().unwrap();
        let events: Vec<Event> = (0..300)
            .map(|i| {
                Event::upsert(
                    &format!("k/{i:03}"),
                    format!("v{i}").as_bytes(),
                    &format!("e{i}"),
                )
            })
            .collect();
        let db = repo.db_append(&empty, &events).unwrap();
        repo.db_commit("heads/db", &db, "test").unwrap();
        assert!(repo.fsck().unwrap().is_empty());
        (put.id, db.log)
    };
    let mut repo = Repo::new(open(dir.path()));
    assert_eq!(repo.read_content(&file).unwrap(), text);
    assert!(repo.fsck().unwrap().is_empty());
    let db = repo.db_rebuild(&log).unwrap();
    assert_eq!(
        repo.db_get(&db, "k/123").unwrap().as_deref(),
        Some(&b"v123"[..])
    );
    assert_eq!(repo.db_scan(&db).unwrap().len(), 300);
}

fn all<S: Store>(open: &dyn Fn(&Path) -> S) {
    contract(open);
    repo_round_trip(open);
}

#[test]
fn fs_store() {
    all(&|p| FsStore::open(p).unwrap());
}

#[test]
fn pack_store() {
    all(&|p| PackStore::open(p).unwrap());
}

#[test]
fn pack_store_cuts_a_torn_tail_and_keeps_what_a_ref_reached() {
    let dir = tempfile::tempdir().unwrap();
    {
        let mut s = PackStore::open(dir.path()).unwrap();
        s.write(id(1), b"durable").unwrap();
        s.set_ref("heads/main", None, id(1)).unwrap();
    }
    // Simulate a crash mid-append: a record cut short after its header.
    let pack = dir.path().join("packs/00000000.pack");
    let mut bytes = std::fs::read(&pack).unwrap();
    let good = bytes.len();
    bytes.extend_from_slice(&[7u8; 32]);
    bytes.extend_from_slice(&1000u32.to_le_bytes());
    bytes.extend_from_slice(b"partial");
    std::fs::write(&pack, &bytes).unwrap();
    let s = PackStore::open(dir.path()).unwrap();
    assert_eq!(
        std::fs::metadata(&pack).unwrap().len() as usize,
        good,
        "torn tail cut"
    );
    assert_eq!(s.read(&id(1)).unwrap(), b"durable");
    assert!(!s.has(&id(7)));
    // A flipped byte inside a record fails its checksum and is cut, with all after it.
    drop(s);
    let mut bytes = std::fs::read(&pack).unwrap();
    bytes[40] ^= 1;
    std::fs::write(&pack, &bytes).unwrap();
    let s = PackStore::open(dir.path()).unwrap();
    assert!(!s.has(&id(1)), "a corrupt record is never served");
}

#[cfg(feature = "store-sqlite")]
#[test]
fn sqlite_store() {
    use ikam_kernel::backends::sqlite::{Sync, open};
    all(&|p| open(p, Sync::Full).unwrap());
}

#[cfg(feature = "store-lmdb")]
#[test]
fn lmdb_store() {
    all(&|p| ikam_kernel::backends::lmdb::open(p).unwrap());
}

#[cfg(feature = "store-redb")]
#[test]
fn redb_store() {
    all(&|p| ikam_kernel::backends::redb_store::open(p).unwrap());
}

#[cfg(feature = "store-fjall")]
#[test]
fn fjall_store() {
    all(&|p| ikam_kernel::backends::fjall_store::open(p, false).unwrap());
    all(&|p| ikam_kernel::backends::fjall_store::open(p, true).unwrap());
}

#[cfg(feature = "store-rocksdb")]
#[test]
fn rocksdb_store() {
    all(&|p| ikam_kernel::backends::rocks::open(p, false).unwrap());
    all(&|p| ikam_kernel::backends::rocks::open(p, true).unwrap());
}

/// E139: `PackStore::read_head` is a prefix of `read`, from the write
/// buffer, from the open pack on disk and after a reopen.
#[test]
fn pack_heads_are_prefixes_of_reads() {
    let dir = tempfile::tempdir().unwrap();
    let blobs: Vec<Vec<u8>> = [0usize, 1, 255, 256, 257, 10_000]
        .iter()
        .enumerate()
        .map(|(i, &n)| (0..n).map(|j| (i * 31 + j * 7) as u8).collect())
        .collect();
    let check = |s: &PackStore| {
        for b in &blobs {
            let id = ikam_kernel::Id::of_content(b);
            for n in [0, 1, 256, 20_000] {
                assert_eq!(s.read_head(&id, n).unwrap(), &b[..n.min(b.len())]);
            }
        }
    };
    let mut s = PackStore::open(dir.path()).unwrap();
    for b in &blobs {
        s.write(ikam_kernel::Id::of_content(b), b).unwrap();
    }
    check(&s);
    s.sync().unwrap();
    check(&s);
    drop(s);
    check(&PackStore::open(dir.path()).unwrap());
}
