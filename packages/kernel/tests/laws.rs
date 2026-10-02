//! Property tests for the kernel laws in docs/plans/2026-10-02-rust-kernel.md.

use std::fs;

use ikam_kernel::matcher::{MIN_MATCH, Part};
use ikam_kernel::{
    Commit, Error, Form, FsStore, Id, Kind, MemStore, Object, Repo, Slice, Store, TreeEntry,
    snapshot,
};
use proptest::prelude::*;

fn any_id() -> impl Strategy<Value = Id> {
    any::<[u8; 32]>().prop_map(Id::from_bytes)
}

fn any_object() -> impl Strategy<Value = Object> {
    let slice = (any_id(), any::<u64>(), 1..u64::MAX).prop_map(|(src, start, len)| Slice {
        src,
        start,
        len,
    });
    let entry = ("[a-z]{1,8}", any::<bool>(), any_id()).prop_map(|(name, f, id)| TreeEntry {
        name,
        kind: if f { Kind::File } else { Kind::Tree },
        id,
    });
    let commit = (any_id(), prop::collection::vec(any_id(), 0..3), ".{0,20}").prop_map(
        |(tree, parents, message)| {
            Object::Commit(Commit {
                tree,
                parents,
                message,
            })
        },
    );
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..256).prop_map(Object::Blob),
        prop::collection::vec(slice, 0..6).prop_map(Object::Rep),
        prop::collection::btree_map("[a-z]{1,8}", entry, 0..6).prop_map(|m| Object::Tree(
            m.into_iter()
                .map(|(name, e)| TreeEntry { name, ..e })
                .collect()
        )),
        commit,
    ]
}

fn bytes(max: usize) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..max)
}

/// An edit: (position fraction, bytes removed, bytes inserted).
fn edits() -> impl Strategy<Value = Vec<(f64, usize, Vec<u8>)>> {
    prop::collection::vec((0.0..1.0f64, 0..64usize, bytes(64)), 1..12)
}

fn apply(base: &[u8], (at, del, ins): &(f64, usize, Vec<u8>)) -> Vec<u8> {
    let i = (at * base.len() as f64) as usize;
    let j = (i + del).min(base.len());
    [&base[..i], ins, &base[j..]].concat()
}

fn rep_slices<S: Store>(repo: &Repo<S>, id: &Id) -> Vec<Slice> {
    match Object::decode(&repo.store().read(id).unwrap()).unwrap() {
        Object::Rep(s) => s,
        other => panic!("expected rep, got {other:?}"),
    }
}

proptest! {
    #[test]
    fn encoding_is_canonical(obj in any_object()) {
        let encoded = obj.encode();
        prop_assert_eq!(&Object::decode(&encoded).unwrap(), &obj);
    }

    #[test]
    fn decode_never_panics_and_is_strict(b in bytes(160)) {
        if let Ok(obj) = Object::decode(&b) {
            prop_assert_eq!(obj.encode(), b);
        }
    }

    #[test]
    fn content_roundtrips_with_stable_id(x in bytes(5000)) {
        let mut repo = Repo::new(MemStore::default());
        let put = repo.put_content(&x).unwrap();
        prop_assert_eq!(put.id, Id::of_content(&x));
        prop_assert_eq!(repo.read_content(&put.id).unwrap(), x.clone());
        prop_assert_eq!(repo.put_content(&x).unwrap().form, Form::Existing);
    }

    /// Every version in an edit history reads back exactly, and its id does
    /// not depend on whether it was stored as a blob or as slices.
    #[test]
    fn edit_histories_roundtrip(base in bytes(20_000), history in edits()) {
        let mut repo = Repo::new(MemStore::default());
        let mut versions = vec![base];
        for e in &history {
            versions.push(apply(versions.last().unwrap(), e));
        }
        for v in &versions {
            let put = repo.put_content(v).unwrap();
            prop_assert_eq!(put.id, Id::of_content(v));
        }
        for v in &versions {
            prop_assert_eq!(&repo.read_content(&Id::of_content(v)).unwrap(), v);
        }
    }

    /// Slice boundaries follow the data: an insertion at an arbitrary offset
    /// is reused up to exactly that byte.
    #[test]
    fn boundaries_are_byte_precise(base in prop::collection::vec(any::<u8>(), 4000..8000), at in 0.1..0.9f64) {
        let k = (at * base.len() as f64) as usize;
        let insert = vec![base[k].wrapping_add(1); 50];
        let edited = [&base[..k], &insert, &base[k..]].concat();
        let mut repo = Repo::new(MemStore::default());
        let base_id = repo.put_content(&base).unwrap().id;
        let put = repo.put_content(&edited).unwrap();
        prop_assert_eq!(put.form, Form::Rep);
        let slices = rep_slices(&repo, &put.id);
        prop_assert_eq!(&slices[0], &Slice { src: base_id, start: 0, len: k as u64 });
        prop_assert_eq!(slices.last().unwrap().src, base_id);
        prop_assert_eq!(repo.read_content(&put.id).unwrap(), edited);
    }

    /// Arbitrary (mostly wrong) plans never lose data: invalid ones fall back.
    #[test]
    fn any_plan_is_safe(base in bytes(2000), x in bytes(2000), raw in prop::collection::vec((any::<bool>(), 0..2100usize, 0..2100usize), 0..8)) {
        let mut repo = Repo::new(MemStore::default());
        let base_id = repo.put_content(&base).unwrap().id;
        let plan: Vec<Part> = raw.iter().map(|&(existing, a, n)| {
            if existing { Part::Existing { src: base_id, start: a, len: n } } else { Part::Input(a..a + n) }
        }).collect();
        let put = repo.put_planned(&x, &plan).unwrap();
        prop_assert_eq!(repo.read_content(&put.id).unwrap(), x);
    }
}

#[test]
fn unique_content_costs_no_overhead() {
    let mut repo = Repo::new(MemStore::default());
    let x: Vec<u8> = (0..10_000u32).map(|i| (i * 7919 % 251) as u8).collect();
    assert_eq!(repo.put_content(&x).unwrap().form, Form::Blob);
    assert_eq!(repo.bytes_written, x.len() + 1);
}

#[test]
fn small_edit_stores_little() {
    let mut repo = Repo::new(MemStore::default());
    let base: Vec<u8> = (0..50_000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect();
    repo.put_content(&base).unwrap();
    let before = repo.bytes_written;
    let edited = [&base[..20_000], b"a small edit", &base[20_000..]].concat();
    assert_eq!(repo.put_content(&edited).unwrap().form, Form::Rep);
    let cost = repo.bytes_written - before;
    assert!(cost < 4 * MIN_MATCH, "edit cost {cost} bytes");
}

#[test]
fn corruption_is_detected_through_reps() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(FsStore::open(dir.path()).unwrap());
    let base: Vec<u8> = (0..5000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 11) as u8)
        .collect();
    let base_id = repo.put_content(&base).unwrap().id;
    let edited = [&base[..2500], b"!", &base[2500..]].concat();
    let put = repo.put_content(&edited).unwrap();
    assert_eq!(put.form, Form::Rep);

    let hex = base_id.to_hex();
    let path = dir.path().join("objects").join(&hex[..2]).join(&hex[2..]);
    let mut tampered = fs::read(&path).unwrap();
    tampered[100] ^= 1;
    fs::write(&path, tampered).unwrap();
    assert!(matches!(
        repo.read_content(&base_id),
        Err(Error::Corrupt(_))
    ));
    assert!(matches!(repo.read_content(&put.id), Err(Error::Corrupt(_))));
}

#[test]
fn commits_checkout_and_log() {
    let work = tempfile::tempdir().unwrap();
    let src = work.path().join("src");
    fs::create_dir_all(src.join("sub")).unwrap();
    fs::write(src.join("a.md"), "# A\n".repeat(200)).unwrap();
    fs::write(src.join("sub/b.txt"), "b").unwrap();
    let store = work.path().join("store");
    let mut repo = Repo::new(FsStore::open(&store).unwrap());

    let t1 = snapshot::snapshot(&mut repo, &src, &store).unwrap();
    let c1 = repo.commit("main", t1, "first").unwrap();
    fs::write(src.join("sub/b.txt"), "b2").unwrap();
    let t2 = snapshot::snapshot(&mut repo, &src, &store).unwrap();
    let c2 = repo.commit("main", t2, "second").unwrap();

    let log: Vec<Id> = repo
        .log(repo.resolve("main").unwrap())
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(log, [c2, c1]);
    let out = work.path().join("out");
    snapshot::checkout(&repo, &snapshot::tree_of(&repo, &c1).unwrap(), &out).unwrap();
    assert_eq!(fs::read(out.join("sub/b.txt")).unwrap(), b"b");
    assert_eq!(
        fs::read(out.join("a.md")).unwrap(),
        fs::read(src.join("a.md")).unwrap()
    );
}

#[test]
fn refs_are_compare_and_swap() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = FsStore::open(dir.path()).unwrap();
    let (a, b) = (Id::of(b"a"), Id::of(b"b"));
    store.set_ref("heads/main", None, a).unwrap();
    assert!(matches!(
        store.set_ref("heads/main", None, b),
        Err(Error::RefConflict(_))
    ));
    store.set_ref("heads/main", Some(a), b).unwrap();
    assert_eq!(store.get_ref("heads/main").unwrap(), Some(b));
    assert!(store.set_ref("../escape", None, a).is_err());
    assert_eq!(store.refs().unwrap(), [("heads/main".to_string(), b)]);
}

#[test]
fn gc_keeps_exactly_what_refs_reach() {
    let mut repo = Repo::new(MemStore::default());
    let base: Vec<u8> = (0..5000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 11) as u8)
        .collect();
    let edited = [&base[..100], b"edit", &base[100..]].concat();
    repo.put_content(&base).unwrap();
    let rep = repo.put_content(&edited).unwrap();
    assert_eq!(rep.form, Form::Rep);
    let orphan = repo.put_content(b"not committed").unwrap().id;

    // Commit only the edited file: its rep's source blob must survive gc.
    let tree = repo
        .put(
            &Object::tree(vec![TreeEntry {
                name: "f".into(),
                kind: Kind::File,
                id: rep.id,
            }])
            .unwrap(),
        )
        .unwrap();
    repo.commit("main", tree, "").unwrap();
    assert!(repo.gc().unwrap() >= 1);
    assert!(matches!(
        repo.read_content(&orphan),
        Err(Error::NotFound(_))
    ));
    assert_eq!(repo.read_content(&rep.id).unwrap(), edited);
}

#[test]
fn trees_reject_bad_names() {
    let e = |name: &str| TreeEntry {
        name: name.into(),
        kind: Kind::File,
        id: Id::of(b""),
    };
    assert!(Object::tree(vec![e("a"), e("a")]).is_err());
    assert!(Object::tree(vec![e("..")]).is_err());
    assert!(Object::tree(vec![e("a/b")]).is_err());
    assert!(Object::tree(vec![e("b"), e("a")]).is_ok());
}

#[test]
fn id_hex_roundtrip_and_rejects_garbage() {
    let id = Id::of_content(&[1, 2, 3]);
    assert_eq!(id, Object::Blob(vec![1, 2, 3]).id());
    assert_eq!(id.to_hex().parse::<Id>().unwrap(), id);
    assert!("xyz".parse::<Id>().is_err());
    assert!(id.to_hex().to_uppercase().parse::<Id>().is_err());
}
