//! Laws of `Repo::merge` (E265): files merge three-way by path, the
//! database by appending the other side's events since the fork; a key or
//! path written on both sides differently is a conflict and nothing is
//! written; the merged views equal a rebuild of the merged log.

#![allow(clippy::single_range_in_vec_init)] // a conflict on one range of lines is the usual case

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

// E417: line-level merge, an option (`MergeOptions::lines`).

use ikam_kernel::merge::{MergeOptions, merge_lines};

const LINES: MergeOptions = MergeOptions { lines: true };

fn text(lines: &[&str]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

/// `main` and `side` forked at a commit holding `f.txt` = `base`.
fn forked_file(base: &str) -> Repo<MemStore> {
    let mut repo = Repo::new(MemStore::default());
    put(&mut repo, "main", &[("f.txt", base)]);
    let c = repo.resolve("main").unwrap();
    repo.branch("side", c).unwrap();
    repo
}

#[test]
fn line_merge_keeps_edits_to_different_lines() {
    let base = text(&["one", "two", "three", "four", "five", "six"]);
    let mut repo = forked_file(&base);
    put(
        &mut repo,
        "main",
        &[(
            "f.txt",
            &text(&["ONE", "two", "three", "four", "five", "six"]),
        )],
    );
    put(
        &mut repo,
        "side",
        &[(
            "f.txt",
            &text(&["one", "two", "three", "four", "FIVE", "six", "seven"]),
        )],
    );
    // Off (the default): any file changed on both sides is a conflict.
    let head = repo.resolve("main").unwrap();
    assert_eq!(
        repo.merge("main", "side", "m").unwrap(),
        Merged::Conflicts(vec![Conflict::Path("f.txt".into())])
    );
    assert_eq!(repo.resolve("main").unwrap(), head);
    // On: both edits are kept.
    assert!(matches!(
        repo.merge_with("main", "side", "m", LINES).unwrap(),
        Merged::Commit(_)
    ));
    assert_eq!(
        read(&repo, "main", "f.txt").unwrap(),
        text(&["ONE", "two", "three", "four", "FIVE", "six", "seven"]).as_bytes()
    );
    assert!(repo.fsck().unwrap().is_empty());
}

#[test]
fn line_merge_refuses_edits_to_the_same_lines() {
    let base = text(&["one", "two", "three", "four"]);
    let mut repo = forked_file(&base);
    put(
        &mut repo,
        "main",
        &[("f.txt", &text(&["one", "TWO", "three", "four"]))],
    );
    put(
        &mut repo,
        "side",
        &[("f.txt", &text(&["one", "2", "three", "4"]))],
    );
    let head = repo.resolve("main").unwrap();
    assert_eq!(
        repo.merge_with("main", "side", "m", LINES).unwrap(),
        Merged::Conflicts(vec![Conflict::Lines {
            path: "f.txt".into(),
            lines: vec![1..2],
        }])
    );
    assert_eq!(repo.resolve("main").unwrap(), head);
}

/// A file that merges cleanly is not stored when another file conflicts:
/// a refused merge writes none of its merged contents.
#[test]
fn line_merge_stores_nothing_when_another_file_conflicts() {
    let base = text(&["one", "two", "three", "four"]);
    let mut repo = Repo::new(MemStore::default());
    put(&mut repo, "main", &[("f.txt", &base), ("g.txt", &base)]);
    let c = repo.resolve("main").unwrap();
    repo.branch("side", c).unwrap();
    put(
        &mut repo,
        "main",
        &[
            ("f.txt", &text(&["ONE", "two", "three", "four"])),
            ("g.txt", &text(&["one", "TWO", "three", "four"])),
        ],
    );
    put(
        &mut repo,
        "side",
        &[
            ("f.txt", &text(&["one", "two", "three", "FOUR"])),
            ("g.txt", &text(&["one", "2", "three", "four"])),
        ],
    );
    let merged_f = text(&["ONE", "two", "three", "FOUR"]);
    let id = Id::of_content(merged_f.as_bytes());
    // The empty database object is the one thing a merge may write first
    // (once per store); write it now so only the merge's own bytes count.
    repo.db_empty().unwrap();
    let written = repo.bytes_written;
    assert!(matches!(
        repo.merge_with("main", "side", "m", LINES).unwrap(),
        Merged::Conflicts(_)
    ));
    assert!(!repo.has_content(&id).unwrap());
    assert_eq!(repo.bytes_written, written);
}

#[test]
fn merge_lines_cases() {
    let b = text(&["a", "b", "c", "d"]);
    let ok =
        |o: &[&str], t: &[&str]| merge_lines(b.as_bytes(), text(o).as_bytes(), text(t).as_bytes());
    let clean = |o: &[&str], t: &[&str], want: &[&str]| {
        assert_eq!(ok(o, t), Some(Ok(text(want).into_bytes())), "{o:?} {t:?}");
    };
    // Adjacent lines changed on each side merge.
    clean(
        &["A", "b", "c", "d"],
        &["a", "B", "c", "d"],
        &["A", "B", "c", "d"],
    );
    // The same change on both sides is kept once.
    clean(
        &["a", "X", "c", "d"],
        &["a", "X", "c", "D"],
        &["a", "X", "c", "D"],
    );
    // A deletion on one side, an edit elsewhere on the other.
    clean(&["a", "c", "d"], &["a", "b", "c", "D"], &["a", "c", "D"]);
    // An insertion beside a line the other side changed.
    clean(
        &["a", "new", "b", "c", "d"],
        &["a", "B", "c", "d"],
        &["a", "new", "B", "c", "d"],
    );
    // Insertions at the same place: a guess at their order, so refused.
    assert_eq!(
        ok(&["a", "x", "b", "c", "d"], &["a", "y", "b", "c", "d"]),
        Some(Err(vec![1..1]))
    );
    // An insertion inside a range the other side replaced.
    assert_eq!(
        ok(&["a", "b", "x", "c", "d"], &["a", "B", "C", "d"]),
        Some(Err(vec![1..3]))
    );
    // A deleted line the other side changed.
    assert_eq!(
        ok(&["a", "c", "d"], &["a", "B", "c", "d"]),
        Some(Err(vec![1..2]))
    );
    // A last line without a newline.
    assert_eq!(
        merge_lines(b"a\nb\nc", b"A\nb\nc", b"a\nb\nC"),
        Some(Ok(b"A\nb\nC".to_vec()))
    );
    // Bytes with a NUL are not text.
    assert_eq!(merge_lines(b"a\0", b"b\0", b"c\0"), None);
}

#[test]
fn line_merge_leaves_binary_and_added_files_as_conflicts() {
    let mut repo = forked_file("x\0y\n");
    put(
        &mut repo,
        "main",
        &[("f.txt", "x\0Y\n"), ("n.txt", "ours\n")],
    );
    put(
        &mut repo,
        "side",
        &[("f.txt", "X\0y\n"), ("n.txt", "theirs\n")],
    );
    assert_eq!(
        repo.merge_with("main", "side", "m", LINES).unwrap(),
        Merged::Conflicts(vec![
            Conflict::Path("f.txt".into()),
            Conflict::Path("n.txt".into())
        ])
    );
}
