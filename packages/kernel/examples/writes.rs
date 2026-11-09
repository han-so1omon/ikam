//! E129: what small writes to a large store read, with `needed_index` off
//! and on.
//!
//! `cargo run --release --example writes -- SRC_STORE REF`
//!
//! Loads the store at SRC_STORE (an `FsStore`, e.g. the bench corpus
//! ingested) into memory behind a counting wrapper, then applies writes of
//! growing size to the tree at REF, each committed, and keeps a reverse-link
//! view ("target/source/label", as in E125) current after each. Prints one
//! JSON line per write and setting: objects read, bytes fetched, full
//! scans and milliseconds, for the write and for the view's update.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use ikam_kernel::{
    Edit, Error, FsStore, Id, Keys, Kind, MemStore, Node, Object, Repo, Store, Target,
};

struct Counting<S: Store> {
    inner: S,
    reads: AtomicU64,
    bytes: AtomicU64,
    scans: AtomicU64,
}

#[derive(Clone, Copy, Default)]
struct Io {
    reads: u64,
    bytes: u64,
    scans: u64,
}

impl<S: Store> Counting<S> {
    fn take(&self) -> Io {
        Io {
            reads: self.reads.swap(0, Relaxed),
            bytes: self.bytes.swap(0, Relaxed),
            scans: self.scans.swap(0, Relaxed),
        }
    }
}

impl<S: Store> Store for Counting<S> {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        self.inner.write(id, encoded)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.inner.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        let out = self.inner.read(id)?;
        self.reads.fetch_add(1, Relaxed);
        self.bytes.fetch_add(out.len() as u64, Relaxed);
        Ok(out)
    }
    fn has(&self, id: &Id) -> bool {
        self.inner.has(id)
    }
    fn ids(&self) -> Result<Vec<Id>, Error> {
        self.scans.fetch_add(1, Relaxed);
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

type R = Repo<Counting<MemStore>>;

/// Files written by one case: paths and their new bytes.
type Writes = Vec<(String, Vec<u8>)>;

fn load(src: &Path) -> Counting<MemStore> {
    let fs = FsStore::open(src).unwrap();
    let mut mem = MemStore::default();
    for id in fs.ids().unwrap() {
        mem.write(id, &fs.read(&id).unwrap()).unwrap();
    }
    for (name, id) in fs.refs().unwrap() {
        mem.set_ref(&name, None, id).unwrap();
    }
    Counting {
        inner: mem,
        reads: 0.into(),
        bytes: 0.into(),
        scans: 0.into(),
    }
}

fn files(repo: &R, tree: Id, prefix: &str, out: &mut Vec<(String, Id)>) {
    let Object::Tree(entries) = repo.get(&tree).unwrap() else {
        panic!("not a tree")
    };
    for e in entries {
        let path = if prefix.is_empty() {
            e.name.clone()
        } else {
            format!("{prefix}/{}", e.name)
        };
        match e.kind {
            Kind::Tree => files(repo, e.id, &path, out),
            _ => out.push((path, e.id)),
        }
    }
}

fn reverse_edges(repo: &R, ids: &[Id]) -> Vec<Node> {
    let mut rev = Vec::new();
    for src in ids {
        if let Ok(obj) = repo.get(src) {
            for (label, to) in obj.links() {
                rev.push(Node {
                    label: format!("{to}/{src}/{label}"),
                    target: Target::Object(*src),
                    edges: vec![],
                });
            }
        }
    }
    rev
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (src, refname) = (Path::new(&a[0]), a[1].as_str());
    for on in [false, true] {
        let mut repo: R = Repo::new(load(src));
        repo.needed_index = on;
        let commit = repo.resolve(refname).unwrap();
        let Object::Commit(c) = repo.get(&commit).unwrap() else {
            panic!()
        };
        let mut all = Vec::new();
        files(&repo, c.tree, "", &mut all);
        // The first content write on a fresh handle builds the matcher's
        // seed index from every stored object.
        repo.store().take();
        let t = Instant::now();
        let doc: Vec<u8> = (0..400)
            .flat_map(|i| {
                format!("line {i} of a new note written on a fresh handle\n").into_bytes()
            })
            .collect();
        repo.put_content(&doc).unwrap();
        let first = repo.store().take();
        println!(
            "{{\"exp\":\"E129\",\"needed_index\":{on},\"write\":\"first_write_on_a_fresh_handle\",\"bytes\":{},\"reads\":{},\"fetched\":{},\"scans\":{},\"ms\":{:.1}}}",
            doc.len(),
            first.reads,
            first.bytes,
            first.scans,
            t.elapsed().as_secs_f64() * 1e3
        );
        let objects = repo.store().ids().unwrap();
        let mut rev = reverse_edges(&repo, &objects);
        rev.sort_by(|a, b| a.label.cmp(&b.label));
        rev.dedup_by(|a, b| a.label == b.label);
        let mut view = repo.put_graph_with(rev, Keys::Any).unwrap();
        let mut known: HashSet<Id> = repo.store().ids().unwrap().into_iter().collect();
        let md: Vec<&(String, Id)> = all.iter().filter(|f| f.0.ends_with(".md")).collect();
        let small: Vec<&(String, Id)> = all
            .iter()
            .filter(|f| f.0.starts_with("l-construction-v01/"))
            .collect();
        let mut fam = BTreeMap::new();
        for f in &all {
            let ext = f.0.rsplit_once('.').map_or("", |x| x.1).to_string();
            fam.entry(ext).or_insert(f);
        }
        let cases: Vec<(&str, Writes)> = vec![
            (
                "one_md_file",
                vec![(md[3].0.clone(), b"# Edited\n\nOne line changed.\n".to_vec())],
            ),
            (
                "one_file_per_type",
                fam.values()
                    .map(|f| {
                        let mut b = repo.read_content(&f.1).unwrap();
                        b.extend_from_slice(b"\n");
                        (f.0.clone(), b)
                    })
                    .collect(),
            ),
            (
                "new_folder",
                small
                    .iter()
                    .map(|f| {
                        let mut b = repo.read_content(&f.1).unwrap();
                        b.extend_from_slice(b"\n");
                        (format!("e129-copy/{}", f.0), b)
                    })
                    .collect(),
            ),
        ];
        let mut root = c.tree;
        repo.store().take();
        for (name, writes) in cases {
            let t = Instant::now();
            let mut edits = Vec::new();
            for (path, bytes) in &writes {
                let id = repo.put_content(bytes).unwrap().id;
                edits.push(Edit::Put {
                    path: path.clone(),
                    id,
                });
            }
            root = repo.edit_tree(Some(root), &edits).unwrap();
            repo.commit(refname, root, name).unwrap();
            let w = repo.store().take();
            let w_ms = t.elapsed().as_secs_f64() * 1e3;
            let new: Vec<Id> = repo
                .store()
                .ids()
                .unwrap()
                .into_iter()
                .filter(|i| !known.contains(i))
                .collect();
            known.extend(new.iter().copied());
            repo.store().take();
            let t = Instant::now();
            let added = reverse_edges(&repo, &new);
            let n = added.len();
            let ed: BTreeMap<String, Option<Node>> = added
                .into_iter()
                .map(|n| (n.label.clone(), Some(n)))
                .collect();
            view = repo.graph_edit_with(&view, ed, Keys::Any).unwrap();
            let v = repo.store().take();
            let v_ms = t.elapsed().as_secs_f64() * 1e3;
            known.extend(repo.store().ids().unwrap());
            repo.store().take();
            println!(
                "{{\"exp\":\"E129\",\"needed_index\":{on},\"write\":\"{name}\",\"files\":{},\"store_objects\":{},\"write_reads\":{},\"write_fetched\":{},\"write_scans\":{},\"write_ms\":{w_ms:.1},\"new_objects\":{},\"new_edges\":{n},\"view_reads\":{},\"view_fetched\":{},\"view_scans\":{},\"view_ms\":{v_ms:.1}}}",
                writes.len(),
                known.len(),
                w.reads,
                w.bytes,
                w.scans,
                new.len(),
                v.reads,
                v.bytes,
                v.scans
            );
        }
        assert!(repo.fsck().unwrap().is_empty());
        for (p, id) in &all {
            let _ = p;
            repo.read_content(id).unwrap();
        }
    }
}
