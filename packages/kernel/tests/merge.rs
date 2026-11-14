//! Laws of `Repo::merge` (E265): files merge three-way by path, the
//! database by appending the other side's events since the fork; a key or
//! path written on both sides differently is a conflict and nothing is
//! written; the merged views equal a rebuild of the merged log.

use ikam_kernel::merge::{Conflict, Merged};
use ikam_kernel::{Db, Edit, Event, Id, MemStore, Object, Repo};

fn put(repo: &mut Repo<MemStore>, name: &str, edits: &[(&str, &str)]) {
    let root = repo.resolve(name).ok().map(|c| tree_of(repo, c));
    let edits: Vec<Edit> = edits
        .iter()
        .map(|(p, v)| match *v {
            "" => Edit::Remove {
                path: p.to_string(),
            },
            v => Edit::Put {
                path: p.to_string(),
                id: repo.put_content(v.as_bytes()).unwrap().id,
            },
        })
        .collect();
    let tree = repo.edit_tree(root, &edits).unwrap();
    repo.commit(name, tree, "files").unwrap();
}

fn append(repo: &mut Repo<MemStore>, name: &str, events: &[Event]) -> Db {
    let c = repo.resolve(name).unwrap();
    let db = repo.db_at(&c).unwrap();
    let db = repo.db_append(&db, events).unwrap();
    repo.db_commit(name, &db, "db").unwrap();
    db
}

fn tree_of(repo: &Repo<MemStore>, commit: Id) -> Id {
    match repo.get(&commit).unwrap() {
        Object::Commit(c) => c.tree,
        _ => unreachable!(),
    }
}

/// The file at `path` in the head of `name`, read by walking its trees.
fn read(repo: &Repo<MemStore>, name: &str, path: &str) -> Option<Vec<u8>> {
    let mut id = tree_of(repo, repo.resolve(name).unwrap());
    for part in path.split('/') {
        let Object::Tree(entries) = repo.get(&id).unwrap() else {
            return None;
        };
        id = entries.into_iter().find(|e| e.name == part)?.id;
    }
    repo.read_content(&id).ok()
}

/// A repo with `main` holding two files and three keys, and `side` at the
/// same commit.
fn forked() -> Repo<MemStore> {
    let mut repo = Repo::new(MemStore::default());
    put(&mut repo, "main", &[("a.txt", "a0"), ("d/b.txt", "b0")]);
    append(
        &mut repo,
        "main",
        &[
            Event::upsert("k/1", b"1", "e1"),
            Event::upsert("k/2", b"2", "e2"),
            Event::upsert("j/1", b"x", "e3"),
        ],
    );
    let head = repo.resolve("main").unwrap();
    repo.branch("side", head).unwrap();
    repo
}

fn kv(repo: &mut Repo<MemStore>, name: &str) -> Vec<(String, Vec<u8>)> {
    let c = repo.resolve(name).unwrap();
    let db = repo.db_at(&c).unwrap();
    assert_eq!(
        repo.db_rebuild(&db.log).unwrap(),
        db,
        "views equal a rebuild"
    );
    repo.db_scan(&db).unwrap()
}

#[test]
fn disjoint_changes_merge() {
    let mut repo = forked();
    put(&mut repo, "main", &[("a.txt", "a1")]);
    append(&mut repo, "main", &[Event::upsert("k/1", b"one", "m1")]);
    put(&mut repo, "side", &[("d/c.txt", "c1")]);
    append(
        &mut repo,
        "side",
        &[
            Event::upsert("k/3", b"3", "s1"),
            Event::remove("j", true, "s2"),
        ],
    );
    let out = repo.merge("main", "side", "merge").unwrap();
    assert!(matches!(out, Merged::Commit(_)), "{out:?}");
    assert_eq!(read(&repo, "main", "a.txt").unwrap(), b"a1");
    assert_eq!(read(&repo, "main", "d/b.txt").unwrap(), b"b0");
    assert_eq!(read(&repo, "main", "d/c.txt").unwrap(), b"c1");
    let got = kv(&mut repo, "main");
    let want: Vec<(String, Vec<u8>)> = [("k/1", "one"), ("k/2", "2"), ("k/3", "3")]
        .iter()
        .map(|(k, v)| (k.to_string(), v.as_bytes().to_vec()))
        .collect();
    assert_eq!(got, want);
    // Both heads are parents, and a second merge changes nothing.
    assert_eq!(
        repo.merge("main", "side", "again").unwrap(),
        Merged::UpToDate
    );
}

#[test]
fn conflicts_write_nothing() {
    let mut repo = forked();
    put(&mut repo, "main", &[("a.txt", "mine")]);
    append(&mut repo, "main", &[Event::upsert("k/1", b"mine", "m1")]);
    put(&mut repo, "side", &[("a.txt", "theirs")]);
    append(
        &mut repo,
        "side",
        &[
            Event::upsert("k/1", b"theirs", "s1"),
            Event::upsert("k/9", b"9", "m1"),
        ],
    );
    let head = repo.resolve("main").unwrap();
    let Merged::Conflicts(c) = repo.merge("main", "side", "merge").unwrap() else {
        panic!("expected conflicts");
    };
    assert!(c.contains(&Conflict::Path("a.txt".into())), "{c:?}");
    assert!(c.contains(&Conflict::Key("k/1".into())), "{c:?}");
    assert!(c.contains(&Conflict::Idempotency("m1".into())), "{c:?}");
    assert_eq!(repo.resolve("main").unwrap(), head, "ref not moved");
}

#[test]
fn a_subtree_remove_clashes_with_a_key_below_it() {
    let mut repo = forked();
    append(&mut repo, "main", &[Event::remove("k", true, "m1")]);
    append(&mut repo, "side", &[Event::upsert("k/7", b"7", "s1")]);
    let Merged::Conflicts(c) = repo.merge("main", "side", "merge").unwrap() else {
        panic!("expected a conflict");
    };
    assert_eq!(c, vec![Conflict::Key("k/7".into())]);
    // A range remove on one side against a key inside it, the other way.
    let mut repo = forked();
    append(&mut repo, "main", &[Event::upsert("k/15", b"x", "m1")]);
    append(
        &mut repo,
        "side",
        &[Event::remove_range("k/1", "k/2", "s1")],
    );
    assert!(matches!(
        repo.merge("main", "side", "merge").unwrap(),
        Merged::Conflicts(_)
    ));
}

#[test]
fn the_same_change_on_both_sides_is_not_a_conflict() {
    let mut repo = forked();
    for side in ["main", "side"] {
        put(&mut repo, side, &[("a.txt", "same")]);
        append(&mut repo, side, &[Event::upsert("k/5", b"5", "retry")]);
    }
    append(&mut repo, "main", &[Event::upsert("k/6", b"6", "m-own")]);
    append(&mut repo, "side", &[Event::upsert("k/6", b"6", "s-own")]);
    assert!(matches!(
        repo.merge("main", "side", "merge").unwrap(),
        Merged::Commit(_)
    ));
    let c = repo.resolve("main").unwrap();
    let db = repo.db_at(&c).unwrap();
    // The retry is logged once; the second write of the same value is
    // its own event.
    let keys: Vec<String> = repo
        .db_events(&db)
        .unwrap()
        .into_iter()
        .map(|(_, e)| e.idempotency_key)
        .collect();
    assert_eq!(keys.iter().filter(|k| *k == "retry").count(), 1);
    assert_eq!(read(&repo, "main", "a.txt").unwrap(), b"same");
    kv(&mut repo, "main");
}

#[test]
fn ancestors_fast_forward_or_change_nothing() {
    let mut repo = forked();
    append(&mut repo, "side", &[Event::upsert("k/8", b"8", "s1")]);
    let side = repo.resolve("side").unwrap();
    assert_eq!(
        repo.merge("main", "side", "m").unwrap(),
        Merged::FastForward(side)
    );
    assert_eq!(repo.merge("side", "main", "m").unwrap(), Merged::UpToDate);
}
