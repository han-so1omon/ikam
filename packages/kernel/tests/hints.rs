//! E141: `replace_hints` indexes the old chunks an edit reads, so a scope
//! of only the edited files stores about what a whole-store index does.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use ikam_kernel::{Edit, Error, Id, Keys, MemStore, Node, Repo, Store, Target};

#[derive(Default)]
struct Counting {
    inner: MemStore,
    reads: AtomicU64,
}

impl Store for Counting {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        self.inner.write(id, encoded)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.inner.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        self.reads.fetch_add(1, Relaxed);
        self.inner.read(id)
    }
    fn has(&self, id: &Id) -> bool {
        self.inner.has(id)
    }
    fn ids(&self) -> Result<Vec<Id>, Error> {
        self.inner.ids()
    }
    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        self.inner.ids_with_prefix(prefix)
    }
    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        self.inner.delete(id)
    }
    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        self.inner.get_ref(name)
    }
    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        self.inner.set_ref(name, expected, new)
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        self.inner.refs()
    }
}

fn node(i: usize, target: Id) -> Node {
    Node {
        label: format!(
            "note-{i:05}/links/to/a/fairly/long/label/{}",
            i * 7919 % 10007
        ),
        target: Target::Object(target),
        edges: vec![],
    }
}

/// A store with a view of 3000 nodes and a tree of 300 files, as a new
/// handle opens it, and the view, tree and target ids.
fn base() -> (MemStore, Id, Id, Id) {
    let mut repo = Repo::new(MemStore::default());
    let blob = repo.put_content(b"a target").unwrap().id;
    // Each node points at its own content, so chunks hold ids that do not
    // compress and a new chunk is cheapest as slices of the old one.
    let nodes: Vec<Node> = (0..3000)
        .map(|i| {
            let t = repo
                .put_content(format!("target {i}").as_bytes())
                .unwrap()
                .id;
            node(i, t)
        })
        .collect();
    let view = repo.put_graph_with(nodes, Keys::Any).unwrap();
    let mut edits = Vec::new();
    for i in 0..300 {
        let id = repo
            .put_content(format!("file {i} body, with some words to repeat").as_bytes())
            .unwrap()
            .id;
        edits.push(Edit::Put {
            path: format!("dir/sub/file-{i:04}.txt"),
            id,
        });
    }
    let tree = repo.edit_tree(None, &edits).unwrap();
    let mut store = MemStore::default();
    for id in repo.store().ids().unwrap() {
        store.write(id, &repo.store().read(&id).unwrap()).unwrap();
    }
    (store, view, tree, blob)
}

/// Edits the view and the tree on a new handle; bytes written and reads.
fn run(scope: Option<Vec<Id>>, hints: bool) -> (Id, Id, usize, u64) {
    let (store, view, tree, blob) = base();
    let mut repo = Repo::new(Counting {
        inner: store,
        reads: 0.into(),
    });
    repo.read_memo = true;
    repo.replace_hints = hints;
    let id = repo.put_content(b"one new file").unwrap().id;
    let scope = scope.map(|mut s| {
        s.push(id);
        s
    });
    repo.set_seed_scope(scope);
    repo.store().reads.store(0, Relaxed);
    let before = repo.bytes_written;
    // New nodes, and three old ones pointed at the new file.
    let ed: BTreeMap<String, Option<Node>> = (3000..3060)
        .map(|i| node(i, blob))
        .chain([10, 900, 2100].map(|i| node(i, id)))
        .map(|n| (n.label.clone(), Some(n)))
        .collect();
    let view = repo.graph_edit_with(&view, ed, Keys::Any).unwrap();
    let tree = repo
        .edit_tree(
            Some(tree),
            &[Edit::Put {
                path: "dir/sub/file-0150.txt".into(),
                id,
            }],
        )
        .unwrap();
    assert!(repo.fsck().unwrap().is_empty());
    (
        view,
        tree,
        repo.bytes_written - before,
        repo.store().reads.load(Relaxed),
    )
}

#[test]
fn hints_let_a_narrow_scope_store_what_the_whole_store_does() {
    let whole = run(None, false);
    let narrow = run(Some(vec![]), false);
    let hinted = run(Some(vec![]), true);
    // The same trees and views whatever the scope.
    assert_eq!((hinted.0, hinted.1), (whole.0, whole.1));
    assert_eq!((narrow.0, narrow.1), (whole.0, whole.1));
    assert!(
        hinted.2 * 2 < narrow.2,
        "{} bytes with hints, {} without",
        hinted.2,
        narrow.2
    );
    assert!(
        hinted.2 * 10 <= whole.2 * 11,
        "{} bytes with hints, {} with the whole store indexed",
        hinted.2,
        whole.2
    );
    assert!(
        hinted.3 < whole.3,
        "{} reads with hints, {} with the whole store indexed",
        hinted.3,
        whole.3
    );
}

#[test]
fn hints_do_nothing_without_a_scope() {
    assert_eq!(run(None, true), run(None, false));
}
