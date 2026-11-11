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
    assert!(
        s.read(&id(1)).is_err(),
        "a corrupt record is never served, even one a checkpoint names"
    );
    // Without the checkpoint, the scan cuts it and all after it.
    drop(s);
    std::fs::remove_file(dir.path().join("packs/00000000.ckpt")).unwrap();
    std::fs::write(&pack, &bytes).unwrap();
    let s = PackStore::open(dir.path()).unwrap();
    assert!(!s.has(&id(1)), "a corrupt record is cut by the scan");
}

#[test]
fn pack_store_checkpoint_opens_the_same_index_as_a_full_scan() {
    let dir = tempfile::tempdir().unwrap();
    let blob = |n: u32| -> (Id, Vec<u8>) {
        let b = format!("object {n} {}", "x".repeat((n % 300) as usize)).into_bytes();
        (Id::of(&b), b)
    };
    {
        let mut s = PackStore::open(dir.path()).unwrap();
        for n in 0..5000 {
            let (i, b) = blob(n);
            s.write(i, &b).unwrap();
        }
        s.set_ref("heads/main", None, blob(0).0).unwrap();
    } // close writes a checkpoint
    let ckpt = dir.path().join("packs/00000000.ckpt");
    assert!(ckpt.exists());
    {
        // More records after the checkpoint, and no clean close.
        let mut s = PackStore::open(dir.path()).unwrap();
        for n in 5000..6000 {
            let (i, b) = blob(n);
            s.write(i, &b).unwrap();
        }
        s.set_ref("heads/main", Some(blob(0).0), blob(5000).0)
            .unwrap();
        let saved = std::fs::read(&ckpt).unwrap();
        drop(s);
        std::fs::write(&ckpt, saved).unwrap(); // as if the close never ran
    }
    let s = PackStore::open(dir.path()).unwrap();
    let mut with: Vec<Id> = s.ids().unwrap();
    with.sort();
    assert_eq!(with.len(), 6000, "checkpoint plus a scan of what followed");
    drop(s);
    // A torn checkpoint falls back to a full scan with the same result.
    let mut bytes = std::fs::read(&ckpt).unwrap();
    bytes.truncate(bytes.len() / 2);
    std::fs::write(&ckpt, bytes).unwrap();
    let s = PackStore::open(dir.path()).unwrap();
    let mut without: Vec<Id> = s.ids().unwrap();
    without.sort();
    assert_eq!(with, without);
    for n in (0..6000).step_by(97) {
        let (i, b) = blob(n);
        assert_eq!(s.read(&i).unwrap(), b);
    }
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

#[test]
fn fs_store_ref_survives_a_writer_killed_mid_update() {
    // A writer killed between taking the lock and the rename leaves
    // refs/<name>.lock behind; the next update must still go through.
    let dir = tempfile::tempdir().unwrap();
    let mut s = FsStore::open(dir.path()).unwrap();
    s.set_ref("heads/main", None, id(1)).unwrap();
    std::fs::write(dir.path().join("refs/heads/main.lock"), b"half-written").unwrap();
    s.set_ref("heads/main", Some(id(1)), id(2)).unwrap();
    assert_eq!(s.get_ref("heads/main").unwrap(), Some(id(2)));
    // A lock held by a live writer still refuses.
    let held = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.path().join("refs/heads/main.lock"))
        .unwrap();
    held.lock().unwrap();
    assert!(matches!(
        s.set_ref("heads/main", Some(id(2)), id(3)),
        Err(ikam_kernel::Error::RefConflict(_))
    ));
    drop(held);
    s.set_ref("heads/main", Some(id(2)), id(3)).unwrap();
}

/// E171: the lazy checkpoint keeps the contract, and a store opened lazily
/// sees exactly what a full load sees, before and after more writes.
#[test]
fn pack_store_lazy_checkpoint() {
    use ikam_kernel::backends::PackOptions;
    let lazy = PackOptions {
        lazy_checkpoint: true,
        ..PackOptions::default()
    };
    all(&|p| PackStore::open_with(p, lazy).unwrap());
    let dir = tempfile::tempdir().unwrap();
    let blob = |n: u32| -> (Id, Vec<u8>) {
        let b = format!("object {n} {}", "y".repeat((n % 200) as usize)).into_bytes();
        (Id::of(&b), b)
    };
    {
        let mut s = PackStore::open_with(dir.path(), lazy).unwrap();
        for n in 0..4000 {
            let (i, b) = blob(n);
            s.write(i, &b).unwrap();
        }
        s.set_ref("heads/main", None, blob(0).0).unwrap();
        s.delete(&blob(7).0).unwrap();
        s.replace(blob(8).0, b"replaced").unwrap();
    } // close writes the checkpoint
    {
        // Lazily opened, then more writes and a replace of a checkpointed id.
        let mut s = PackStore::open_with(dir.path(), lazy).unwrap();
        assert!(!s.has(&blob(7).0));
        assert_eq!(s.read(&blob(8).0).unwrap(), b"replaced");
        for n in 4000..5000 {
            let (i, b) = blob(n);
            s.write(i, &b).unwrap();
        }
        s.replace(blob(9).0, b"again").unwrap();
        s.set_ref("heads/main", Some(blob(0).0), blob(4000).0)
            .unwrap();
    }
    // One open at a time (the lock): take a full view of each.
    type View = (
        Vec<Id>,
        Vec<(Id, bool, Option<(Vec<u8>, usize)>)>,
        Vec<Vec<Id>>,
    );
    let view = |s: &PackStore| -> View {
        let mut ids = s.ids().unwrap();
        ids.sort();
        let probes = (0..5000)
            .step_by(13)
            .map(|n| {
                let i = blob(n).0;
                let got = s
                    .has(&i)
                    .then(|| (s.read(&i).unwrap(), s.stored_len(&i).unwrap()));
                (i, s.has(&i), got)
            })
            .collect();
        let prefixes = [&[0x00u8][..], &[0x7f], &[0xff, 0x01]]
            .iter()
            .map(|p| s.ids_with_prefix(p).unwrap())
            .collect();
        (ids, probes, prefixes)
    };
    let full = view(&PackStore::open(dir.path()).unwrap());
    let lz = view(&PackStore::open_with(dir.path(), lazy).unwrap());
    assert_eq!(full.0.len(), 4999);
    assert!(full == lz, "lazy open sees what a full load sees");
    {
        let s = PackStore::open_with(dir.path(), lazy).unwrap();
        assert_eq!(s.read(&blob(9).0).unwrap(), b"again");
    }
    // A torn checkpoint is refused and the open falls back to a full scan.
    let ckpt = dir.path().join("packs/00000000.ckpt");
    let mut bytes = std::fs::read(&ckpt).unwrap();
    bytes.truncate(bytes.len() - 5);
    std::fs::write(&ckpt, bytes).unwrap();
    let s = PackStore::open_with(dir.path(), lazy).unwrap();
    assert_eq!(s.ids().unwrap().len(), 4999);
}

/// E170: both hybrids keep the contract.
#[cfg(feature = "store-rocksdb")]
#[test]
fn hybrid_stores() {
    use ikam_kernel::backends::hybrid::{IndexHybrid, RefsHybrid};
    all(&|p| RefsHybrid::open(p).unwrap());
    all(&|p| IndexHybrid::open(p).unwrap());
}

/// E172: refs logged in the pack keep the contract, alone and with E171.
#[test]
fn pack_store_ref_log() {
    use ikam_kernel::backends::PackOptions;
    let log = PackOptions {
        ref_log: true,
        ..PackOptions::default()
    };
    let both = PackOptions {
        ref_log: true,
        lazy_checkpoint: true,
        ..PackOptions::default()
    };
    all(&|p| PackStore::open_with(p, log).unwrap());
    all(&|p| PackStore::open_with(p, both).unwrap());

    // A ref record is durable with the objects before it, and a torn record
    // ahead of it takes the ref with it: the ref falls back, never forward.
    let dir = tempfile::tempdir().unwrap();
    let pack = dir.path().join("packs/00000000.pack");
    {
        let mut s = PackStore::open_with(dir.path(), log).unwrap();
        s.write(id(1), b"first").unwrap();
        s.set_ref("heads/main", None, id(1)).unwrap();
        s.write(id(2), b"second").unwrap();
        s.set_ref("heads/main", Some(id(1)), id(2)).unwrap();
    }
    // As after a kill: no checkpoint and no snapshot, so refs come from replay.
    std::fs::remove_file(dir.path().join("refs.snap")).unwrap();
    std::fs::remove_file(dir.path().join("packs/00000000.ckpt")).ok();
    let len = std::fs::metadata(&pack).unwrap().len() as usize;
    {
        let s = PackStore::open_with(dir.path(), log).unwrap();
        assert_eq!(s.get_ref("heads/main").unwrap(), Some(id(2)));
        assert_eq!(s.read(&id(2)).unwrap(), b"second");
        assert!(s.ids().unwrap().len() == 2, "ref records are not objects");
    }
    std::fs::remove_file(dir.path().join("refs.snap")).ok();
    std::fs::remove_file(dir.path().join("packs/00000000.ckpt")).ok();
    // Flip a byte inside object 2's record: it and the ref after it are cut.
    let mut bytes = std::fs::read(&pack).unwrap();
    assert_eq!(bytes.len(), len);
    let at = bytes.windows(6).position(|w| w == b"second").unwrap();
    bytes[at] ^= 1;
    std::fs::write(&pack, &bytes).unwrap();
    std::fs::remove_file(dir.path().join("packs/00000000.ckpt")).ok();
    std::fs::remove_file(dir.path().join("refs.snap")).ok();
    let s = PackStore::open_with(dir.path(), log).unwrap();
    assert_eq!(s.get_ref("heads/main").unwrap(), Some(id(1)));
    assert!(!s.has(&id(2)));
    assert_eq!(s.read(&id(1)).unwrap(), b"first");
    drop(s);

    // A store that kept refs as files carries them over on first open.
    let dir = tempfile::tempdir().unwrap();
    {
        let mut s = PackStore::open(dir.path()).unwrap();
        s.write(id(3), b"three").unwrap();
        s.set_ref("heads/old", None, id(3)).unwrap();
    }
    {
        let mut s = PackStore::open_with(dir.path(), log).unwrap();
        assert_eq!(s.get_ref("heads/old").unwrap(), Some(id(3)));
        s.set_ref("heads/old", Some(id(3)), id(3)).unwrap();
    }
    let s = PackStore::open_with(dir.path(), log).unwrap();
    assert_eq!(s.refs().unwrap(), vec![("heads/old".to_string(), id(3))]);
}

/// E175: with filters, every stored id is found and no other, through a
/// lazy checkpoint's filter; a damaged filter is rebuilt, never trusted.
/// (Sealed packs' filters are tested in the crate, at a smaller scale.)
#[test]
fn pack_store_filters() {
    use ikam_kernel::backends::PackOptions;
    let f = PackOptions {
        filters: true,
        ..PackOptions::default()
    };
    let all_on = PackOptions {
        filters: true,
        lazy_checkpoint: true,
        ref_log: true,
    };
    all(&|p| PackStore::open_with(p, f).unwrap());
    all(&|p| PackStore::open_with(p, all_on).unwrap());

    let obj = |i: u64| {
        let b = format!("object {i}").into_bytes();
        (Id::of_content(&b), b)
    };
    let dir = tempfile::tempdir().unwrap();
    {
        let mut s = PackStore::open_with(dir.path(), all_on).unwrap();
        for i in 0..20_000 {
            let (id, b) = obj(i);
            s.write(id, &b).unwrap();
        }
        s.set_ref("heads/main", None, obj(0).0).unwrap();
    }
    let check = |s: &PackStore| {
        for i in 0..20_000 {
            let (id, b) = obj(i);
            assert_eq!(s.read(&id).unwrap(), b, "lost {i}");
        }
        for i in 20_000..30_000 {
            assert!(!s.has(&obj(i).0));
        }
    };
    check(&PackStore::open_with(dir.path(), all_on).unwrap());
    let cflt = dir.path().join("packs/00000000.cflt");
    assert!(cflt.exists(), "the lazy checkpoint has a filter");
    let mut b = std::fs::read(&cflt).unwrap();
    for x in b[20..].iter_mut().take(4096) {
        *x = 0;
    }
    std::fs::write(&cflt, b).unwrap();
    check(&PackStore::open_with(dir.path(), all_on).unwrap());
    check(&PackStore::open(dir.path()).unwrap());
}

/// E176: the pack cleaner keeps exactly the newest version of every live
/// object and every ref, drops the rest, and a kill partway (old packs still
/// there, or some removed oldest first) reads the same.
#[test]
fn pack_store_compact() {
    use ikam_kernel::backends::PackOptions;
    use std::collections::BTreeMap;
    let obj = |i: u64, v: u64| {
        format!("object {i} version {v} {}", "x".repeat((i % 50) as usize)).into_bytes()
    };
    let key = |i: u64| Id::of_content(format!("key {i}").as_bytes());
    for opts in [
        PackOptions::default(),
        PackOptions {
            ref_log: true,
            ..PackOptions::default()
        },
        PackOptions {
            ref_log: true,
            lazy_checkpoint: true,
            filters: true,
        },
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut want: BTreeMap<Id, Vec<u8>> = BTreeMap::new();
        let mut gone = Vec::new();
        let mut s = PackStore::open_with(dir.path(), opts).unwrap();
        let mut head = None;
        for round in 0..4u64 {
            for i in 0..3_000u64 {
                let k = key(i);
                match (i + round) % 7 {
                    0 if want.contains_key(&k) => {
                        s.delete(&k).unwrap();
                        want.remove(&k);
                        gone.push(k);
                    }
                    1 | 2 => {
                        let b = obj(i, round);
                        s.replace(k, &b).unwrap();
                        want.insert(k, b);
                    }
                    _ if !want.contains_key(&k) => {
                        let b = obj(i, round);
                        s.write(k, &b).unwrap();
                        want.insert(k, b);
                    }
                    _ => {}
                }
            }
            let r = key(10_000 + round);
            s.set_ref("heads/main", head, r).unwrap();
            head = Some(r);
            if round % 2 == 1 {
                let c = s.compact().unwrap();
                assert!(c.dropped > 0 && c.bytes_after < c.bytes_before, "{c:?}");
            }
        }
        // Old packs kept, as after a kill before the cleaner removed any.
        let before = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(before.path().join("packs")).unwrap();
        for e in std::fs::read_dir(dir.path().join("packs")).unwrap() {
            let p = e.unwrap().path();
            std::fs::copy(&p, before.path().join("packs").join(p.file_name().unwrap())).unwrap();
        }
        let c = s.compact().unwrap();
        assert_eq!(c.kept, want.len() as u64);
        drop(s);
        let check = |p: &Path| {
            let s = PackStore::open_with(p, opts).unwrap();
            for (k, b) in &want {
                assert_eq!(&s.read(k).unwrap(), b);
            }
            for k in &gone {
                if !want.contains_key(k) {
                    assert!(!s.has(k));
                }
            }
            assert_eq!(s.get_ref("heads/main").unwrap(), head);
            let mut ids = s.ids().unwrap();
            ids.sort();
            assert_eq!(ids, want.keys().copied().collect::<Vec<_>>());
        };
        check(dir.path());
        if opts.ref_log {
            // Kill before any old pack was removed: the old packs come back
            // (only those the cleaner removed; it never rewrites a file).
            for e in std::fs::read_dir(before.path().join("packs")).unwrap() {
                let p = e.unwrap().path();
                let to = dir.path().join("packs").join(p.file_name().unwrap());
                if !to.exists() {
                    std::fs::copy(&p, to).unwrap();
                }
            }
            check(dir.path());
            // Kill after the oldest were removed: remove the lowest numbers.
            let mut nos: Vec<_> = std::fs::read_dir(before.path().join("packs"))
                .unwrap()
                .filter_map(|e| {
                    e.unwrap()
                        .file_name()
                        .to_str()?
                        .strip_suffix(".pack")
                        .map(String::from)
                })
                .collect();
            nos.sort();
            let first = &nos[0];
            std::fs::remove_file(dir.path().join(format!("packs/{first}.pack"))).unwrap();
            check(dir.path());
        }
    }
}

/// E177: writers sharing flushes. Eight threads each build a chain on their
/// own ref; every commit returns only once durable, a stale expected value
/// is refused, and after a reopen every chain is whole. The store must have
/// the ref log; staging a ref without it is refused.
#[test]
fn pack_store_group_commit() {
    use ikam_kernel::backends::{GroupCommit, PackOptions};
    use std::sync::Arc;
    let opts = PackOptions {
        lazy_checkpoint: true,
        ref_log: true,
        ..PackOptions::default()
    };
    let dir = tempfile::tempdir().unwrap();
    assert!(GroupCommit::new(PackStore::open(dir.path().join("plain")).unwrap()).is_err());
    let mut plain = PackStore::open(dir.path().join("plain2")).unwrap();
    assert!(plain.stage_ref("heads/x", None, id(1)).is_err());
    drop(plain);

    let root = dir.path().join("g");
    let g = Arc::new(GroupCommit::new(PackStore::open_with(&root, opts).unwrap()).unwrap());
    let chain = |t: u64, i: u64| {
        let b = format!("w{t} commit {i} {}", "x".repeat((i % 50) as usize)).into_bytes();
        (Id::from_bytes(*blake3::hash(&b).as_bytes()), b)
    };
    let hs: Vec<_> = (0..8u64)
        .map(|t| {
            let g = g.clone();
            std::thread::spawn(move || {
                let name = format!("heads/w{t}");
                let mut head = None;
                for i in 0..300 {
                    let (cid, b) = chain(t, i);
                    g.commit(&[(cid, b)], &name, head, cid).unwrap();
                    head = Some(cid);
                }
                // A stale expected value is refused, and moves nothing.
                let (cid, b) = chain(t, 1_000);
                assert!(g.commit(&[(cid, b)], &name, None, cid).is_err());
                assert_eq!(g.get_ref(&name).unwrap(), head);
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    let g = Arc::try_unwrap(g).ok().unwrap();
    assert!(g.flushes() <= 8 * 300 + 8);
    drop(g.into_inner());
    let s = PackStore::open_with(&root, opts).unwrap();
    for t in 0..8 {
        let (last, _) = chain(t, 299);
        assert_eq!(s.get_ref(&format!("heads/w{t}")).unwrap(), Some(last));
        for i in 0..300 {
            let (cid, b) = chain(t, i);
            assert_eq!(s.read(&cid).unwrap(), b);
        }
    }
}

/// E179: a tail the open had to scan (the process died before its next
/// checkpoint) is saved by that open, so a store killed again and again
/// does not rescan an ever longer tail. Checked with and without the
/// lazy checkpoint and the ref log.
#[test]
fn pack_store_saves_a_scanned_tail() {
    use ikam_kernel::backends::PackOptions;
    for opts in [
        PackOptions::default(),
        PackOptions {
            lazy_checkpoint: true,
            ref_log: true,
            ..PackOptions::default()
        },
    ] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let big = vec![7u8; 1 << 20];
        let mut last = None;
        {
            let mut s = PackStore::open_with(root, opts).unwrap();
            for i in 0..70u32 {
                let mut b = big.clone();
                b[..4].copy_from_slice(&i.to_le_bytes());
                let id = Id::from_bytes(*blake3::hash(&b).as_bytes());
                s.write(id, &b).unwrap();
                last = Some(id);
            }
            s.set_ref("heads/t", None, last.unwrap()).unwrap();
        }
        // As if the process had died before saving anything: no checkpoint.
        let ckpt = root.join("packs/00000000.ckpt");
        std::fs::remove_file(&ckpt).unwrap();
        let _ = std::fs::remove_file(root.join("packs/00000000.cflt"));
        let _ = std::fs::remove_file(root.join("refs.snap"));
        let s = PackStore::open_with(root, opts).unwrap();
        assert!(ckpt.exists(), "the open saved the tail it scanned");
        assert_eq!(s.get_ref("heads/t").unwrap(), last);
        assert_eq!(s.read(&last.unwrap()).unwrap()[4..], big[4..]);
        drop(s);
        let s = PackStore::open_with(root, opts).unwrap();
        assert_eq!(s.ids().unwrap().len(), 70);
    }
}

#[test]
fn pack_store_prefix_lookups_in_the_open_pack_match_a_mem_store() {
    // E190: the open pack's index is ordered, so a prefix lookup is a
    // range. Same answers as a MemStore for every prefix length, with
    // deletes, before and after a reopen.
    let dir = tempfile::tempdir().unwrap();
    let mut mem = ikam_kernel::MemStore::default();
    let ids: Vec<Id> = (0..2_000u32).map(|i| Id::of(&i.to_be_bytes())).collect();
    {
        let mut pack = PackStore::open(dir.path()).unwrap();
        for (i, id) in ids.iter().enumerate() {
            pack.write(*id, b"x").unwrap();
            mem.write(*id, b"x").unwrap();
            if i % 7 == 0 {
                pack.delete(id).unwrap();
                mem.delete(id).unwrap();
            }
        }
        for id in ids.iter().step_by(13) {
            for n in [0, 1, 2, 5, 32] {
                let p = &id.as_bytes()[..n];
                assert_eq!(
                    pack.ids_with_prefix(p).unwrap(),
                    mem.ids_with_prefix(p).unwrap()
                );
            }
        }
    }
    let pack = PackStore::open(dir.path()).unwrap();
    for id in ids.iter().step_by(13) {
        let p = &id.as_bytes()[..2];
        assert_eq!(
            pack.ids_with_prefix(p).unwrap(),
            mem.ids_with_prefix(p).unwrap()
        );
    }
}
