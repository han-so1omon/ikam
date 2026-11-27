//! `Edit::PutGraph`: a graph object at a path in a tree edit (E438).

use ikam_kernel::{
    Edge, Edit, Error, Id, Kind, MemStore, Node, Object, Repo, Store, Target, TreeEntry, node_key,
};

fn node(label: &str, to: &[&str]) -> Node {
    let mut n = Node {
        label: label.into(),
        target: Target::None,
        edges: to
            .iter()
            .map(|t| Edge {
                to: node_key(t),
                label: "next".into(),
            })
            .collect(),
    };
    n.edges.sort();
    n
}

/// A chain of `n` nodes, `v` in each label's suffix so versions differ.
fn chain(n: usize, changed: &[usize]) -> Vec<Node> {
    (0..n)
        .map(|i| {
            let next = format!("op/{:05}", i + 1);
            let mut x = node(&format!("op/{i:05}"), &[next.as_str()]);
            if changed.contains(&i) {
                x.edges.push(Edge {
                    to: node_key("op/00000"),
                    label: "back".into(),
                });
                x.edges.sort();
            }
            x
        })
        .collect()
}

/// The entry at `path` in commit `rev`'s tree.
fn entry_at<S: Store>(repo: &Repo<S>, rev: &str, path: &str) -> Option<TreeEntry> {
    let Object::Commit(c) = repo.get(&repo.resolve(rev).unwrap()).unwrap() else {
        panic!("not a commit")
    };
    let mut at = TreeEntry {
        name: String::new(),
        kind: Kind::Tree,
        id: c.tree,
    };
    for part in path.split('/') {
        let Object::Tree(entries) = repo.get(&at.id).ok()? else {
            return None;
        };
        at = entries.into_iter().find(|e| e.name == part)?;
    }
    Some(at)
}

fn tree_of<S: Store>(repo: &Repo<S>, rev: &str) -> Id {
    let Object::Commit(c) = repo.get(&repo.resolve(rev).unwrap()).unwrap() else {
        panic!("not a commit")
    };
    c.tree
}

#[test]
fn put_graph_commits_and_reads_back() {
    let mut repo = Repo::new(MemStore::default());
    let nodes = chain(3_000, &[]);
    let g = repo.put_graph(nodes.clone()).unwrap();
    assert!(
        matches!(
            repo.get(&g).unwrap(),
            Object::Graph(ikam_kernel::Graph::Children(..))
        ),
        "a graph of several chunks"
    );
    let readme = repo.put_content(b"notes").unwrap().id;
    let tree = repo
        .edit_tree(
            None,
            &[
                Edit::Put {
                    path: "README".into(),
                    id: readme,
                },
                Edit::PutGraph {
                    path: "models/solver/net".into(),
                    id: g,
                },
            ],
        )
        .unwrap();
    repo.commit("main", tree, "graph at a path").unwrap();

    let e = entry_at(&repo, "main", "models/solver/net").unwrap();
    assert_eq!((e.kind, e.id), (Kind::Graph, g));
    assert_eq!(repo.graph_nodes(&e.id).unwrap(), nodes);
    assert_eq!(entry_at(&repo, "main", "README").unwrap().kind, Kind::File);

    // Reachable from the commit: fsck is clean and gc keeps every chunk.
    assert!(repo.fsck().unwrap().is_empty());
    assert!(repo.fsck_deep().unwrap().is_empty());
    repo.gc().unwrap();
    assert_eq!(repo.graph_nodes(&g).unwrap(), nodes);

    // Copy and Rename carry a graph entry as they carry any other.
    let t2 = repo
        .edit_tree(
            Some(tree),
            &[
                Edit::Copy {
                    from: "models/solver/net".into(),
                    to: "models/copy".into(),
                },
                Edit::Rename {
                    from: "models/solver".into(),
                    to: "solver".into(),
                },
            ],
        )
        .unwrap();
    repo.commit("main", t2, "move").unwrap();
    assert_eq!(
        entry_at(&repo, "main", "models/copy").unwrap().kind,
        Kind::Graph
    );
    assert_eq!(entry_at(&repo, "main", "solver/net").unwrap().id, g);
    assert!(entry_at(&repo, "main", "models/solver").is_none());
}

#[test]
fn put_graph_diffs_against_the_previous_commit() {
    let mut repo = Repo::new(MemStore::default());
    let v1 = repo.put_graph(chain(2_000, &[])).unwrap();
    let data = repo.put_content(b"inputs").unwrap().id;
    let t1 = repo
        .edit_tree(
            None,
            &[
                Edit::PutGraph {
                    path: "m/net".into(),
                    id: v1,
                },
                Edit::Put {
                    path: "data/x".into(),
                    id: data,
                },
            ],
        )
        .unwrap();
    let c1 = repo.commit("main", t1, "v1").unwrap();

    let v2 = repo.put_graph(chain(2_000, &[7, 1_500])).unwrap();
    let t2 = repo
        .edit_tree(
            Some(t1),
            &[Edit::PutGraph {
                path: "m/net".into(),
                id: v2,
            }],
        )
        .unwrap();
    let c2 = repo.commit("main", t2, "v2").unwrap();
    assert_ne!(c1, c2);

    // The previous commit still reads v1; the new one reads v2.
    assert_eq!(entry_at(&repo, &c1.to_string(), "m/net").unwrap().id, v1);
    assert_eq!(entry_at(&repo, "main", "m/net").unwrap().id, v2);
    // The untouched subtree keeps its id.
    assert_eq!(
        entry_at(&repo, &c1.to_string(), "data").unwrap().id,
        entry_at(&repo, "main", "data").unwrap().id
    );
    // The graphs' diff names exactly the two changed nodes.
    let changes = repo.graph_diff(&v1, &v2).unwrap();
    let labels: Vec<&str> = changes.iter().map(|(l, ..)| l.as_str()).collect();
    assert_eq!(labels, ["op/00007", "op/01500"]);

    // Putting the same graph again changes nothing: the same tree id.
    let t3 = repo
        .edit_tree(
            Some(t2),
            &[Edit::PutGraph {
                path: "m/net".into(),
                id: v2,
            }],
        )
        .unwrap();
    assert_eq!(t3, t2);
    assert_eq!(tree_of(&repo, "main"), t2);
}

#[test]
fn put_graph_refuses_a_missing_or_wrong_id() {
    let mut repo = Repo::new(MemStore::default());
    let file = repo.put_content(b"not a graph").unwrap().id;
    let base = repo
        .edit_tree(
            None,
            &[Edit::Put {
                path: "a".into(),
                id: file,
            }],
        )
        .unwrap();
    let put = |path: &str, id: Id| Edit::PutGraph {
        path: path.into(),
        id,
    };

    let missing = Id::of(b"no such object");
    assert!(matches!(
        repo.edit_tree(Some(base), &[put("g", missing)]),
        Err(Error::NotFound(_))
    ));
    // Content and trees are not graphs.
    assert!(matches!(
        repo.edit_tree(Some(base), &[put("g", file)]),
        Err(Error::WrongKind(_))
    ));
    assert!(matches!(
        repo.edit_tree(Some(base), &[put("g", base)]),
        Err(Error::WrongKind(_))
    ));
    // A bad path is refused as `Put` refuses it.
    let g = repo.put_graph(chain(10, &[])).unwrap();
    assert!(matches!(
        repo.edit_tree(Some(base), &[put("x/../g", g)]),
        Err(Error::InvalidName(_))
    ));
    // A refused edit list writes no tree: a later good edit is as if alone.
    let good = repo.edit_tree(Some(base), &[put("g", g)]).unwrap();
    let after_refusal = repo
        .edit_tree(Some(base), &[put("g", g), put("h", missing)])
        .is_err();
    assert!(after_refusal);
    assert_eq!(repo.edit_tree(Some(base), &[put("g", g)]).unwrap(), good);
}

/// The compute crate's own helper (exp/430) wrote graph entries by hand;
/// `PutGraph` gives the same tree.
#[test]
fn put_graph_matches_a_hand_written_entry() {
    let mut repo = Repo::new(MemStore::default());
    let g = repo.put_graph(chain(50, &[])).unwrap();
    let f = repo.put_content(b"x").unwrap().id;
    let by_edit = repo
        .edit_tree(
            None,
            &[
                Edit::Put {
                    path: "f".into(),
                    id: f,
                },
                Edit::PutGraph {
                    path: "dir/g".into(),
                    id: g,
                },
            ],
        )
        .unwrap();
    let sub = repo
        .put(
            &Object::tree(vec![TreeEntry {
                name: "g".into(),
                kind: Kind::Graph,
                id: g,
            }])
            .unwrap(),
        )
        .unwrap();
    let by_hand = repo
        .put(
            &Object::tree(vec![
                TreeEntry {
                    name: "dir".into(),
                    kind: Kind::Tree,
                    id: sub,
                },
                TreeEntry {
                    name: "f".into(),
                    kind: Kind::File,
                    id: f,
                },
            ])
            .unwrap(),
        )
        .unwrap();
    assert_eq!(by_edit, by_hand);
}

/// Trees from the existing edits keep their ids: pinned on main 437aef6.
#[test]
fn existing_edits_keep_their_tree_ids() {
    let mut repo = Repo::new(MemStore::default());
    let a = repo.put_content(b"alpha").unwrap().id;
    let b = repo.put_content(b"beta").unwrap().id;
    let tree = repo
        .edit_tree(
            None,
            &[
                Edit::Put {
                    path: "x/a".into(),
                    id: a,
                },
                Edit::Put {
                    path: "x/y/b".into(),
                    id: b,
                },
                Edit::Mkdir { path: "e".into() },
                Edit::Copy {
                    from: "x".into(),
                    to: "z".into(),
                },
                Edit::Rename {
                    from: "z/y".into(),
                    to: "w".into(),
                },
                Edit::Remove { path: "x/a".into() },
            ],
        )
        .unwrap();
    assert_eq!(tree.to_string(), PINNED);
}

const PINNED: &str = "7eaa8d01982791426a612488b49d5552e386a263818702a9aef5a2e1d2c62f75";
