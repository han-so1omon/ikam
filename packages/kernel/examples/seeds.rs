//! E131: what `seed_scope` saves on a fresh handle's first write, and what it
//! costs in stored bytes.
//!
//! `cargo run --release --example seeds -- SRC_STORE REF`
//!
//! For each write case and scope, copies the store at SRC_STORE into memory,
//! opens a new handle with that scope, applies the writes and prints one JSON
//! line: objects read and bytes fetched by the writes (the first write builds
//! the seed index), milliseconds, encoded bytes written, and how many of the
//! new derivations read content outside the scope.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use ikam_kernel::{Arg, Error, FsStore, Id, Kind, MemStore, Object, Repo, Store};

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

/// Files written by one case: paths and their new bytes.
type Writes = Vec<(String, Vec<u8>)>;

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

fn top(path: &str) -> &str {
    path.split('/').next().unwrap()
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (src, refname) = (Path::new(&a[0]), a[1].as_str());
    let fs = FsStore::open(src).unwrap();
    let objects: Vec<(Id, Vec<u8>)> = fs
        .ids()
        .unwrap()
        .into_iter()
        .map(|id| (id, fs.read(&id).unwrap()))
        .collect();
    let refs = fs.refs().unwrap();
    let open = || -> R {
        let mut mem = MemStore::default();
        for (id, b) in &objects {
            mem.write(*id, b).unwrap();
        }
        for (name, id) in &refs {
            mem.set_ref(name, None, *id).unwrap();
        }
        Repo::new(Counting {
            inner: mem,
            reads: 0.into(),
            bytes: 0.into(),
        })
    };

    let probe = open();
    let commit = probe.resolve(refname).unwrap();
    let Object::Commit(c) = probe.get(&commit).unwrap() else {
        panic!()
    };
    let mut all = Vec::new();
    files(&probe, c.tree, "", &mut all);
    let read = |id: &Id| probe.read_content(id).unwrap();

    // Write cases: (name, writes, folder the writes come from).
    let mut by_ext = std::collections::BTreeMap::new();
    for f in &all {
        let ext = f.0.rsplit_once('.').map_or("", |x| x.1).to_string();
        by_ext.entry(ext).or_insert(f);
    }
    let edit = |f: &(String, Id)| {
        let mut b = read(&f.1);
        b.extend_from_slice(b"\n");
        (f.0.clone(), b)
    };
    let md = all.iter().filter(|f| f.0.ends_with(".md")).nth(3).unwrap();
    let folder = "l-construction-v01";
    let cases: Vec<(&str, Writes, Vec<String>)> = vec![
        (
            "one_md_file_edited",
            vec![edit(md)],
            vec![top(&md.0).into()],
        ),
        (
            "one_file_per_type_edited",
            by_ext.values().map(|f| edit(f)).collect(),
            by_ext.values().map(|f| top(&f.0).to_string()).collect(),
        ),
        (
            "folder_copied_to_a_new_folder",
            all.iter()
                .filter(|f| top(&f.0) == folder)
                .map(|f| {
                    let (_, b) = edit(f);
                    (format!("e131-copy/{}", f.0), b)
                })
                .collect(),
            vec![folder.into()],
        ),
    ];

    for (name, writes, sources) in &cases {
        let dest: HashSet<&str> = writes.iter().map(|w| top(&w.0)).collect();
        let src: HashSet<&str> = sources.iter().map(|s| s.as_str()).collect();
        let scopes: Vec<(&str, Option<Vec<Id>>)> = vec![
            ("whole_store", None),
            ("files_of_the_tree", Some(all.iter().map(|f| f.1).collect())),
            (
                "folders_written_to",
                Some(
                    all.iter()
                        .filter(|f| dest.contains(top(&f.0)))
                        .map(|f| f.1)
                        .collect(),
                ),
            ),
            (
                "folders_written_to_and_read_from",
                Some(
                    all.iter()
                        .filter(|f| dest.contains(top(&f.0)) || src.contains(top(&f.0)))
                        .map(|f| f.1)
                        .collect(),
                ),
            ),
            ("empty", Some(vec![])),
        ];
        for (scope_name, scope) in scopes {
            let mut repo = open();
            let roots = scope.clone();
            repo.seed_scope = scope;
            // The ledger is built by any first lookup; keep it out of the
            // seed-index numbers (E130 measures it).
            repo.has_content(&Id::of_content(b"x")).unwrap();
            repo.store().reads.swap(0, Relaxed);
            repo.store().bytes.swap(0, Relaxed);
            let before = repo.bytes_written;
            let t = Instant::now();
            let mut new = Vec::new();
            for (_, b) in writes {
                new.push(repo.put_content(b).unwrap().id);
            }
            let ms = t.elapsed().as_secs_f64() * 1e3;
            let reads = repo.store().reads.swap(0, Relaxed);
            let fetched = repo.store().bytes.swap(0, Relaxed);
            let written = repo.bytes_written - before;
            // Derivation arguments of the new contents that are stored
            // content from before this handle, outside the scope's closure.
            let preexisting: HashSet<Id> = objects.iter().map(|o| o.0).collect();
            let allowed: Option<HashSet<Id>> = roots.map(|r| {
                let mut seen: HashSet<Id> = HashSet::new();
                let mut stack = r;
                while let Some(c) = stack.pop() {
                    if seen.insert(c) {
                        for d in repo.derivations(&c).unwrap() {
                            stack.extend(d.args.iter().map(Arg::id));
                        }
                    }
                }
                seen
            });
            let mut outside = 0;
            let mut stack = new.clone();
            let mut seen = HashSet::new();
            while let Some(c) = stack.pop() {
                if !seen.insert(c) {
                    continue;
                }
                for d in repo.derivations(&c).unwrap() {
                    for arg in d.args.iter().map(Arg::id) {
                        if preexisting.contains(&arg)
                            && allowed.as_ref().is_some_and(|s| !s.contains(&arg))
                        {
                            outside += 1;
                        }
                        stack.push(arg);
                    }
                }
            }
            for ((_, b), id) in writes.iter().zip(&new) {
                assert_eq!(&repo.read_content(id).unwrap(), b);
            }
            assert!(repo.fsck().unwrap().is_empty());
            println!(
                "{{\"exp\":\"E131\",\"write\":\"{name}\",\"scope\":\"{scope_name}\",\"files\":{},\"input_bytes\":{},\"reads\":{reads},\"fetched\":{fetched},\"ms\":{ms:.1},\"written\":{written},\"args_outside_scope\":{outside}}}",
                writes.len(),
                writes.iter().map(|w| w.1.len()).sum::<usize>(),
            );
        }
    }
}
