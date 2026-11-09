//! E144: with `object_templates` off, tree and view edits plan new chunks
//! only as slices of earlier content: the same roots, and no stored object
//! is rewritten or removed.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use ikam_kernel::{Edit, Error, Id, Keys, MemStore, Node, Repo, Store, Target};

#[derive(Default)]
struct Counting {
    inner: MemStore,
    rewrites: AtomicU64,
}

impl Store for Counting {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        self.inner.write(id, encoded)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.rewrites.fetch_add(1, Relaxed);
        self.inner.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
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
        self.rewrites.fetch_add(1, Relaxed);
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

/// Builds a view and a tree, edits both, and returns the new roots, the
/// stored objects rewritten or removed by the edits, and the bytes written.
fn run(templates: bool) -> (Id, Id, u64, usize) {
    let mut repo = Repo::new(Counting::default());
    repo.object_templates = templates;
    let blob = repo.put_content(b"a target").unwrap().id;
    let nodes: Vec<Node> = (0..3000).map(|i| node(i, blob)).collect();
    let view = repo.put_graph_with(nodes, Keys::Any).unwrap();
    let mut edits = Vec::new();
    for i in 0..300 {
        let id = repo
            .put_content(format!("file {i} body").as_bytes())
            .unwrap()
            .id;
        edits.push(Edit::Put {
            path: format!("dir/sub/file-{i:04}.txt"),
            id,
        });
    }
    let tree = repo.edit_tree(None, &edits).unwrap();
    repo.store().rewrites.store(0, Relaxed);
    let before = repo.bytes_written;
    let mut view = view;
    let mut tree = tree;
    for round in 0..4 {
        let ed: BTreeMap<String, Option<Node>> = (0..40)
            .map(|i| {
                let n = node(3000 + round * 40 + i, blob);
                (n.label.clone(), Some(n))
            })
            .chain([(node(round * 700, blob).label, None)])
            .collect();
        view = repo.graph_edit_with(&view, ed, Keys::Any).unwrap();
        let id = repo
            .put_content(format!("round {round} new body").as_bytes())
            .unwrap()
            .id;
        tree = repo
            .edit_tree(
                Some(tree),
                &[Edit::Put {
                    path: format!("dir/sub/file-{:04}.txt", round * 70),
                    id,
                }],
            )
            .unwrap();
    }
    assert!(repo.fsck().unwrap().is_empty());
    let rewrites = repo.store().rewrites.load(Relaxed);
    (view, tree, rewrites, repo.bytes_written - before)
}

#[test]
fn without_object_templates_edits_only_add() {
    let on = run(true);
    let off = run(false);
    assert_eq!((off.0, off.1), (on.0, on.1), "the roots are the same");
    assert_eq!(off.2, 0, "no stored object rewritten or removed");
    // With templates on, this edit replaces an old chunk with a fill of a
    // template, and writes more.
    assert!(on.2 > 0);
    assert!(
        off.3 < on.3,
        "{} bytes written without, {} with",
        off.3,
        on.3
    );
}
