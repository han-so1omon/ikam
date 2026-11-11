//! E226: where a commit's time goes on a Mercante-style report copy.
//!
//! `cargo run --release --example commits [folder sizes ...]`
//!
//! As `ikam live` does for one WebDAV PUT: store the report
//! (`put_content`), edit the working tree at `reports/<name>`
//! (`edit_tree`), then commit (`Repo::commit`). An FsStore in a temporary
//! directory, one handle kept open (as the server keeps it), defaults.
//! The folder already holds N reports; each run times 20 copies after one
//! warm-up and reports medians, plus the bytes each phase stored.

mod ir_common;

use std::time::Instant;

use std::cell::Cell;

use ikam_kernel::{Edit, Error, FsStore, Id, MemStore, Repo, Store};
use ir_common::*;

/// A store that counts what the Repo asks of it.
struct Counting<S> {
    inner: S,
    writes: Cell<usize>,
    reads: Cell<usize>,
    scans: Cell<usize>,
    deletes: Cell<usize>,
}

impl<S: Store> Counting<S> {
    fn new(inner: S) -> Counting<S> {
        Counting {
            inner,
            writes: Cell::new(0),
            reads: Cell::new(0),
            scans: Cell::new(0),
            deletes: Cell::new(0),
        }
    }
    fn counts(&self) -> [usize; 4] {
        [
            self.writes.get(),
            self.reads.get(),
            self.scans.get(),
            self.deletes.get(),
        ]
    }
}

fn bump(c: &Cell<usize>) {
    c.set(c.get() + 1);
}

impl<S: Store> Store for Counting<S> {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        let fresh = self.inner.write(id, encoded)?;
        if fresh {
            bump(&self.writes);
        }
        Ok(fresh)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        bump(&self.writes);
        self.inner.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        bump(&self.reads);
        self.inner.read(id)
    }
    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        bump(&self.reads);
        self.inner.read_head(id, n)
    }
    fn stored_len(&self, id: &Id) -> Result<usize, Error> {
        self.inner.stored_len(id)
    }
    fn has(&self, id: &Id) -> bool {
        self.inner.has(id)
    }
    fn ids(&self) -> Result<Vec<Id>, Error> {
        bump(&self.scans);
        self.inner.ids()
    }
    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        self.inner.ids_with_prefix(prefix)
    }
    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        bump(&self.deletes);
        self.inner.delete(id)
    }
    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        self.inner.get_ref(name)
    }
    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        bump(&self.writes);
        self.inner.set_ref(name, expected, new)
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        self.inner.refs()
    }
}

const COPIES: usize = 20;

/// One phase's samples: ms, bytes stored, store writes, store reads.
#[derive(Default)]
struct Phase(Vec<(f64, usize, usize, usize)>);

impl Phase {
    fn median(&self) -> (f64, usize, usize, usize) {
        let mid = |mut v: Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let ms = mid(self.0.iter().map(|p| p.0).collect());
        let n = |f: fn(&(f64, usize, usize, usize)) -> usize| {
            mid(self.0.iter().map(|p| f(p) as f64).collect()) as usize
        };
        (ms, n(|p| p.1), n(|p| p.2), n(|p| p.3))
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mem = args.iter().any(|a| a == "--mem");
    let deferred = args.iter().any(|a| a == "--deferred");
    let templates = !args.iter().any(|a| a == "--no-object-templates");
    // `--every K`: commit after every K copies (`ikam live` commits once
    // edits pause), else after each.
    let every = args
        .iter()
        .position(|a| a == "--every")
        .and_then(|i| args.get(i + 1)?.parse().ok())
        .unwrap_or(1);
    let sizes: Vec<usize> = args
        .iter()
        .enumerate()
        .filter(|(i, _)| *i == 0 || args[i - 1] != "--every")
        .filter_map(|(_, a)| a.parse().ok())
        .collect();
    let sizes = if sizes.is_empty() {
        vec![2, 30, 300, 3000]
    } else {
        sizes
    };
    let kind = match (mem, deferred) {
        (true, _) => "MemStore",
        (false, true) => "FsStore::open_deferred (temporary directory)",
        (false, false) => "FsStore (temporary directory)",
    };
    println!(
        "store: {kind}; a commit after every {every} copies; {COPIES} copies; object_templates {templates}"
    );
    println!(
        "| reports in folder | store ms (B, writes) | tree ms (B, writes, reads) | commit ms (B, writes) | copy ms | all copies ms | first copy ms (reads, scans) | drops (scans) |"
    );
    println!("| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    for n in sizes {
        let dir = tempfile::tempdir().unwrap();
        if mem {
            run(n, MemStore::default, true, every, templates);
        } else if deferred {
            run(
                n,
                || FsStore::open_deferred(dir.path()).unwrap(),
                false,
                every,
                templates,
            );
        } else {
            run(
                n,
                || FsStore::open(dir.path()).unwrap(),
                false,
                every,
                templates,
            );
        }
    }
}

fn run<S: Store>(n: usize, open: impl Fn() -> S, mem: bool, every: usize, templates: bool) {
    let all = reports(n + COPIES + 1, false, 7);
    let mut repo = Repo::new(Counting::new(open()));
    repo.object_templates = templates;
    // Setup: the N reports, committed in one change.
    let mut edits = Vec::new();
    for (path, bytes) in &all[..n] {
        let id = repo.put_content(bytes).unwrap().id;
        edits.push(Edit::Put {
            path: path.clone(),
            id,
        });
    }
    let mut work = repo.edit_tree(None, &edits).unwrap();
    repo.commit("main", work, "setup").unwrap();
    // A fresh handle, as after a restart (a MemStore cannot be reopened).
    if !mem {
        repo = Repo::new(Counting::new(open()));
        repo.object_templates = templates;
    }
    let (mut store, mut tree, mut commit) = (Phase::default(), Phase::default(), Phase::default());
    let mut first = (0.0, 0, 0);
    let mut drops = (0, 0); // stored objects dropped, whole-store scans
    let mut all_ms = 0.0;
    for (i, (path, bytes)) in all[n..].iter().enumerate() {
        let at =
            |repo: &Repo<Counting<S>>| (Instant::now(), repo.bytes_written, repo.store().counts());
        let a = at(&repo);
        let id: Id = repo.put_content(bytes).unwrap().id;
        let b = at(&repo);
        let put = Edit::Put {
            path: path.clone(),
            id,
        };
        work = repo.edit_tree(Some(work), &[put]).unwrap();
        let c = at(&repo);
        if (i + 1) % every == 0 || i + 1 == COPIES + 1 || i == 0 {
            repo.commit("main", work, "live: changes").unwrap();
        }
        let d = at(&repo);
        if i > 0 {
            all_ms += (d.0 - a.0).as_secs_f64() * 1e3;
        }
        if i == 0 {
            first = (
                (d.0 - a.0).as_secs_f64() * 1e3,
                d.2[1] - a.2[1],
                d.2[2] - a.2[2],
            );
            continue;
        }
        let diff = |x: &(Instant, usize, [usize; 4]), y: &(Instant, usize, [usize; 4])| {
            (
                (y.0 - x.0).as_secs_f64() * 1e3,
                y.1 - x.1,
                y.2[0] - x.2[0],
                y.2[1] - x.2[1],
            )
        };
        store.0.push(diff(&a, &b));
        tree.0.push(diff(&b, &c));
        if (i + 1) % every == 0 {
            commit.0.push(diff(&c, &d));
        }
        drops.0 += d.2[3] - a.2[3];
        drops.1 += d.2[2] - a.2[2];
    }
    let (s, t, c) = (store.median(), tree.median(), commit.median());
    println!(
        "| {n} | {:.2} ({}, {}) | {:.2} ({}, {}, {}) | {:.2} ({}, {}) | {:.2} | {all_ms:.1} | {:.1} ({}, {}) | {} ({}) |",
        s.0,
        s.1,
        s.2,
        t.0,
        t.1,
        t.2,
        t.3,
        c.0,
        c.1,
        c.2,
        s.0 + t.0,
        first.0,
        first.1,
        first.2,
        drops.0,
        drops.1
    );
}
