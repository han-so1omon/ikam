//! E135: what a fresh handle's projection pass (the derivation ledger and,
//! with `needed_index` on, the needed index) fetches when the store can
//! read only an object's header.
//!
//! `cargo run --release --example heads -- STORE REF`
//!
//! Opens the `FsStore` at STORE behind a counting wrapper. With
//! `partial=false` the wrapper answers `read_head` by reading the whole
//! object, as a store that can't read part of one would; with `partial=true`
//! it passes it on to `FsStore::read_head`. Prints one JSON line per
//! setting and run: requests, bytes fetched and milliseconds for the first
//! lookup on a new handle, and checks that every file's derivations match.

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use ikam_kernel::{Error, FsStore, Id, Kind, Object, Repo, Store};

struct Counting<S: Store> {
    inner: S,
    partial: bool,
    requests: AtomicU64,
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
        self.requests.fetch_add(1, Relaxed);
        self.bytes.fetch_add(out.len() as u64, Relaxed);
        Ok(out)
    }
    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        if !self.partial {
            let mut out = self.read(id)?;
            out.truncate(n);
            return Ok(out);
        }
        let out = self.inner.read_head(id, n)?;
        self.requests.fetch_add(1, Relaxed);
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

type R = Repo<Counting<FsStore>>;

fn files(repo: &R, tree: Id, out: &mut Vec<Id>) {
    let Object::Tree(entries) = repo.get(&tree).unwrap() else {
        panic!("not a tree")
    };
    for e in entries {
        match e.kind {
            Kind::Tree => files(repo, e.id, out),
            _ => out.push(e.id),
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (src, refname) = (Path::new(&a[0]), a[1].as_str());
    let open = |partial: bool, needed: bool| -> R {
        let mut repo = Repo::new(Counting {
            inner: FsStore::open(src).unwrap(),
            partial,
            requests: 0.into(),
            bytes: 0.into(),
        });
        repo.needed_index = needed;
        repo
    };
    let mut reference: Option<Vec<String>> = None;
    for run in 0..2 {
        for needed in [false, true] {
            for partial in [false, true] {
                let repo = open(partial, needed);
                let t = Instant::now();
                repo.has_content(&Id::of_content(b"not stored")).unwrap();
                let ms = t.elapsed().as_secs_f64() * 1e3;
                let requests = repo.store().requests.swap(0, Relaxed);
                let bytes = repo.store().bytes.swap(0, Relaxed);
                // Every file's derivations, as text, must match across settings.
                let c = repo.resolve(refname).unwrap();
                let Object::Commit(c) = repo.get(&c).unwrap() else {
                    panic!()
                };
                let mut all = Vec::new();
                files(&repo, c.tree, &mut all);
                let ders: Vec<String> = all
                    .iter()
                    .map(|id| format!("{:?}", repo.derivations(id).unwrap()))
                    .collect();
                match &reference {
                    None => reference = Some(ders),
                    Some(r) => assert_eq!(r, &ders, "derivations differ"),
                }
                println!(
                    "{{\"exp\":\"E135\",\"run\":{run},\"needed_index\":{needed},\"partial_reads\":{partial},\"objects\":{},\"requests\":{requests},\"fetched\":{bytes},\"ms\":{ms:.1}}}",
                    repo.store().inner.ids().unwrap().len()
                );
            }
        }
    }
}
