//! Laws of scoped shares between separate projects (E315 to E318): each
//! project is its own store; a share returns only bytes of its scope, stops
//! at once when revoked, replaced or expired, and a merge copies its scope
//! into the receiver's own store, three-way against the last merge.

#![allow(clippy::single_range_in_vec_init)] // a grant of one range is the usual case

use ikam_kernel::merge::{Conflict, Merged};
use ikam_kernel::share::{Refused, Right, Scope, Share, import_share};
use ikam_kernel::{
    Arg, Edge, Edit, Event, Id, Kind, MemStore, Node, Object, Repo, Target, TreeEntry,
};

const KEY: [u8; 32] = [7; 32];
const NOW: u64 = 1_000;

fn put(repo: &mut Repo<MemStore>, name: &str, edits: &[(&str, &str)]) -> Id {
    let root = repo.resolve(name).ok().map(|c| tree(repo, c));
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
    let t = repo.edit_tree(root, &edits).unwrap();
    repo.commit(name, t, "files").unwrap()
}

fn tree(repo: &Repo<MemStore>, commit: Id) -> Id {
    match repo.get(&commit).unwrap() {
        Object::Commit(c) => c.tree,
        _ => unreachable!(),
    }
}

fn read(repo: &Repo<MemStore>, name: &str, path: &str) -> Option<Vec<u8>> {
    let mut id = tree(repo, repo.resolve(name).unwrap());
    for part in path.split('/') {
        let Object::Tree(entries) = repo.get(&id).unwrap() else {
            return None;
        };
        id = entries.into_iter().find(|e| e.name == part)?.id;
    }
    Some(repo.read_content(&id).unwrap())
}

fn share(name: &str, commit: Id, scope: Scope, right: Right) -> Share {
    Share {
        name: name.into(),
        from: "a".into(),
        to: "b".into(),
        commit,
        scope,
        right,
        expires: NOW + 60,
    }
}

/// Project "a" with public and private files.
fn source() -> (Repo<MemStore>, Id) {
    let mut a = Repo::new(MemStore::default());
    let c = put(
        &mut a,
        "main",
        &[
            ("pub/one.md", "alpha shared line\nsecond line\n"),
            ("pub/two.md", "beta\n"),
            ("pubx/near.md", "alpha in a sibling folder\n"),
            ("priv/secret.md", "alpha private line\n"),
        ],
    );
    (a, c)
}

#[test]
fn checks_seal_project_expiry_and_log() {
    let (mut a, c) = source();
    let s = share("s1", c, Scope::Path("pub".into()), Right::Read);
    let tag = a.share_issue(&s, &KEY).unwrap();
    assert_eq!(a.share_check(&s, &tag, &KEY, "b", Right::Read, NOW), Ok(()));
    assert_eq!(
        a.share_check(&s, &tag, &[8; 32], "b", Right::Read, NOW),
        Err(Refused::Seal)
    );
    assert_eq!(
        a.share_check(&s, &tag, &KEY, "c", Right::Read, NOW),
        Err(Refused::Project)
    );
    assert_eq!(
        a.share_check(&s, &tag, &KEY, "b", Right::Read, NOW + 60),
        Err(Refused::Expired)
    );
    assert_eq!(
        a.share_check(&s, &tag, &KEY, "b", Right::Merge, NOW),
        Err(Refused::Right)
    );
    let mut forged = s.clone();
    forged.scope = Scope::Path("".into());
    assert_eq!(
        a.share_check(&forged, &tag, &KEY, "b", Right::Read, NOW),
        Err(Refused::Seal)
    );
    // An unlogged share with a valid seal (issued elsewhere) does not check.
    let other = share("s2", c, Scope::Path("pub".into()), Right::Read);
    let tag2 = other.seal(&KEY);
    assert_eq!(
        a.share_check(&other, &tag2, &KEY, "b", Right::Read, NOW),
        Err(Refused::Revoked)
    );
    // Replacing a share under the same name retires the old one.
    let wider = share("s1", c, Scope::Path("".into()), Right::Read);
    let tag3 = a.share_issue(&wider, &KEY).unwrap();
    assert_eq!(
        a.share_check(&s, &tag, &KEY, "b", Right::Read, NOW),
        Err(Refused::Revoked)
    );
    assert_eq!(
        a.share_check(&wider, &tag3, &KEY, "b", Right::Read, NOW),
        Ok(())
    );
    assert!(a.share_revoke("s1").unwrap());
    assert_eq!(
        a.share_check(&wider, &tag3, &KEY, "b", Right::Read, NOW),
        Err(Refused::Revoked)
    );
    assert!(!a.share_revoke("s1").unwrap());
    assert!(a.shares().unwrap().is_empty());
}

#[test]
fn path_scope_reads_only_its_prefix() {
    let (a, c) = source();
    let s = share("s", c, Scope::Path("pub".into()), Right::Read);
    let files: Vec<String> = a
        .share_files(&s)
        .unwrap()
        .into_iter()
        .map(|f| f.0)
        .collect();
    assert_eq!(files, ["pub/one.md", "pub/two.md"]);
    assert_eq!(a.share_read(&s, "pub/two.md", None).unwrap(), b"beta\n");
    assert_eq!(
        a.share_read(&s, "pub/one.md", Some(0..5)).unwrap(),
        b"alpha"
    );
    for p in [
        "priv/secret.md",
        "pubx/near.md",
        "pub",
        "pub/missing.md",
        "pub/../priv/secret.md",
    ] {
        assert!(a.share_read(&s, p, None).is_err(), "{p}");
    }
}

#[test]
fn share_is_pinned_to_its_commit() {
    let (mut a, c) = source();
    let s = share("s", c, Scope::Path("pub".into()), Right::Read);
    put(
        &mut a,
        "main",
        &[("pub/two.md", "changed later\n"), ("pub/three.md", "new\n")],
    );
    assert_eq!(a.share_read(&s, "pub/two.md", None).unwrap(), b"beta\n");
    assert!(a.share_read(&s, "pub/three.md", None).is_err());
}

#[test]
fn section_scope_reads_only_inside_its_ranges() {
    let (mut a, c) = source();
    let id = a
        .put_content(b"alpha shared line\nsecond line\n")
        .unwrap()
        .id;
    let s = share(
        "s",
        c,
        Scope::Section {
            path: "pub/one.md".into(),
            id,
            ranges: vec![6..17],
        },
        Right::Read,
    );
    assert_eq!(
        a.share_read(&s, "pub/one.md", Some(6..17)).unwrap(),
        b"shared line"
    );
    assert_eq!(a.share_read(&s, "pub/one.md", Some(7..10)).unwrap(), b"har");
    for r in [5..17, 6..18, 0..30, 10..10] {
        assert_eq!(
            a.share_read(&s, "pub/one.md", Some(r.clone())),
            Err(Refused::Outside),
            "{r:?}"
        );
    }
    assert_eq!(a.share_read(&s, "pub/one.md", None), Err(Refused::Outside));
    assert_eq!(
        a.share_read(&s, "pub/two.md", Some(0..2)),
        Err(Refused::Outside)
    );
    // A grant for another version of the file reads nothing.
    let other = a.put_content(b"some other version").unwrap().id;
    let stale = share(
        "t",
        c,
        Scope::Section {
            path: "pub/one.md".into(),
            id: other,
            ranges: vec![0..5],
        },
        Right::Read,
    );
    assert_eq!(
        a.share_read(&stale, "pub/one.md", Some(0..5)),
        Err(Refused::Outside)
    );
}

#[test]
fn fragment_scope_reads_only_listed_fragments() {
    let (mut a, c) = source();
    let f1 = a.put_content(b"a fragment to share").unwrap().id;
    let f2 = a.put_content(b"a fragment kept private").unwrap().id;
    let s = share("s", c, Scope::Fragments(vec![f1]), Right::Read);
    assert_eq!(a.share_fragment(&s, &f1).unwrap(), b"a fragment to share");
    assert_eq!(a.share_fragment(&s, &f2), Err(Refused::Outside));
    assert!(a.share_read(&s, "pub/one.md", None).is_err());
}

fn with_records(a: &mut Repo<MemStore>) -> Id {
    let db = a.db_at(&a.resolve("main").unwrap()).unwrap();
    let events: Vec<Event> = ["notes/1", "notes/2", "orders/1", "people/1"]
        .iter()
        .map(|k| Event::upsert(k, format!("value of {k} alpha").as_bytes(), k))
        .collect();
    let db = a.db_append(&db, &events).unwrap();
    a.db_commit("main", &db, "records").unwrap()
}

#[test]
fn records_scope_clips_every_range() {
    let (mut a, _) = source();
    let c = with_records(&mut a);
    let s = share(
        "s",
        c,
        Scope::Records {
            start: "notes/".into(),
            end: Some("notes0".into()),
        },
        Right::Read,
    );
    let keys = |rows: Vec<(String, Vec<u8>)>| rows.into_iter().map(|r| r.0).collect::<Vec<_>>();
    assert_eq!(
        keys(a.share_records(&s, "", None).unwrap()),
        ["notes/1", "notes/2"]
    );
    assert_eq!(
        keys(a.share_records(&s, "notes/2", None).unwrap()),
        ["notes/2"]
    );
    assert!(
        a.share_records(&s, "orders/", Some("orders0"))
            .unwrap()
            .is_empty()
    );
    assert!(a.share_records(&s, "a", Some("b")).unwrap().is_empty());
}

#[test]
fn graph_scope_clips_to_its_prefix() {
    let (mut a, c0) = source();
    let node = |l: &str| Node {
        label: l.into(),
        target: Target::None,
        edges: vec![],
    };
    let g = a
        .put_graph(vec![node("x/1"), node("x/2"), node("y/1")])
        .unwrap();
    let Object::Tree(mut entries) = a.get(&tree(&a, c0)).unwrap() else {
        unreachable!()
    };
    entries.push(TreeEntry {
        name: "g".into(),
        kind: Kind::Graph,
        id: g,
    });
    let t = a.put(&Object::tree(entries).unwrap()).unwrap();
    let c = a.commit("main", t, "graph").unwrap();
    let s = share(
        "s",
        c,
        Scope::Graph {
            path: "g".into(),
            prefix: "x/".into(),
        },
        Right::Read,
    );
    let labels = |n: Vec<Node>| n.into_iter().map(|n| n.label).collect::<Vec<_>>();
    assert_eq!(labels(a.share_nodes(&s, "").unwrap()), ["x/1", "x/2"]);
    assert_eq!(labels(a.share_nodes(&s, "x/2").unwrap()), ["x/2"]);
    assert!(a.share_nodes(&s, "y/").unwrap().is_empty());
}

#[test]
fn search_never_finds_outside_the_scope() {
    let (mut a, c) = source();
    let s = share("s", c, Scope::Path("pub".into()), Right::Read);
    assert_eq!(a.share_search(&s, b"alpha").unwrap(), ["pub/one.md"]);
    assert!(a.share_search(&s, b"private").unwrap().is_empty());
    assert!(a.share_search(&s, b"sibling").unwrap().is_empty());
    let id = a
        .put_content(b"alpha shared line\nsecond line\n")
        .unwrap()
        .id;
    let sec = share(
        "t",
        c,
        Scope::Section {
            path: "pub/one.md".into(),
            id,
            ranges: vec![6..17],
        },
        Right::Read,
    );
    assert_eq!(a.share_search(&sec, b"shared").unwrap(), ["pub/one.md"]);
    assert!(a.share_search(&sec, b"alpha").unwrap().is_empty());
    assert!(a.share_search(&sec, b"second").unwrap().is_empty());
    let c2 = with_records(&mut a);
    let rec = share(
        "r",
        c2,
        Scope::Records {
            start: "notes/".into(),
            end: Some("notes0".into()),
        },
        Right::Read,
    );
    assert_eq!(
        a.share_search(&rec, b"alpha").unwrap(),
        ["notes/1", "notes/2"]
    );
    assert!(a.share_search(&rec, b"orders").unwrap().is_empty());
}

#[test]
fn proposals_land_on_a_branch_in_the_source() {
    let (mut a, c) = source();
    let s = share("p", c, Scope::Path("pub".into()), Right::Propose);
    assert_eq!(
        a.share_propose(&s, &[("priv/secret.md".into(), b"x".to_vec())], "m"),
        Err(Refused::Outside)
    );
    let r = share("r", c, Scope::Path("pub".into()), Right::Read);
    assert_eq!(
        a.share_propose(&r, &[("pub/two.md".into(), b"x".to_vec())], "m"),
        Err(Refused::Right)
    );
    a.share_propose(
        &s,
        &[("pub/two.md".into(), b"beta, edited\n".to_vec())],
        "edit",
    )
    .unwrap();
    assert_eq!(read(&a, "main", "pub/two.md").unwrap(), b"beta\n");
    assert_eq!(
        read(&a, "proposals/p", "pub/two.md").unwrap(),
        b"beta, edited\n"
    );
    assert_eq!(
        a.merge("main", "proposals/p", "take it").unwrap(),
        Merged::FastForward(a.resolve("proposals/p").unwrap())
    );
    assert_eq!(read(&a, "main", "pub/two.md").unwrap(), b"beta, edited\n");
    // A section proposal may change only its ranges.
    let old = b"alpha shared line\nsecond line\n";
    let id = a.put_content(old).unwrap().id;
    let sec = share(
        "q",
        c,
        Scope::Section {
            path: "pub/one.md".into(),
            id,
            ranges: vec![6..17],
        },
        Right::Propose,
    );
    let inside = b"alpha SHARED LINE\nsecond line\n".to_vec();
    let outside = b"ALPHA shared line\nsecond line\n".to_vec();
    assert_eq!(
        a.share_propose(&sec, &[("pub/one.md".into(), outside)], "m"),
        Err(Refused::Outside)
    );
    a.share_propose(&sec, &[("pub/one.md".into(), inside.clone())], "m")
        .unwrap();
    assert_eq!(read(&a, "proposals/q", "pub/one.md").unwrap(), inside);
}

#[test]
fn merge_copies_a_path_scope_and_merges_again_three_way() {
    let (mut a, c) = source();
    let mut b = Repo::new(MemStore::default());
    put(&mut b, "main", &[("own.md", "b's own file\n")]);
    let s = share("m", c, Scope::Path("pub".into()), Right::Merge);
    a.share_issue(&s, &KEY).unwrap();
    assert!(matches!(
        import_share(&mut a, &s, &mut b, "main", "first").unwrap(),
        Merged::Commit(_)
    ));
    assert_eq!(
        read(&b, "main", "pub/one.md").unwrap(),
        b"alpha shared line\nsecond line\n"
    );
    assert_eq!(read(&b, "main", "own.md").unwrap(), b"b's own file\n");
    assert!(read(&b, "main", "priv/secret.md").is_none());
    let note = match b.get(&b.resolve("import/a/m").unwrap()).unwrap() {
        Object::Commit(c) => c.message,
        _ => unreachable!(),
    };
    assert!(note.contains(&format!("import a share m at {}", c.to_hex())));
    // The source changes, adds and removes; the receiver edits its own file.
    let c2 = put(
        &mut a,
        "main",
        &[
            ("pub/one.md", "alpha v2\n"),
            ("pub/two.md", ""),
            ("pub/new.md", "n\n"),
        ],
    );
    put(&mut b, "main", &[("own.md", "b edited\n")]);
    let s2 = share("m", c2, Scope::Path("pub".into()), Right::Merge);
    a.share_issue(&s2, &KEY).unwrap();
    assert!(matches!(
        import_share(&mut a, &s2, &mut b, "main", "again").unwrap(),
        Merged::Commit(_)
    ));
    assert_eq!(read(&b, "main", "pub/one.md").unwrap(), b"alpha v2\n");
    assert!(read(&b, "main", "pub/two.md").is_none());
    assert_eq!(read(&b, "main", "pub/new.md").unwrap(), b"n\n");
    assert_eq!(read(&b, "main", "own.md").unwrap(), b"b edited\n");
    // Both sides change the same imported file: a conflict, nothing written.
    let head = put(&mut b, "main", &[("pub/one.md", "b's take\n")]);
    let c3 = put(&mut a, "main", &[("pub/one.md", "a's take\n")]);
    let s3 = share("m", c3, Scope::Path("pub".into()), Right::Merge);
    assert_eq!(
        import_share(&mut a, &s3, &mut b, "main", "third").unwrap(),
        Merged::Conflicts(vec![Conflict::Path("pub/one.md".into())])
    );
    assert_eq!(b.resolve("main").unwrap(), head);
    assert!(b.fsck().unwrap().is_empty());
    // Revoking stops reads and merges; what was merged stays.
    a.share_revoke("m").unwrap();
    assert_eq!(
        a.share_check(&s2, &s2.seal(&KEY), &KEY, "b", Right::Merge, NOW),
        Err(Refused::Revoked)
    );
    assert_eq!(read(&b, "main", "pub/new.md").unwrap(), b"n\n");
}

#[test]
fn merge_copies_records_incrementally() {
    let (mut a, _) = source();
    let c = with_records(&mut a);
    let scope = || Scope::Records {
        start: "notes/".into(),
        end: Some("notes0".into()),
    };
    let mut b = Repo::new(MemStore::default());
    let s = share("r", c, scope(), Right::Merge);
    import_share(&mut a, &s, &mut b, "main", "records").unwrap();
    let rows = |b: &mut Repo<MemStore>| {
        let db = b.db_at(&b.resolve("main").unwrap()).unwrap();
        b.db_scan(&db)
            .unwrap()
            .into_iter()
            .map(|r| r.0)
            .collect::<Vec<_>>()
    };
    assert_eq!(rows(&mut b), ["notes/1", "notes/2"]);
    let db = a.db_at(&c).unwrap();
    let db = a
        .db_append(
            &db,
            &[
                Event::remove("notes/1", false, "d1"),
                Event::upsert("notes/3", b"v", "u3"),
            ],
        )
        .unwrap();
    let c2 = a.db_commit("main", &db, "more").unwrap();
    let s2 = share("r", c2, scope(), Right::Merge);
    import_share(&mut a, &s2, &mut b, "main", "records again").unwrap();
    assert_eq!(rows(&mut b), ["notes/2", "notes/3"]);
    assert!(b.fsck().unwrap().is_empty());
}

#[test]
fn a_read_share_does_not_merge() {
    let (mut a, c) = source();
    let mut b = Repo::new(MemStore::default());
    let read_share = share("x", c, Scope::Path("pub".into()), Right::Read);
    assert_eq!(
        import_share(&mut a, &read_share, &mut b, "main", "m"),
        Err(Refused::Right)
    );
    assert!(b.resolve("main").is_err());
}

#[test]
fn section_and_fragment_scopes_merge_only_their_bytes() {
    // E365: a section import is the granted ranges only; again three-way.
    let (mut a, c) = source();
    let mut b = Repo::new(MemStore::default());
    put(&mut b, "main", &[("own.md", "b's own\n")]);
    let file = a
        .put_content(b"alpha shared line\nsecond line\n")
        .unwrap()
        .id;
    let section = |c: Id, id: Id, ranges: Vec<std::ops::Range<u64>>| {
        let scope = Scope::Section {
            path: "pub/one.md".into(),
            id,
            ranges,
        };
        share("s", c, scope, Right::Merge)
    };
    // Two touching ranges and one apart: "alpha shared" and "second".
    let s = section(c, file, vec![6..12, 0..6, 18..24]);
    import_share(&mut a, &s, &mut b, "main", "sections").unwrap();
    assert_eq!(
        read(&b, "main", "pub/one.md").unwrap(),
        b"alpha sharedsecond"
    );
    assert_eq!(read(&b, "main", "own.md").unwrap(), b"b's own\n");
    assert!(read(&b, "main", "pub/two.md").is_none());
    // The source edits inside the section; the receiver's file follows.
    let c2 = put(
        &mut a,
        "main",
        &[("pub/one.md", "ALPHA shared line\nsecond line\n")],
    );
    let file2 = a
        .put_content(b"ALPHA shared line\nsecond line\n")
        .unwrap()
        .id;
    let s2 = section(c2, file2, vec![0..12]);
    assert!(matches!(
        import_share(&mut a, &s2, &mut b, "main", "again").unwrap(),
        Merged::FastForward(_) | Merged::Commit(_)
    ));
    assert_eq!(read(&b, "main", "pub/one.md").unwrap(), b"ALPHA shared");
    // A section of a file version the commit does not have is refused.
    let stale = section(c2, file, vec![0..6]);
    assert_eq!(
        import_share(&mut a, &stale, &mut b, "main", "stale"),
        Err(Refused::Outside)
    );
    // A section may not land in the database's own entry.
    let db_path = Scope::Section {
        path: "db/one.md".into(),
        id: file2,
        ranges: vec![0..6],
    };
    assert_eq!(
        import_share(
            &mut a,
            &share("d", c2, db_path, Right::Merge),
            &mut b,
            "main",
            "m"
        ),
        Err(Refused::NotMergeable)
    );
    // Fragments land under fragments/, by id.
    let f = a.put_content(b"frag bytes").unwrap().id;
    let frag = share("f", c, Scope::Fragments(vec![f]), Right::Merge);
    import_share(&mut a, &frag, &mut b, "main", "fragments").unwrap();
    assert_eq!(
        read(&b, "main", &format!("fragments/{}", f.to_hex())).unwrap(),
        b"frag bytes"
    );
    assert!(b.fsck().unwrap().is_empty());
}

#[test]
fn graph_scope_merges_its_nodes_without_other_bytes() {
    // E365: a graph import holds the scope's nodes with their bytes copied
    // (a range as its own content), and edges only among them.
    let (mut a, c0) = source();
    let blob = a.put_content(b"inside|OUTSIDE").unwrap().id;
    let whole = a.put_content(b"whole node").unwrap().id;
    let edge = |to: &str| Edge {
        to: ikam_kernel::node_key(to),
        label: "rel".into(),
    };
    let nodes = vec![
        Node {
            label: "x/1".into(),
            target: Target::Content(Arg::Range {
                id: blob,
                start: 0,
                len: 6,
            }),
            edges: vec![edge("x/2"), edge("y/1")],
        },
        Node {
            label: "x/2".into(),
            target: Target::Content(Arg::Whole(whole)),
            edges: vec![],
        },
        Node {
            label: "y/1".into(),
            target: Target::Content(Arg::Range {
                id: blob,
                start: 7,
                len: 7,
            }),
            edges: vec![],
        },
    ];
    let graph_commit = |a: &mut Repo<MemStore>, nodes: Vec<Node>| {
        let g = a.put_graph(nodes).unwrap();
        let root = tree(a, a.resolve("main").unwrap());
        let Object::Tree(mut entries) = a.get(&root).unwrap() else {
            unreachable!()
        };
        entries.retain(|e| e.name != "kg");
        let sub = a
            .put(
                &Object::tree(vec![TreeEntry {
                    name: "g".into(),
                    kind: Kind::Graph,
                    id: g,
                }])
                .unwrap(),
            )
            .unwrap();
        entries.push(TreeEntry {
            name: "kg".into(),
            kind: Kind::Tree,
            id: sub,
        });
        let t = a.put(&Object::tree(entries).unwrap()).unwrap();
        a.commit("main", t, "graph").unwrap()
    };
    let _ = c0;
    let c = graph_commit(&mut a, nodes.clone());
    let scope = || Scope::Graph {
        path: "kg/g".into(),
        prefix: "x/".into(),
    };
    let mut b = Repo::new(MemStore::default());
    put(&mut b, "main", &[("own.md", "b's own\n")]);
    let s = share("g", c, scope(), Right::Merge);
    import_share(&mut a, &s, &mut b, "main", "graph").unwrap();
    let got = |b: &Repo<MemStore>| {
        let root = tree(b, b.resolve("main").unwrap());
        let Object::Tree(es) = b.get(&root).unwrap() else {
            unreachable!()
        };
        let kg = es.iter().find(|e| e.name == "kg").unwrap().id;
        let Object::Tree(es) = b.get(&kg).unwrap() else {
            unreachable!()
        };
        let g = es.iter().find(|e| e.name == "g").unwrap();
        assert_eq!(g.kind, Kind::Graph);
        b.graph_nodes(&g.id).unwrap()
    };
    let ns = got(&b);
    assert_eq!(
        ns.iter().map(|n| n.label.as_str()).collect::<Vec<_>>(),
        ["x/1", "x/2"]
    );
    let bytes = |b: &Repo<MemStore>, n: &Node| match &n.target {
        Target::Content(Arg::Whole(id)) => b.read_content(id).unwrap(),
        t => panic!("{t:?}"),
    };
    assert_eq!(bytes(&b, &ns[0]), b"inside");
    assert_eq!(bytes(&b, &ns[1]), b"whole node");
    assert_eq!(ns[0].edges, vec![edge("x/2")], "no edge out of the scope");
    // Nothing outside the scope reached b's store.
    assert!(b.read_content(&blob).is_err());
    assert_eq!(read(&b, "main", "own.md").unwrap(), b"b's own\n");
    // The source changes a node in the scope; importing again follows it.
    let mut nodes2 = nodes.clone();
    nodes2[1].target = Target::Content(Arg::Whole(a.put_content(b"v2").unwrap().id));
    let c2 = graph_commit(&mut a, nodes2);
    let s2 = share("g", c2, scope(), Right::Merge);
    import_share(&mut a, &s2, &mut b, "main", "graph again").unwrap();
    assert_eq!(bytes(&b, &got(&b)[1]), b"v2");
    // Nor may a graph land in the database's own entry.
    let in_db = Scope::Graph {
        path: "db/g".into(),
        prefix: "x/".into(),
    };
    assert_eq!(
        import_share(
            &mut a,
            &share("g", c2, in_db, Right::Merge),
            &mut b,
            "main",
            "m"
        ),
        Err(Refused::NotMergeable)
    );
    // A node pointing at another object is not in the scope: refused.
    let mut nodes3 = nodes.clone();
    nodes3[0].target = Target::Object(c);
    let c3 = graph_commit(&mut a, nodes3);
    let s3 = share("g", c3, scope(), Right::Merge);
    assert_eq!(
        import_share(&mut a, &s3, &mut b, "main", "m"),
        Err(Refused::NotMergeable)
    );
    assert!(b.fsck().unwrap().is_empty());
}

#[test]
fn shares_decode_from_the_log() {
    let (mut a, c) = source();
    let id = a.put_content(b"x").unwrap().id;
    let scopes = [
        Scope::Path("pub".into()),
        Scope::Section {
            path: "pub/one.md".into(),
            id,
            ranges: vec![1..3, 5..9],
        },
        Scope::Fragments(vec![id, c]),
        Scope::Records {
            start: "a".into(),
            end: None,
        },
        Scope::Records {
            start: "a".into(),
            end: Some("b".into()),
        },
        Scope::Graph {
            path: "g".into(),
            prefix: "x/".into(),
        },
    ];
    for (i, scope) in scopes.into_iter().enumerate() {
        let s = share(
            &format!("s{i}"),
            c,
            scope,
            [Right::Read, Right::Propose, Right::Merge][i % 3],
        );
        assert_eq!(Share::decode(&s.encode()), Some(s.clone()));
        a.share_issue(&s, &KEY).unwrap();
        assert_eq!(a.share_named(&s.name).unwrap(), Some(s));
    }
    let bytes = share("t", c, Scope::Path("p".into()), Right::Read).encode();
    for n in 0..bytes.len() {
        assert_eq!(Share::decode(&bytes[..n]), None);
    }
    assert_eq!(Share::decode(&[bytes.as_slice(), b"x"].concat()), None);
    assert_eq!(a.share_named("missing").unwrap(), None);
}
