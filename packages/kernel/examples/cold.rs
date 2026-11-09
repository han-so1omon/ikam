//! E130: what the first read, write and lookup on a fresh handle read.
//!
//! `cargo run --release --example cold -- STORE REF`
//!
//! Opens the `FsStore` at STORE behind a counting wrapper, on a new handle
//! for each phase, and prints one JSON line per phase: objects read, bytes
//! fetched, full scans and milliseconds. Then estimates what a rebuildable
//! cache of the derivation ledger would cost to load instead (written to a
//! scratch file next to nothing in the store; the kernel does not read it).

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use ikam_kernel::{Error, FsStore, Id, Kind, Object, Repo, Store};

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

type R = Repo<Counting<FsStore>>;

fn open(src: &Path) -> R {
    Repo::new(Counting {
        inner: FsStore::open(src).unwrap(),
        reads: 0.into(),
        bytes: 0.into(),
        scans: 0.into(),
    })
}

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

fn line(phase: &str, io: Io, ms: f64, extra: &str) {
    println!(
        "{{\"exp\":\"E130\",\"phase\":\"{phase}\",\"reads\":{},\"fetched\":{},\"scans\":{},\"ms\":{ms:.1}{extra}}}",
        io.reads, io.bytes, io.scans
    );
}

fn timed<T>(repo: &R, phase: &str, f: impl FnOnce() -> T) -> T {
    repo.store().take();
    let t = Instant::now();
    let out = f();
    line(
        phase,
        repo.store().take(),
        t.elapsed().as_secs_f64() * 1e3,
        "",
    );
    out
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (src, refname) = (Path::new(&a[0]), a[1].as_str());

    let t = Instant::now();
    let repo = open(src);
    line(
        "open",
        repo.store().take(),
        t.elapsed().as_secs_f64() * 1e3,
        "",
    );
    let objects = repo.store().inner.ids().unwrap().len();

    // Which files are stored as bytes, and which only as a derivation.
    let commit = repo.resolve(refname).unwrap();
    let Object::Commit(c) = repo.get(&commit).unwrap() else {
        panic!()
    };
    let mut all = Vec::new();
    files(&repo, c.tree, &mut all);
    let plain = *all.iter().find(|i| repo.store().has(i)).unwrap();
    let derived = *all.iter().find(|i| !repo.store().has(i)).unwrap();
    let derived_share = all.iter().filter(|i| !repo.store().has(i)).count();
    println!(
        "{{\"exp\":\"E130\",\"store_objects\":{objects},\"files\":{},\"files_stored_only_as_derivations\":{derived_share}}}",
        all.len()
    );

    // Each phase on a new handle.
    let repo = open(src);
    timed(&repo, "resolve_ref_and_root_tree", || {
        let c = repo.resolve(refname).unwrap();
        repo.get(&c).unwrap();
    });
    let repo = open(src);
    timed(&repo, "first_read_of_a_file_stored_as_bytes", || {
        repo.read_content(&plain).unwrap()
    });
    let repo = open(src);
    timed(&repo, "first_read_of_a_derived_file", || {
        repo.read_content(&derived).unwrap()
    });
    timed(&repo, "second_read_of_it_same_handle", || {
        repo.read_content(&derived).unwrap()
    });
    let repo = open(src);
    timed(&repo, "first_has_content", || {
        repo.has_content(&Id::of_content(b"not stored")).unwrap()
    });

    // A write needs a scratch copy: the store must not change.
    let doc: Vec<u8> = (0..400)
        .flat_map(|i| format!("line {i} of a new note written on a fresh handle\n").into_bytes())
        .collect();
    {
        let mut mem = ikam_kernel::MemStore::default();
        let fs = FsStore::open(src).unwrap();
        for id in fs.ids().unwrap() {
            mem.write(id, &fs.read(&id).unwrap()).unwrap();
        }
        let mut repo = Repo::new(Counting {
            inner: mem,
            reads: 0.into(),
            bytes: 0.into(),
            scans: 0.into(),
        });
        repo.store().take();
        let t = Instant::now();
        repo.has_content(&Id::of_content(b"x")).unwrap();
        let ledger = repo.store().take();
        let ledger_ms = t.elapsed().as_secs_f64() * 1e3;
        line(
            "write_part_ledger_build",
            ledger,
            ledger_ms,
            ",\"store\":\"memory copy\"",
        );
        let t = Instant::now();
        repo.put_content(&doc).unwrap();
        line(
            "write_part_after_ledger_seed_index_and_plan",
            repo.store().take(),
            t.elapsed().as_secs_f64() * 1e3,
            ",\"store\":\"memory copy\"",
        );
        let t = Instant::now();
        repo.put_content(&doc[..doc.len() / 2]).unwrap();
        line(
            "second_write_same_handle",
            repo.store().take(),
            t.elapsed().as_secs_f64() * 1e3,
            ",\"store\":\"memory copy\"",
        );
    }

    // A ledger cache: every derivation record's encoding, length-prefixed,
    // in one file. Loading it is one sequential read plus decoding records.
    let repo = open(src);
    let fs = &repo.store().inner;
    let mut cache = Vec::new();
    let mut records = 0;
    for id in fs.ids().unwrap() {
        if let Ok(Object::Derivation(d)) = repo.get(&id) {
            let enc = Object::Derivation(d).encode();
            cache.extend_from_slice(&(enc.len() as u32).to_le_bytes());
            cache.extend_from_slice(&enc);
            records += 1;
        }
    }
    let path = std::env::temp_dir().join("e130-ledger-cache.bin");
    std::fs::write(&path, &cache).unwrap();
    let t = Instant::now();
    let bytes = std::fs::read(&path).unwrap();
    let (mut at, mut n) = (0, 0);
    while at < bytes.len() {
        let len = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let obj = Object::decode(&bytes[at + 4..at + 4 + len]).unwrap();
        assert!(matches!(obj, Object::Derivation(_)));
        n += 1;
        at += 4 + len;
    }
    assert_eq!(n, records);
    line(
        "ledger_cache_load_estimate",
        Io {
            reads: 1,
            bytes: bytes.len() as u64,
            scans: 0,
        },
        t.elapsed().as_secs_f64() * 1e3,
        &format!(",\"records\":{records}"),
    );
    std::fs::remove_file(&path).ok();
}
