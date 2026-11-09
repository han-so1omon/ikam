//! E138: `read_memo` keeps what one edit reads, and changes nothing else.

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

/// Builds a view and a tree, then edits both on one handle, with the memo
/// on or off. Returns the new roots, the bytes written by the edits, the
/// edits' store reads, and every stored object.
type Run = (Id, Id, usize, u64, Vec<(Id, Vec<u8>)>);

fn run(memo: bool) -> Run {
    let mut repo = Repo::new(Counting::default());
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
    repo.read_memo = memo;
    repo.store().reads.store(0, Relaxed);
    let before = repo.bytes_written;
    let ed: BTreeMap<String, Option<Node>> = (3000..3060)
        .chain([10, 900, 2100])
        .map(|i| {
            let n = node(i * 3 % 3100, blob);
            (n.label.clone(), Some(n))
        })
        .collect();
    let view = repo.graph_edit_with(&view, ed, Keys::Any).unwrap();
    let id = repo.put_content(b"one new file").unwrap().id;
    let tree = repo
        .edit_tree(
            Some(tree),
            &[Edit::Put {
                path: "dir/sub/file-0150.txt".into(),
                id,
            }],
        )
        .unwrap();
    let written = repo.bytes_written - before;
    let reads = repo.store().reads.load(Relaxed);
    assert!(repo.fsck().unwrap().is_empty());
    let mut objects: Vec<(Id, Vec<u8>)> = repo
        .store()
        .ids()
        .unwrap()
        .into_iter()
        .map(|i| (i, repo.store().inner.read(&i).unwrap()))
        .collect();
    objects.sort();
    (view, tree, written, reads, objects)
}

#[test]
fn the_memo_stores_the_same_bytes_with_fewer_reads() {
    let off = run(false);
    let on = run(true);
    assert_eq!((on.0, on.1, on.2), (off.0, off.1, off.2));
    assert!(on.4 == off.4, "stored objects differ");
    assert!(
        on.3 < off.3,
        "{} reads with the memo, {} without",
        on.3,
        off.3
    );
}

#[test]
fn the_memo_is_dropped_when_the_edit_returns() {
    let mut repo = Repo::new(MemStore::default());
    repo.read_memo = true;
    let id = repo.put_content(b"some bytes").unwrap().id;
    repo.edit_tree(
        None,
        &[Edit::Put {
            path: "a".into(),
            id,
        }],
    )
    .unwrap();
    // Nothing is kept between edits: a store changed outside the handle
    // is read as it is.
    assert_eq!(repo.read_content(&id).unwrap(), b"some bytes");
}
