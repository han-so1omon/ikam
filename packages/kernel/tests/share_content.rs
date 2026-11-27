//! E437: a records share that also carries the content its records name.

use ikam_kernel::merge::Merged;
use ikam_kernel::share::{Refused, Right, Scope, Share, import_share, named_content};
use ikam_kernel::{Edit, Event, Id, MemStore, Object, Repo, Store};

const KEY: [u8; 32] = [7; 32];

fn bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s as u8
        })
        .collect()
}

fn records(start: &str, end: &str, content: bool) -> Scope {
    Scope::Records {
        start: start.into(),
        end: Some(end.into()),
        content,
    }
}

fn share(commit: Id, scope: Scope) -> Share {
    Share {
        name: "runs".into(),
        from: "a".into(),
        to: "b".into(),
        commit,
        scope,
        right: Right::Merge,
        expires: 2_000,
    }
}

/// What the source holds, by role in the test.
struct Source {
    repo: Repo<MemStore>,
    commit: Id,
    /// Content named by records inside `runs/`.
    named: Vec<(Id, Vec<u8>)>,
    /// Content that is in the store but only named outside the scope, or
    /// not named at all.
    outside: Vec<Id>,
    /// A named id that is not in the store.
    missing: Id,
    /// A tree a record names.
    tree: Id,
}

fn source() -> Source {
    let mut a = Repo::new(MemStore::default());
    let small = b"a state of 16 floats, encoded".to_vec();
    let big = bytes(1, 300_000); // several chunks
    let hexed = bytes(2, 5_000);
    let out = b"content named only outside the scope".to_vec();
    let private = b"a private file".to_vec();
    let [small_id, big_id, hex_id, out_id, private_id] =
        [&small, &big, &hexed, &out, &private].map(|b| a.put_content(b).unwrap().id);
    let t = a
        .edit_tree(
            None,
            &[
                Edit::Put {
                    path: "priv/file".into(),
                    id: private_id,
                },
                Edit::Put {
                    path: "pub/inputs".into(),
                    id: small_id,
                },
            ],
        )
        .unwrap();
    a.commit("main", t, "files").unwrap();
    let missing = Id::of_content(b"never stored");
    let events = vec![
        Event::upsert("runs/r1/0000/state", small_id.as_bytes(), "1"),
        Event::upsert("runs/r1/0001/state", big_id.as_bytes(), "2"),
        Event::upsert("runs/r1/0002/state", hex_id.to_hex().as_bytes(), "3"),
        // The same content named twice is carried once.
        Event::upsert("runs/r1/0003/state", small_id.as_bytes(), "4"),
        Event::upsert("runs/r1/0000/params", b"load 1.5", "5"),
        // Names that carry nothing: not stored, a tree, not a whole id.
        Event::upsert("runs/r1/0004/state", missing.as_bytes(), "6"),
        Event::upsert("runs/r1/tree", t.as_bytes(), "7"),
        Event::upsert(
            "runs/r1/note",
            format!("see {}", out_id.to_hex()).as_bytes(),
            "8",
        ),
        Event::upsert(
            "runs/r1/upper",
            out_id.to_hex().to_uppercase().as_bytes(),
            "9",
        ),
        // Outside the scope's key range.
        Event::upsert("lab/x/state", out_id.as_bytes(), "10"),
    ];
    let db = a.db_empty().unwrap();
    let db = a.db_append(&db, &events).unwrap();
    let commit = a.db_commit("main", &db, "records").unwrap();
    Source {
        repo: a,
        commit,
        named: vec![(small_id, small), (big_id, big), (hex_id, hexed)],
        outside: vec![out_id, private_id],
        missing,
        tree: t,
    }
}

fn objects<S: Store>(r: &Repo<S>) -> usize {
    r.store().ids().unwrap().len()
}

/// Import into a fresh receiver with or without the option.
fn import(content: bool) -> (Source, Repo<MemStore>, Share) {
    let mut src = source();
    let s = share(src.commit, records("runs/", "runs0", content));
    let tag = src.repo.share_issue(&s, &KEY).unwrap();
    src.repo
        .share_check(&s, &tag, &KEY, "b", Right::Merge, 1_000)
        .unwrap();
    let mut b = Repo::new(MemStore::default());
    let m = import_share(&mut src.repo, &s, &mut b, "main", "merge runs").unwrap();
    assert!(matches!(m, Merged::FastForward(_)));
    (src, b, s)
}

#[test]
fn named_content_is_a_whole_id() {
    let id = Id::of_content(b"x");
    assert_eq!(named_content(id.as_bytes()), Some(id));
    assert_eq!(named_content(id.to_hex().as_bytes()), Some(id));
    assert_eq!(named_content(id.to_hex().to_uppercase().as_bytes()), None);
    assert_eq!(named_content(&id.as_bytes()[..31]), None);
    assert_eq!(named_content(format!(" {}", id.to_hex()).as_bytes()), None);
}

#[test]
fn the_option_encodes_as_its_own_kind_and_off_is_unchanged() {
    let c = Id::of_content(b"commit");
    let off = share(c, records("a", "b", false));
    let on = share(c, records("a", "b", true));
    assert_eq!(Share::decode(&on.encode()), Some(on.clone()));
    assert_eq!(Share::decode(&off.encode()), Some(off.clone()));
    // The two differ only in the scope's kind byte (3 as before, 5).
    let (eo, en) = (off.encode(), on.encode());
    let diff: Vec<usize> = (0..eo.len()).filter(|&i| eo[i] != en[i]).collect();
    assert_eq!(diff.len(), 1);
    assert_eq!((eo[diff[0]], en[diff[0]]), (3, 5));
    assert_ne!(off.seal(&KEY), on.seal(&KEY));
}

#[test]
fn with_content_every_named_content_resolves_and_nothing_else_comes() {
    let (mut src, mut b, s) = import(true);
    let (_, mut plain, _) = import(false);
    // Every content a scoped record names resolves in the receiver.
    for (id, want) in &src.named {
        assert_eq!(&b.read_content(id).unwrap(), want);
    }
    let db = b.db_at(&b.resolve("main").unwrap()).unwrap();
    let mut resolved = 0;
    let mut dangling = 0;
    for (_, v) in b.db_scan(&db).unwrap() {
        if let Some(id) = named_content(&v) {
            if b.has_content(&id).unwrap() {
                resolved += 1;
            } else {
                dangling += 1;
            }
        }
    }
    // 4 records name stored content (one twice); the missing id and the
    // tree are not content at the source, so they stay unresolved.
    assert_eq!((resolved, dangling), (4, 2));
    // Nothing outside the scope: not the content named only by `lab/`, not
    // the private file, not the tree a record names, not the missing id.
    for id in &src.outside {
        assert!(!b.has_content(id).unwrap());
        assert!(!b.store().has(id));
    }
    assert!(!b.has_content(&src.missing).unwrap());
    let Object::Commit(c) = src.repo.get(&src.commit).unwrap() else {
        unreachable!()
    };
    assert!(!b.store().has(&c.tree));
    assert!(read_tree(&b, "priv").is_none());
    // Only the records' range came: no `lab/` key.
    assert!(b.db_prefix(&db, "lab/", 10, false).unwrap().is_empty());
    // The content sits at content/<id>, so it is reachable: fsck is clean
    // and gc finds nothing to drop, on both sides.
    let ids: Vec<String> = read_tree(&b, "content").unwrap();
    let mut want: Vec<String> = src.named.iter().map(|(i, _)| i.to_hex()).collect();
    want.sort();
    assert_eq!(ids, want);
    assert!(b.fsck().unwrap().is_empty());
    assert!(b.fsck_deep().unwrap().is_empty());
    assert!(src.repo.fsck().unwrap().is_empty());
    assert!(src.repo.fsck_deep().unwrap().is_empty());
    // gc drops what any records import leaves (as on main), nothing more.
    let plain_garbage = plain.gc().unwrap();
    let garbage = b.gc().unwrap();
    assert_eq!(garbage, plain_garbage);
    for (id, want) in &src.named {
        assert_eq!(&b.read_content(id).unwrap(), want);
    }
    // Objects: what the option adds over a plain import is the named
    // content's own objects and the trees that hold them.
    let content_objects = {
        let mut only = Repo::new(MemStore::default());
        for (_, bytes) in &src.named {
            only.put_content(bytes).unwrap();
        }
        objects(&only)
    };
    let extra = objects(&b) - objects(&plain);
    assert!(
        extra >= content_objects && extra <= content_objects + 3,
        "extra {extra}, content {content_objects}"
    );
    eprintln!(
        "E437 objects: plain import {}, with content {}, content alone {}, gc dropped {garbage} both",
        objects(&plain),
        objects(&b),
        content_objects
    );
    // Reads through the share answer only named content.
    assert_eq!(
        src.repo.share_content(&s, &src.named[1].0).unwrap(),
        src.named[1].1
    );
    for id in src.outside.iter().chain([&src.missing, &src.tree]) {
        assert_eq!(src.repo.share_content(&s, id), Err(Refused::Outside));
    }
    // A merge's size counts the content.
    let rec_bytes: usize = src
        .repo
        .share_records(&s, "", None)
        .unwrap()
        .iter()
        .map(|(k, v)| k.len() + v.len())
        .sum();
    let content_bytes: usize = src.named.iter().map(|(_, b)| b.len()).sum();
    assert_eq!(
        src.repo.share_import_bytes(&s).unwrap(),
        rec_bytes + content_bytes
    );
}

#[test]
fn without_content_nothing_named_comes_and_reads_refuse() {
    let (mut src, b, s) = import(false);
    for (id, _) in &src.named {
        assert!(!b.has_content(id).unwrap(), "dangling, as on main");
    }
    assert!(read_tree(&b, "content").is_none());
    assert!(b.fsck().unwrap().is_empty());
    assert_eq!(
        src.repo.share_content(&s, &src.named[0].0),
        Err(Refused::Outside)
    );
}

/// The import without the option makes the same objects and commits as
/// main 437aef6 did (pinned from a run there).
#[test]
fn without_content_the_import_is_as_on_main() {
    let (_, b, s) = import(false);
    let head = b.resolve("main").unwrap();
    let mut ids: Vec<String> = b
        .store()
        .ids()
        .unwrap()
        .iter()
        .map(|i| i.to_hex())
        .collect();
    ids.sort();
    let all = blake3::hash(ids.join("\n").as_bytes()).to_hex().to_string();
    let share = blake3::hash(&s.encode()).to_hex().to_string();
    assert_eq!([head.to_hex(), all, share], MAIN_PINS);
    assert_eq!(ids.len(), 28);
}

const MAIN_PINS: [&str; 3] = [
    "58c2042ad0e2f3db7c057701bbf1a77f1255caa23295fc824035fbd722857dda",
    "7486cd65fd1ceed96d8cfd832083fa69d838c981c220e8406fbc3a0ea61f27c1",
    "69f716ecf8f399f2113342297fd9fc30a83fb2076298b0136a5e46a12ffc7f8b",
];

#[test]
fn a_later_import_drops_content_no_record_names() {
    let (mut src, mut b, s) = import(true);
    let db = src.repo.db_at(&src.commit).unwrap();
    let db = src
        .repo
        .db_append(&db, &[Event::remove("runs/r1/0001/state", false, "11")])
        .unwrap();
    let c2 = src.repo.db_commit("main", &db, "drop a step").unwrap();
    let s2 = Share { commit: c2, ..s };
    src.repo.share_issue(&s2, &KEY).unwrap();
    import_share(&mut src.repo, &s2, &mut b, "main", "again").unwrap();
    let mut want: Vec<String> = [&src.named[0], &src.named[2]]
        .iter()
        .map(|(i, _)| i.to_hex())
        .collect();
    want.sort();
    assert_eq!(read_tree(&b, "content").unwrap(), want);
    assert!(b.fsck().unwrap().is_empty());
}

/// Names in the directory at `path` of `main`'s tree.
fn read_tree(r: &Repo<MemStore>, path: &str) -> Option<Vec<String>> {
    let Object::Commit(c) = r.get(&r.resolve("main").ok()?).ok()? else {
        return None;
    };
    let mut id = c.tree;
    for part in path.split('/') {
        let Object::Tree(es) = r.get(&id).ok()? else {
            return None;
        };
        id = es.into_iter().find(|e| e.name == part)?.id;
    }
    let Object::Tree(es) = r.get(&id).ok()? else {
        return None;
    };
    Some(es.into_iter().map(|e| e.name).collect())
}
