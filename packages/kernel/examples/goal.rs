//! E133: one goal end to end, with today's defaults and with the options
//! E124 to E132 recommend.
//!
//! `cargo run --release --example goal -- SRC_STORE REF`
//!
//! The goal: find the text files that mention a short term ("Q3"), hand a
//! model the lines around each mention, change the term in five of them,
//! commit, and keep a reverse-link view current. Each configuration starts
//! from the same store (with a trigram index and the view already built) on
//! a fresh handle, then runs a second goal ("AI") on the same handle. Prints
//! one JSON line per stage: objects read, bytes fetched, scans, ms, bytes
//! handed to the model and bytes written.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

use ikam_kernel::{
    Edit, Error, FsStore, Graph, Id, Keys, Kind, MemStore, Node, Object, Repo, Store, Target,
};

struct Counting<S: Store> {
    inner: S,
    reads: AtomicU64,
    bytes: AtomicU64,
    scans: AtomicU64,
    // E138: reads by the stored encoding's first byte, and repeat reads.
    by_tag: std::sync::Mutex<BTreeMap<u8, (u64, u64)>>,
    seen: std::sync::Mutex<HashMap<Id, (u64, usize)>>,
    repeats: AtomicU64,
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
        let e = *self
            .by_tag
            .lock()
            .unwrap()
            .entry(out.first().copied().unwrap_or(0))
            .or_default();
        self.by_tag.lock().unwrap().insert(
            out.first().copied().unwrap_or(0),
            (e.0 + 1, e.1 + out.len() as u64),
        );
        let mut seen = self.seen.lock().unwrap();
        let e = seen.entry(*id).or_insert((0, out.len()));
        e.0 += 1;
        if e.0 > 1 {
            self.repeats.fetch_add(1, Relaxed);
        }
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

/// E139: the goal on a file store instead of memory, when `GOAL_FS` names a
/// scratch directory (emptied and refilled for each configuration).
enum AnyStore {
    Mem(MemStore),
    Fs(FsStore),
}

macro_rules! each {
    ($s:expr, $x:ident => $e:expr) => {
        match $s {
            AnyStore::Mem($x) => $e,
            AnyStore::Fs($x) => $e,
        }
    };
}

impl Store for AnyStore {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        each!(self, s => s.write(id, encoded))
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        each!(self, s => s.replace(id, encoded))
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        each!(self, s => s.read(id))
    }
    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        each!(self, s => s.read_head(id, n))
    }
    fn has(&self, id: &Id) -> bool {
        each!(self, s => s.has(id))
    }
    fn ids(&self) -> Result<Vec<Id>, Error> {
        each!(self, s => s.ids())
    }
    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        each!(self, s => s.ids_with_prefix(prefix))
    }
    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        each!(self, s => s.delete(id))
    }
    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        each!(self, s => s.get_ref(name))
    }
    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        each!(self, s => s.set_ref(name, expected, new))
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        each!(self, s => s.refs())
    }
}

type R = Repo<Counting<AnyStore>>;

const TEXT: &[&str] = &[
    "md", "txt", "csv", "json", "jsonl", "html", "xml", "py", "log", "eml",
];

fn wrap(objects: &[(Id, Vec<u8>)], refs: &[(String, Id)]) -> R {
    let mut mem = match std::env::var_os("GOAL_FS") {
        Some(dir) => {
            let _ = std::fs::remove_dir_all(&dir);
            AnyStore::Fs(FsStore::open(dir).unwrap())
        }
        None => AnyStore::Mem(MemStore::default()),
    };
    for (id, b) in objects {
        mem.write(*id, b).unwrap();
    }
    for (name, id) in refs {
        mem.set_ref(name, None, *id).unwrap();
    }
    Repo::new(Counting {
        inner: mem,
        reads: 0.into(),
        bytes: 0.into(),
        scans: 0.into(),
        by_tag: Default::default(),
        seen: Default::default(),
        repeats: 0.into(),
    })
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

fn tree_ids(repo: &R, tree: Id, out: &mut Vec<Id>) {
    out.push(tree);
    if let Ok(Object::Tree(entries)) = repo.get(&tree) {
        for e in entries {
            if let Kind::Tree = e.kind {
                tree_ids(repo, e.id, out);
            }
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
    rev.sort_by(|a, b| a.label.cmp(&b.label));
    rev.dedup_by(|a, b| a.label == b.label);
    rev
}

/// The lines within two of a line holding `term`.
fn windows(bytes: &[u8], term: &[u8]) -> usize {
    let lines: Vec<&[u8]> = bytes.split(|&b| b == b'\n').collect();
    let finder = memchr::memmem::Finder::new(term);
    let mut keep = vec![false; lines.len()];
    for (i, l) in lines.iter().enumerate() {
        if finder.find(l).is_some() {
            for k in keep
                .iter_mut()
                .take((i + 3).min(lines.len()))
                .skip(i.saturating_sub(2))
            {
                *k = true;
            }
        }
    }
    lines
        .iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(l, _)| l.len() + 1)
        .sum()
}

/// The view's chunks: a graph's children, down to its node chunks.
fn chunks(repo: &R, id: Id, out: &mut Vec<Id>) {
    out.push(id);
    if let Ok(Object::Graph(Graph::Children(_, children))) = repo.get(&id) {
        for (_, c) in children {
            chunks(repo, c, out);
        }
    }
}

fn top(path: &str) -> &str {
    path.split('/').next().unwrap()
}

struct Goal<'a> {
    term: &'a str,
    to: &'a str,
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
    let mut refs = fs.refs().unwrap();

    // Setup, not measured: a trigram index over the text files and a
    // reverse-link view over every object, as a long-running store has.
    let mut setup = wrap(&objects, &refs);
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
    let ids = setup.store().inner.ids().unwrap();
    let view = setup
        .put_graph_with(reverse_edges(&setup, &ids), Keys::Any)
        .unwrap();
    objects = setup
        .store()
        .inner
        .ids()
        .unwrap()
        .into_iter()
        .map(|id| (id, setup.store().inner.read(&id).unwrap()))
        .collect();
    refs = setup.store().inner.refs().unwrap();
    drop(setup);
    println!(
        "{{\"exp\":\"E133\",\"store_objects\":{},\"text_files\":{}}}",
        objects.len(),
        text.len()
    );

    let goals = [
        Goal {
            term: "Q3",
            to: "Q4",
        },
        Goal {
            term: "AI",
            to: "ML",
        },
    ];
    for (config, recommended, scope_view) in [
        ("defaults", false, false),
        ("recommended", true, true),
        ("recommended_scope_without_the_view", true, false),
        ("recommended_whole_store_scope", true, false),
        // E137: only the files the goal edits, not their folders.
        ("recommended_scope_edited_files", true, true),
        // E138: the same, keeping what each tree and view edit reads.
        ("recommended_read_memo", true, true),
    ] {
        let mut repo = wrap(&objects, &refs);
        repo.needed_index = recommended;
        repo.read_memo = config == "recommended_read_memo";
        let mut root = c.tree;
        let mut view = view;
        let mut known: HashSet<Id> = objects.iter().map(|o| o.0).collect();
        let line = |repo: &R, goal: &str, stage: &str, ms: f64, extra: String| {
            let io = repo.store().take();
            println!(
                "{{\"exp\":\"E133\",\"config\":\"{config}\",\"goal\":\"{goal}\",\"stage\":\"{stage}\",\"reads\":{},\"fetched\":{},\"scans\":{},\"ms\":{ms:.1}{extra}}}",
                io.reads, io.bytes, io.scans
            );
        };
        repo.store().take();
        let t = Instant::now();
        repo.has_content(&Id::of_content(b"x")).unwrap();
        line(
            &repo,
            "-",
            "first_lookup_builds_the_ledger",
            t.elapsed().as_secs_f64() * 1e3,
            String::new(),
        );
        for g in &goals {
            let term = g.term.as_bytes();
            // Search.
            let t = Instant::now();
            let index = if recommended {
                repo.search_index().unwrap()
            } else {
                None
            };
            let (hits, _) = repo.search(term, &text_ids, index.as_ref()).unwrap();
            line(
                &repo,
                g.term,
                "search",
                t.elapsed().as_secs_f64() * 1e3,
                format!(",\"files_matched\":{}", hits.len()),
            );
            // Read: the model gets the lines around each mention.
            let t = Instant::now();
            let mut contents = Vec::new();
            if recommended {
                let s = repo.session(64 << 20);
                for id in &hits {
                    contents.push(s.read_content(id).unwrap());
                }
            } else {
                for id in &hits {
                    contents.push(repo.read_content(id).unwrap());
                }
            }
            let model: usize = contents.iter().map(|b| windows(b, term)).sum();
            let whole: usize = contents.iter().map(Vec::len).sum();
            line(
                &repo,
                g.term,
                "read",
                t.elapsed().as_secs_f64() * 1e3,
                format!(",\"to_model\":{model},\"whole_files\":{whole}"),
            );
            // Edit five of them and commit.
            let hit_set: HashSet<&Id> = hits.iter().collect();
            let chosen: Vec<&(String, Id)> = text
                .iter()
                .filter(|f| hit_set.contains(&f.1))
                .take(5)
                .collect();
            if recommended && config != "recommended_whole_store_scope" {
                let folders: HashSet<&str> = chosen.iter().map(|f| top(&f.0)).collect();
                let mut scope: Vec<Id> = if config == "recommended_scope_edited_files"
                    || config == "recommended_read_memo"
                {
                    chosen.iter().map(|f| f.1).collect()
                } else {
                    all.iter()
                        .filter(|f| folders.contains(top(&f.0)))
                        .map(|f| f.1)
                        .collect()
                };
                // Trees and view chunks are stored as content, so their
                // previous versions are where their new versions repeat.
                if scope_view {
                    chunks(&repo, view, &mut scope);
                    let mut trees = Vec::new();
                    tree_ids(&repo, root, &mut trees);
                    scope.extend(trees);
                }
                // A new scope per goal; the seed index is rebuilt for it.
                repo.set_seed_scope(Some(scope));
            }
            repo.store().by_tag.lock().unwrap().clear();
            let t = Instant::now();
            let before = repo.bytes_written;
            let mut edits = Vec::new();
            let mut first_put = None;
            for f in &chosen {
                let i = hits.iter().position(|h| *h == f.1).unwrap();
                let b = &contents[i];
                let at = memchr::memmem::find(b, term).unwrap();
                let mut nb = b[..at].to_vec();
                nb.extend_from_slice(g.to.as_bytes());
                nb.extend_from_slice(&b[at + term.len()..]);
                let id = repo.put_content(&nb).unwrap().id;
                // E139: the first write builds the seed index for the scope.
                if first_put.is_none() {
                    first_put = Some((
                        repo.store().reads.load(Relaxed),
                        repo.store().bytes.load(Relaxed),
                    ));
                }
                edits.push(Edit::Put {
                    path: f.0.clone(),
                    id,
                });
            }
            root = repo.edit_tree(Some(root), &edits).unwrap();
            repo.commit(refname, root, g.term).unwrap();
            let written = repo.bytes_written - before;
            let tags: Vec<String> = repo
                .store()
                .by_tag
                .lock()
                .unwrap()
                .iter()
                .map(|(t, (n, b))| format!("\"{}\":[{n},{b}]", *t as char))
                .collect();
            let (fr, fb) = first_put.unwrap_or_default();
            println!(
                "{{\"exp\":\"E139\",\"config\":\"{config}\",\"goal\":\"{}\",\"part\":\"edit_reads\",\"first_put_reads\":{fr},\"first_put_fetched\":{fb},\"tags\":{{{}}}}}",
                g.term,
                tags.join(",")
            );
            line(
                &repo,
                g.term,
                "edit_and_commit",
                t.elapsed().as_secs_f64() * 1e3,
                format!(",\"files_edited\":{},\"written\":{written}", chosen.len()),
            );
            // Keep the reverse-link view current.
            let new: Vec<Id> = repo
                .store()
                .inner
                .ids()
                .unwrap()
                .into_iter()
                .filter(|i| !known.contains(i))
                .collect();
            known.extend(new.iter().copied());
            repo.store().take();
            let t = Instant::now();
            let before = repo.bytes_written;
            let ed: BTreeMap<String, Option<Node>> = reverse_edges(&repo, &new)
                .into_iter()
                .map(|n| (n.label.clone(), Some(n)))
                .collect();
            let edges = ed.len();
            // E138: split the stage into finding the new edges and the edit.
            let found = repo.store().take();
            let found_ms = t.elapsed().as_secs_f64() * 1e3;
            println!(
                "{{\"exp\":\"E138\",\"config\":\"{config}\",\"goal\":\"{}\",\"part\":\"find_new_edges\",\"new_objects\":{},\"reads\":{},\"fetched\":{},\"ms\":{found_ms:.1}}}",
                g.term,
                new.len(),
                found.reads,
                found.bytes
            );
            // Put the counts back so the stage line still covers both parts.
            repo.store().reads.fetch_add(found.reads, Relaxed);
            repo.store().bytes.fetch_add(found.bytes, Relaxed);
            repo.store().by_tag.lock().unwrap().clear();
            repo.store().seen.lock().unwrap().clear();
            repo.store().repeats.swap(0, Relaxed);
            view = repo.graph_edit_with(&view, ed, Keys::Any).unwrap();
            let tags: Vec<String> = repo
                .store()
                .by_tag
                .lock()
                .unwrap()
                .iter()
                .map(|(t, (n, b))| format!("\"{}\":[{n},{b}]", *t as char))
                .collect();
            println!(
                "{{\"exp\":\"E138\",\"config\":\"{config}\",\"goal\":\"{}\",\"part\":\"graph_edit_reads_by_tag\",\"repeat_reads\":{},\"tags\":{{{}}}}}",
                g.term,
                repo.store().repeats.load(Relaxed),
                tags.join(",")
            );
            let seen = repo.store().seen.lock().unwrap();
            let mut top: Vec<(u64, usize)> = seen.values().copied().filter(|v| v.0 > 1).collect();
            top.sort_by_key(|v| std::cmp::Reverse(v.0));
            let distinct_repeated = top.len();
            top.truncate(8);
            println!(
                "{{\"exp\":\"E138\",\"config\":\"{config}\",\"goal\":\"{}\",\"part\":\"most_read\",\"distinct\":{},\"distinct_repeated\":{distinct_repeated},\"top_reads_and_sizes\":{:?}}}",
                g.term,
                seen.len(),
                top
            );
            drop(seen);
            let written = repo.bytes_written - before;
            line(
                &repo,
                g.term,
                "view_update",
                t.elapsed().as_secs_f64() * 1e3,
                format!(",\"new_edges\":{edges},\"written\":{written}"),
            );
            known.extend(repo.store().inner.ids().unwrap());
        }
        assert!(repo.fsck().unwrap().is_empty());
    }
}
