//! Property tests for the kernel laws in docs/plans/2026-10-02-rust-kernel.md.

use std::fs;
use std::path::Path;

use ikam_kernel::matcher::MIN_MATCH;
use ikam_kernel::{
    Arg, Commit, Derivation, Error, Form, FsStore, Id, Kind, MemStore, Object, Repo, Store,
    TreeEntry, func, snapshot,
};
use proptest::prelude::*;

fn any_id() -> impl Strategy<Value = Id> {
    any::<[u8; 32]>().prop_map(Id::from_bytes)
}

fn any_arg() -> impl Strategy<Value = Arg> {
    prop_oneof![
        any_id().prop_map(Arg::Whole),
        (any_id(), 0..u64::MAX / 2, 1..u64::MAX / 2).prop_map(|(id, start, len)| Arg::Range {
            id,
            start,
            len
        }),
    ]
}

fn any_object() -> impl Strategy<Value = Object> {
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
    let derivation = (any_id(), any_id(), prop::collection::vec(any_arg(), 0..5))
        .prop_map(|(output, func, args)| Object::Derivation(Derivation { output, func, args }));
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..256).prop_map(Object::Blob),
        prop::collection::btree_map("[a-z]{1,8}", entry, 0..6).prop_map(|m| Object::Tree(
            m.into_iter()
                .map(|(name, e)| TreeEntry { name, ..e })
                .collect()
        )),
        commit,
        derivation,
    ]
}

fn bytes(max: usize) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..max)
}

/// An edit: (position fraction, bytes removed, bytes inserted).
fn edits() -> impl Strategy<Value = Vec<(f64, usize, Vec<u8>)>> {
    prop::collection::vec((0.0..1.0f64, 0..64usize, bytes(64)), 1..12)
}

fn edit(base: &[u8], (at, del, ins): &(f64, usize, Vec<u8>)) -> Vec<u8> {
    let i = (at * base.len() as f64) as usize;
    let j = (i + del).min(base.len());
    [&base[..i], ins, &base[j..]].concat()
}

fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut state = seed | 1;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 56) as u8
        })
        .collect()
}

fn object_path(root: &Path, id: &Id) -> std::path::PathBuf {
    let hex = id.to_hex();
    root.join("objects").join(&hex[..2]).join(&hex[2..])
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
    /// not depend on how it was stored.
    #[test]
    fn edit_histories_roundtrip(base in bytes(20_000), history in edits()) {
        let mut repo = Repo::new(MemStore::default());
        let mut versions = vec![base];
        for e in &history {
            versions.push(edit(versions.last().unwrap(), e));
        }
        for v in &versions {
            prop_assert_eq!(repo.put_content(v).unwrap().id, Id::of_content(v));
        }
        for v in &versions {
            prop_assert_eq!(&repo.read_content(&Id::of_content(v)).unwrap(), v);
        }
        prop_assert!(repo.fsck().unwrap().is_empty());
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
        prop_assert_eq!(put.form, Form::Derived);
        let d = &repo.derivations(&put.id).unwrap()[0];
        prop_assert_eq!(d.func, func::concat());
        prop_assert_eq!(d.args[0], Arg::Range { id: base_id, start: 0, len: k as u64 });
        prop_assert_eq!(d.args.last().unwrap().id(), base_id);
        prop_assert_eq!(repo.read_content(&put.id).unwrap(), edited);
    }

    /// Arbitrary (mostly wrong) derivation proposals never lose data: the
    /// wrong ones are rejected and write nothing.
    #[test]
    fn any_proposal_is_safe(base in bytes(2000), x in bytes(2000),
                            raw in prop::collection::vec((0..2100u64, 1..2100u64), 0..8)) {
        let mut repo = Repo::new(MemStore::default());
        let base_id = repo.put_content(&base).unwrap().id;
        let args = raw.iter().map(|&(start, len)| Arg::Range { id: base_id, start, len }).collect();
        let before = repo.bytes_written;
        match repo.put_derivation(&x, func::concat(), args).unwrap() {
            Some(put) => prop_assert_eq!(repo.read_content(&put.id).unwrap(), x),
            None => prop_assert_eq!(repo.bytes_written, before),
        }
    }
}

#[test]
fn incompressible_unique_content_costs_one_byte() {
    let mut repo = Repo::new(MemStore::default());
    let x = noise(10_000, 7);
    assert_eq!(repo.put_content(&x).unwrap().form, Form::Blob);
    assert_eq!(repo.bytes_written, x.len() + 1);
}

#[test]
fn compressible_content_is_stored_compressed() {
    let mut repo = Repo::new(MemStore::default());
    let x = "the same sentence again and again. "
        .repeat(500)
        .into_bytes();
    let put = repo.put_content(&x).unwrap();
    assert_eq!(put.id, Id::of_content(&x));
    assert!(
        repo.bytes_written < x.len() / 10,
        "stored {} bytes",
        repo.bytes_written
    );
    assert_eq!(repo.read_content(&put.id).unwrap(), x);
}

#[test]
fn small_edit_stores_little() {
    let mut repo = Repo::new(MemStore::default());
    let base = noise(50_000, 11);
    repo.put_content(&base).unwrap();
    let before = repo.bytes_written;
    let edited = [&base[..20_000], b"a small edit", &base[20_000..]].concat();
    assert_eq!(repo.put_content(&edited).unwrap().form, Form::Derived);
    let cost = repo.bytes_written - before;
    assert!(cost < 4 * MIN_MATCH, "edit cost {cost} bytes");
}

/// A range may point into content that is itself only derived: reuse is not
/// limited to stored bytes.
#[test]
fn ranges_may_point_into_derived_content() {
    let mut repo = Repo::new(MemStore::default());
    let base = noise(8000, 3);
    repo.put_content(&base).unwrap();
    let mid = [&base[..4000], b"--inserted--", &base[4000..]].concat();
    let mid_id = repo.put_content(&mid).unwrap().id;
    assert!(!repo.store().has(&mid_id), "mid is derived, not stored");
    let tail = mid[3990..4020].to_vec();
    let put = repo
        .put_derivation(
            &tail,
            func::concat(),
            vec![Arg::Range {
                id: mid_id,
                start: 3990,
                len: 30,
            }],
        )
        .unwrap()
        .unwrap();
    assert_eq!(repo.read_content(&put.id).unwrap(), tail);
}

/// One id may have several derivations, and a corrupt stored copy falls
/// through to them: the read heals, while fsck still reports the damage.
#[test]
fn derivations_heal_a_corrupt_copy() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(FsStore::open(dir.path()).unwrap());
    let (a, b) = (noise(3000, 5), noise(3000, 9));
    let (a_id, b_id) = (
        repo.put_content(&a).unwrap().id,
        repo.put_content(&b).unwrap().id,
    );
    let ab = [a.clone(), b.clone()].concat();
    let ab_id = Id::of_content(&ab);
    // Store ab's bytes directly, then record two independent derivations.
    fs::create_dir_all(object_path(dir.path(), &ab_id).parent().unwrap()).unwrap();
    fs::write(
        object_path(dir.path(), &ab_id),
        [b"B".as_slice(), &ab].concat(),
    )
    .unwrap();
    let whole = vec![Arg::Whole(a_id), Arg::Whole(b_id)];
    let halves = vec![
        Arg::Range {
            id: a_id,
            start: 0,
            len: 1500,
        },
        Arg::Range {
            id: a_id,
            start: 1500,
            len: 1500,
        },
        Arg::Whole(b_id),
    ];
    assert_eq!(
        repo.put_derivation(&ab, func::concat(), whole)
            .unwrap()
            .unwrap()
            .form,
        Form::Existing
    );
    repo.put_derivation(&ab, func::concat(), halves)
        .unwrap()
        .unwrap();
    assert_eq!(repo.derivations(&ab_id).unwrap().len(), 2);

    fs::write(
        object_path(dir.path(), &ab_id),
        [b"B".as_slice(), &noise(6000, 1)].concat(),
    )
    .unwrap();
    assert_eq!(
        repo.read_content(&ab_id).unwrap(),
        ab,
        "healed through a derivation"
    );
    let bad: Vec<Id> = repo.fsck().unwrap().into_iter().map(|(id, _)| id).collect();
    assert_eq!(bad, [ab_id]);

    // A corrupt source with no derivation of its own cannot heal.
    fs::write(
        object_path(dir.path(), &a_id),
        [b"B".as_slice(), &noise(3000, 2)].concat(),
    )
    .unwrap();
    assert!(matches!(repo.read_content(&a_id), Err(Error::Corrupt(_))));
    assert!(repo.read_content(&ab_id).is_err());
}

/// Mutually recursive derivations must fail cleanly, not loop.
#[test]
fn derivation_cycles_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = Repo::new(FsStore::open(dir.path()).unwrap());
    let (x, y) = (b"x-part|y-part".to_vec(), b"y-part|x-part".to_vec());
    let (x_id, y_id) = (
        repo.put_content(&x).unwrap().id,
        repo.put_content(&y).unwrap().id,
    );
    let swap = |id| {
        vec![
            Arg::Range {
                id,
                start: 7,
                len: 6,
            },
            Arg::Range {
                id,
                start: 6,
                len: 1,
            },
            Arg::Range {
                id,
                start: 0,
                len: 6,
            },
        ]
    };
    repo.put_derivation(&x, func::concat(), swap(y_id))
        .unwrap()
        .unwrap();
    repo.put_derivation(&y, func::concat(), swap(x_id))
        .unwrap()
        .unwrap();
    fs::remove_file(object_path(dir.path(), &x_id)).unwrap();
    assert_eq!(
        repo.read_content(&x_id).unwrap(),
        x,
        "derived from stored y"
    );
    fs::remove_file(object_path(dir.path(), &y_id)).unwrap();
    assert!(matches!(repo.read_content(&x_id), Err(Error::Exec(_))));
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

fn commit_files<S: Store>(repo: &mut Repo<S>, files: &[Id]) {
    let entries = files
        .iter()
        .enumerate()
        .map(|(i, id)| TreeEntry {
            name: format!("f{i:03}"),
            kind: Kind::File,
            id: *id,
        })
        .collect();
    let tree = repo.put(&Object::tree(entries).unwrap()).unwrap();
    repo.commit("main", tree, "").unwrap();
}

#[test]
fn gc_keeps_exactly_what_refs_reach() {
    let mut repo = Repo::new(MemStore::default());
    let base = noise(5000, 13);
    let edited = [&base[..100], b"edit", &base[100..]].concat();
    repo.put_content(&base).unwrap();
    let derived = repo.put_content(&edited).unwrap();
    assert_eq!(derived.form, Form::Derived);
    let orphan = repo.put_content(b"not committed").unwrap().id;

    // Commit only the edited file: the content its derivation reads survives.
    commit_files(&mut repo, &[derived.id]);
    assert!(repo.gc().unwrap() >= 1);
    assert!(matches!(
        repo.read_content(&orphan),
        Err(Error::NotFound(_))
    ));
    assert_eq!(repo.read_content(&derived.id).unwrap(), edited);
    assert!(repo.fsck().unwrap().is_empty());
}

/// Ingest order should not decide storage size: repack re-plans the whole
/// store and is applied only if smaller and fully verified.
#[test]
fn repack_removes_ingest_order_dependence() {
    let mut versions = vec![noise(20_000, 17)];
    for i in 0..12u8 {
        let mut v = versions.last().unwrap().clone();
        let at = (i as usize * 1543) % v.len();
        v.splice(at..at, noise(200, i as u64 + 100));
        versions.push(v);
    }
    let size_after = |order: &[Vec<u8>]| {
        let mut repo = Repo::new(MemStore::default());
        let ids: Vec<Id> = order
            .iter()
            .map(|v| repo.put_content(v).unwrap().id)
            .collect();
        commit_files(&mut repo, &ids);
        let r = repo.repack().unwrap();
        for v in order {
            assert_eq!(&repo.read_content(&Id::of_content(v)).unwrap(), v);
        }
        assert!(repo.fsck().unwrap().is_empty());
        (r.before, r.after)
    };
    let reversed: Vec<Vec<u8>> = versions.iter().rev().cloned().collect();
    let (fwd_before, fwd_after) = size_after(&versions);
    let (rev_before, rev_after) = size_after(&reversed);
    assert!(fwd_after <= fwd_before && rev_after <= rev_before);
    assert_eq!(
        fwd_after, rev_after,
        "after repack, order no longer matters"
    );
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
