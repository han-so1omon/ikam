//! E140: section grants for a goal. What a grant exposes, what reading
//! only the granted sections fetches, and what checking reads and edits
//! against the grant costs, against a grant for whole files.
//!
//! `cargo run --release --example grants -- STORE REF`
//!
//! Copies the store into memory, builds the trigram index (setup), and for
//! each goal: finds the files, grants the lines around each mention (two
//! lines either side) for reading and for changing, then measures on a new
//! handle. Prints one JSON line per goal and part.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use ikam_kernel::{
    Denied, Error, FileGrant, FsStore, Grant, Id, Kind, MemStore, Object, Repo, Store,
    windows_around,
};

struct Counting<S: Store> {
    inner: S,
    reads: AtomicU64,
    bytes: AtomicU64,
}

impl<S: Store> Counting<S> {
    fn take(&self) -> (u64, u64) {
        (self.reads.swap(0, Relaxed), self.bytes.swap(0, Relaxed))
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
    // Partial reads pass through and count only the bytes fetched (E135).
    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        let out = self.inner.read_head(id, n)?;
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

const TEXT: &[&str] = &[
    "md", "txt", "csv", "json", "jsonl", "html", "xml", "py", "log", "eml",
];

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

/// Mean nanoseconds of `f` over `n` runs.
fn ns(n: u32, mut f: impl FnMut()) -> f64 {
    let t = Instant::now();
    for _ in 0..n {
        f();
    }
    t.elapsed().as_secs_f64() * 1e9 / n as f64
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (src, refname) = (Path::new(&a[0]), a[1].as_str());
    let fs = FsStore::open(src).unwrap();
    let mut objects: Vec<(Id, Vec<u8>)> = fs
        .ids()
        .unwrap()
        .into_iter()
        .map(|id| (id, fs.read(&id).unwrap()))
        .collect();
    let refs = fs.refs().unwrap();
    let open = |objects: &[(Id, Vec<u8>)]| -> R {
        let mut mem = MemStore::default();
        for (id, b) in objects {
            mem.write(*id, b).unwrap();
        }
        for (name, id) in &refs {
            mem.set_ref(name, None, *id).unwrap();
        }
        let mut repo = Repo::new(Counting {
            inner: mem,
            reads: 0.into(),
            bytes: 0.into(),
        });
        // The recommended setup (E129): no whole-store ledger on a new handle.
        repo.needed_index = true;
        repo
    };

    // Setup: the trigram index over the text files.
    let mut setup = open(&objects);
    let commit = setup.resolve(refname).unwrap();
    let Object::Commit(c) = setup.get(&commit).unwrap() else {
        panic!()
    };
    let mut all = Vec::new();
    files(&setup, c.tree, "", &mut all);
    let text: Vec<(String, Id)> = all
        .iter()
        .filter(|f| {
            f.0.rsplit_once('.')
                .is_some_and(|(_, e)| TEXT.contains(&e.to_lowercase().as_str()))
        })
        .cloned()
        .collect();
    let text_ids: Vec<Id> = text.iter().map(|f| f.1).collect();
    setup.build_search_index(&text_ids).unwrap();
    objects = setup
        .store()
        .inner
        .ids()
        .unwrap()
        .into_iter()
        .map(|id| (id, setup.store().inner.read(&id).unwrap()))
        .collect();
    drop(setup);
    let key = *blake3::hash(b"e140 example key, not a secret").as_bytes();

    for (term, to) in [("Q3", "Q4"), ("AI", "ML")] {
        let tb = term.as_bytes();
        // The server finds the files and their mentions (it reads the
        // matched files whole to place the windows).
        let repo = open(&objects);
        let index = repo.search_index().unwrap();
        repo.store().take();
        let t = Instant::now();
        let (hits, _) = repo.search(tb, &text_ids, index.as_ref()).unwrap();
        let hit_set: HashSet<Id> = hits.iter().copied().collect();
        let matched: Vec<&(String, Id)> = text.iter().filter(|f| hit_set.contains(&f.1)).collect();
        let s = repo.session(64 << 20);
        let contents: Vec<Vec<u8>> = matched
            .iter()
            .map(|f| s.read_content(&f.1).unwrap())
            .collect();
        let mut grant = Grant {
            goal: format!("{term} to {to}"),
            files: Vec::new(),
        };
        for (f, b) in matched.iter().zip(&contents) {
            let w = windows_around(b, tb, 2);
            grant.files.push(FileGrant::new(&f.0, f.1, w.clone(), w));
        }
        let build_ms = t.elapsed().as_secs_f64() * 1e3;
        let (build_reads, build_fetched) = repo.store().take();
        let whole: u64 = contents.iter().map(|b| b.len() as u64).sum();
        let granted: u64 = grant.files.iter().map(FileGrant::granted_bytes).sum();
        let sections: usize = grant.files.iter().map(|f| f.read.len()).sum();
        let enc = grant.encode();
        let tag = grant.seal(&key);
        let seal_ns = ns(2000, || {
            std::hint::black_box(grant.seal(&key));
        });
        let verify_ns = ns(2000, || {
            assert!(grant.verify(&key, std::hint::black_box(&tag)));
        });
        println!(
            "{{\"exp\":\"E140\",\"goal\":\"{term}\",\"part\":\"grant\",\"paths\":{},\"distinct_files\":{},\"sections\":{sections},\"whole_bytes\":{whole},\"granted_bytes\":{granted},\"build_reads\":{build_reads},\"build_fetched\":{build_fetched},\"build_ms\":{build_ms:.1},\"encoded_bytes\":{},\"seal_ns\":{seal_ns:.0},\"verify_ns\":{verify_ns:.0}}}",
            grant.files.len(),
            hit_set.len(),
            enc.len()
        );
        drop(s);
        drop(repo);

        // The caller reads its sections on a new handle, against reading
        // the same files whole on another.
        for how in ["whole_files", "granted_sections"] {
            let repo = open(&objects);
            let s = repo.session(64 << 20);
            // The new handle's first lookup (E135 heads) is counted apart.
            repo.has_content(&Id::of_content(b"x")).unwrap();
            let (first_reads, first_fetched) = repo.store().take();
            let t = Instant::now();
            let mut handed = 0u64;
            for (f, b) in grant.files.iter().zip(&contents) {
                if how == "whole_files" {
                    handed += s.read_content(&f.id).unwrap().len() as u64;
                } else {
                    for (r, got) in f.read(&s).unwrap() {
                        assert_eq!(&b[r.start as usize..r.end as usize], got.as_slice());
                        handed += got.len() as u64;
                    }
                }
            }
            let ms = t.elapsed().as_secs_f64() * 1e3;
            let (reads, fetched) = repo.store().take();
            println!(
                "{{\"exp\":\"E140\",\"goal\":\"{term}\",\"part\":\"read\",\"how\":\"{how}\",\"first_lookup_reads\":{first_reads},\"first_lookup_fetched\":{first_fetched},\"handed_to_caller\":{handed},\"reads\":{reads},\"fetched\":{fetched},\"ms\":{ms:.1}}}"
            );
        }

        // Checks. Reads: each granted section, and the first line of each
        // file when it is outside the grant.
        let mut read_checks = 0u32;
        let mut read_refused = 0u32;
        let read_ns = ns(20, || {
            for f in &grant.files {
                for r in &f.read {
                    assert_eq!(grant.may_read(&f.path, &f.id, r.clone()), Ok(()));
                    read_checks += 1;
                }
                if grant.may_read(&f.path, &f.id, 0..1).is_err() {
                    read_refused += 1;
                }
                read_checks += 1;
            }
        }) / (read_checks as f64 / 20.0);

        // Edits, for every matched file:
        // - the goal's edit: every mention of the term replaced;
        // - one line far from any mention changed (when the file has one);
        // - the goal's edit plus that line;
        // - the goal's edit against a version the grant does not name;
        // - an edit to a text file the grant does not name.
        let mut tally = std::collections::BTreeMap::<&str, (u32, u32, u32, f64)>::new();
        let mut note = |kind: &'static str, ok: bool, file_level_ok: bool, ns: f64| {
            let e = tally.entry(kind).or_default();
            if ok {
                e.0 += 1;
            }
            if file_level_ok {
                e.1 += 1;
            }
            e.2 += 1;
            e.3 += ns;
        };
        for (f, b) in grant.files.iter().zip(&contents) {
            let mentions: Vec<usize> = memchr::memmem::find_iter(b, tb).collect();
            let goal_edit = mentions.iter().rev().fold(b.clone(), |mut v, &at| {
                v.splice(at..at + tb.len(), to.bytes());
                v
            });
            let mut res = Ok(vec![]);
            let t = ns(5, || res = grant.check_edit(&f.path, &f.id, b, &goal_edit));
            note("goal_edit", res.is_ok(), true, t);
            // A line outside every window: the first such line with text.
            let mut at = 0usize;
            let mut far = None;
            for l in b.split_inclusive(|&c| c == b'\n') {
                let r = at as u64..(at + l.len()) as u64;
                if l.len() > 1 && !f.write.iter().any(|w| w.start < r.end && r.start < w.end) {
                    far = Some(at);
                    break;
                }
                at += l.len();
            }
            if let Some(line) = far {
                // Inside the line, not at its start, which may be where a
                // window ends (an insertion there is inside the window).
                let at = line + 1;
                let mut outside = b.clone();
                outside.insert(at, 1);
                let t = ns(5, || res = grant.check_edit(&f.path, &f.id, b, &outside));
                note("line_outside_the_windows", res.is_ok(), true, t);
                assert!(matches!(res, Err(Denied::Outside(_))));
                let mut both = goal_edit.clone();
                // The same place in the edited bytes.
                let before = mentions.iter().filter(|&&m| m < at).count() as isize;
                let shift = before * (to.len() as isize - tb.len() as isize);
                both.insert((at as isize + shift) as usize, 1);
                let t = ns(5, || res = grant.check_edit(&f.path, &f.id, b, &both));
                note("goal_edit_plus_a_line_outside", res.is_ok(), true, t);
            }
            let stale = Id::of_content(&goal_edit);
            let t = ns(5, || res = grant.check_edit(&f.path, &stale, &goal_edit, b));
            note("edit_of_another_version", res.is_ok(), true, t);
        }
        let others: Vec<&(String, Id)> = text
            .iter()
            .filter(|f| !hit_set.contains(&f.1))
            .take(grant.files.len())
            .collect();
        for f in others {
            let b = b"any bytes";
            let mut res = Ok(vec![]);
            let t = ns(5, || res = grant.check_edit(&f.0, &f.1, b, b"other bytes"));
            note("edit_of_a_file_not_granted", res.is_ok(), false, t);
        }
        println!(
            "{{\"exp\":\"E140\",\"goal\":\"{term}\",\"part\":\"read_checks\",\"checks\":{},\"refused_first_line\":{},\"ns_per_check\":{read_ns:.0}}}",
            read_checks / 20,
            read_refused / 20
        );
        for (kind, (ok, file_ok, n, total)) in tally {
            println!(
                "{{\"exp\":\"E140\",\"goal\":\"{term}\",\"part\":\"edit_checks\",\"edit\":\"{kind}\",\"edits\":{n},\"allowed_by_section_grant\":{ok},\"allowed_by_file_grant\":{file_ok},\"us_per_check\":{:.2}}}",
                total / n as f64 / 1e3
            );
        }
    }
}
