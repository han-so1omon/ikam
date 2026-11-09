//! E132: one- and two-byte searches through the trigram index.
//!
//! `cargo run --release --example short -- SRC_STORE REF`
//!
//! Copies the store into memory, builds a trigram index over the tree's text
//! files, and for each query prints scan and indexed costs (objects read,
//! bytes fetched, ms) and the files matched, which must be the same.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use ikam_kernel::{Error, FsStore, Id, Kind, MemStore, Object, Repo, Store};

struct Counting<S: Store> {
    inner: S,
    reads: AtomicU64,
    bytes: AtomicU64,
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

const TEXT: &[&str] = &[
    "md", "txt", "csv", "json", "html", "htm", "xml", "yaml", "yml", "py", "rs", "js", "ts", "tsv",
    "toml", "sql", "eml", "log",
];

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (src, refname) = (Path::new(&a[0]), a[1].as_str());
    let fs = FsStore::open(src).unwrap();
    let mut mem = MemStore::default();
    for id in fs.ids().unwrap() {
        mem.write(id, &fs.read(&id).unwrap()).unwrap();
    }
    for (name, id) in fs.refs().unwrap() {
        mem.set_ref(&name, None, id).unwrap();
    }
    let mut repo: R = Repo::new(Counting {
        inner: mem,
        reads: 0.into(),
        bytes: 0.into(),
    });
    let commit = repo.resolve(refname).unwrap();
    let Object::Commit(c) = repo.get(&commit).unwrap() else {
        panic!()
    };
    let mut all = Vec::new();
    files(&repo, c.tree, "", &mut all);
    let text: Vec<Id> = all
        .iter()
        .filter(|f| {
            f.0.rsplit_once('.')
                .is_some_and(|(_, e)| TEXT.contains(&e.to_lowercase().as_str()))
        })
        .map(|f| f.1)
        .collect();
    let text_bytes: usize = text
        .iter()
        .map(|id| repo.read_content(id).unwrap().len())
        .sum();
    let t = Instant::now();
    let index_id = repo.build_search_index(&text).unwrap();
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    let index_bytes = repo.read_content(&index_id).unwrap().len();
    println!(
        "{{\"exp\":\"E132\",\"text_files\":{},\"text_bytes\":{text_bytes},\"index_bytes\":{index_bytes},\"build_ms\":{build_ms:.1}}}",
        text.len()
    );
    let take = |repo: &R| {
        (
            repo.store().reads.swap(0, Relaxed),
            repo.store().bytes.swap(0, Relaxed),
        )
    };
    for q in ["Q3", "AI", "EU", "%", "x", "invoice", "Q3 revenue"] {
        let mut line = format!("{{\"exp\":\"E132\",\"query\":\"{q}\"");
        let mut found = Vec::new();
        for with_index in [false, true] {
            take(&repo);
            let t = Instant::now();
            let index = if with_index {
                repo.search_index().unwrap()
            } else {
                None
            };
            let (mut hits, _) = repo.search(q.as_bytes(), &text, index.as_ref()).unwrap();
            let ms = t.elapsed().as_secs_f64() * 1e3;
            let (reads, fetched) = take(&repo);
            hits.sort();
            let tag = if with_index { "index" } else { "scan" };
            line += &format!(
                ",\"{tag}_reads\":{reads},\"{tag}_fetched\":{fetched},\"{tag}_ms\":{ms:.1}"
            );
            found.push(hits);
        }
        assert_eq!(found[0], found[1]);
        println!("{line},\"files_matched\":{}}}", found[0].len());
    }
}
