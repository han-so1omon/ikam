//! E186 to E188: per-fragment summaries stored beside the fragments.
//!
//! `cargo run --release --example stored_summaries` (E186, E187), and
//! `cargo run --release --example stored_summaries -- crash` (E188).
//!
//! E182 kept each fragment's summary in memory; this stores it. A summary is
//! a pure function of its fragment's bytes (`functions/summary/`, built to
//! `summary.wasm`), so it is recorded as a derivation
//! `summary = summary.wasm(fragment)`: content-addressed, it can be missing
//! but never stale, and `fsck_deep` re-evaluates it. A view graph names the
//! summary of every fragment the files use (`f/<fragment hex>`), and with
//! postings each word's fragments (`w/<word>`: their ids, sorted). The
//! commit's tree is `files/` plus the view (`summaries`, a graph entry), so
//! one ref swap moves files and view together and `gc` keeps both.
//!
//! (E182 measured lazy summaries going stale; a stored summary is keyed by
//! its fragment's id, so it cannot.) Modes, each on its own `FsStore` built from the same corpus and edit
//! stream as E182 (`common/insights.rs`):
//! - `memory`: E182's eager in-memory summaries (nothing stored, rebuilt
//!   from every fragment after a restart);
//! - `stored`: WASM derivations and the view, without postings;
//! - `stored+postings`: the same with word postings in the view;
//! - `native`: summaries computed natively and stored in the view as plain
//!   content, with no derivation (unverifiable; the cost of verification).
//!
//! E186 measures, per mode: the build, each edit's summary upkeep (time,
//! bytes read, bytes written beyond what `memory` writes for the same
//! edit), storage, a restart (open, then the first goal query and corpus
//! aggregates), warm goal queries, and exactness (every file summary and
//! the aggregates against a full recompute after every edit; every stored
//! summary against the native function; postings against the summaries).
//! E187 is the postings trade (`stored+postings` against `stored`).
//!
//! E188, kills: a child process runs the edit stream with postings on a
//! store and is killed at a random point, 30 times per store (`FsStore`,
//! `PackStore`); after each kill the parent opens the store and checks that
//! the head's view names exactly the head's fragments, every summary is
//! exact, the postings match, and `fsck` is clean.

#[path = "common/insights.rs"]
mod insights;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::{Duration, Instant};

use ikam_kernel::backends::PackStore;
use ikam_kernel::{
    Arg, Error, FsStore, Id, Keys, Kind, Node, Object, Repo, Store, Target, TreeEntry, blocks,
};
use insights::*;

// ---- counting store ------------------------------------------------------

#[derive(Default)]
struct Counts {
    written: AtomicU64,
    read: AtomicU64,
}

struct Counted<S> {
    s: S,
    c: Arc<Counts>,
}

impl<S: Store> Store for Counted<S> {
    fn write(&mut self, id: Id, encoded: &[u8]) -> Result<bool, Error> {
        let new = self.s.write(id, encoded)?;
        if new {
            self.c.written.fetch_add(encoded.len() as u64, Relaxed);
        }
        Ok(new)
    }
    fn replace(&mut self, id: Id, encoded: &[u8]) -> Result<(), Error> {
        self.c.written.fetch_add(encoded.len() as u64, Relaxed);
        self.s.replace(id, encoded)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        let b = self.s.read(id)?;
        self.c.read.fetch_add(b.len() as u64, Relaxed);
        Ok(b)
    }
    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        let b = self.s.read_head(id, n)?;
        self.c.read.fetch_add(b.len() as u64, Relaxed);
        Ok(b)
    }
    fn stored_len(&self, id: &Id) -> Result<usize, Error> {
        self.s.stored_len(id)
    }
    fn has(&self, id: &Id) -> bool {
        self.s.has(id)
    }
    fn ids(&self) -> Result<Vec<Id>, Error> {
        self.s.ids()
    }
    fn ids_with_prefix(&self, prefix: &[u8]) -> Result<Vec<Id>, Error> {
        self.s.ids_with_prefix(prefix)
    }
    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        self.s.delete(id)
    }
    fn get_ref(&self, name: &str) -> Result<Option<Id>, Error> {
        self.s.get_ref(name)
    }
    fn set_ref(&mut self, name: &str, expected: Option<Id>, new: Id) -> Result<(), Error> {
        self.s.set_ref(name, expected, new)
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        self.s.refs()
    }
}

// ---- summaries as bytes ----------------------------------------------------

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn varint(b: &[u8], at: &mut usize) -> u64 {
    let (mut v, mut shift) = (0u64, 0);
    loop {
        let x = b[*at];
        *at += 1;
        v |= u64::from(x & 0x7f) << shift;
        if x & 0x80 == 0 {
            return v;
        }
        shift += 7;
    }
}

/// The encoding `summary.wasm` writes (see `functions/summary/src/lib.rs`).
fn encode(s: &Summary) -> Vec<u8> {
    let mut out = b"S1".to_vec();
    for v in [
        s.money_n as u64,
        s.money_cents as u64,
        s.dates as u64,
        s.headings as u64,
        s.items as u64,
        s.words.len() as u64,
    ] {
        put_varint(&mut out, v);
    }
    let words: BTreeMap<&Vec<u8>, &u32> = s.words.iter().collect();
    for (w, c) in words {
        put_varint(&mut out, w.len() as u64);
        out.extend_from_slice(w);
        put_varint(&mut out, *c as u64);
    }
    out
}

fn decode(b: &[u8]) -> Summary {
    assert_eq!(&b[..2], b"S1");
    let mut at = 2;
    let mut s = Summary {
        money_n: varint(b, &mut at) as u32,
        money_cents: varint(b, &mut at) as i64,
        dates: varint(b, &mut at) as u32,
        headings: varint(b, &mut at) as u32,
        items: varint(b, &mut at) as u32,
        ..Default::default()
    };
    for _ in 0..varint(b, &mut at) {
        let n = varint(b, &mut at) as usize;
        let w = b[at..at + n].to_vec();
        at += n;
        s.words.insert(w, varint(b, &mut at) as u32);
    }
    s
}

// ---- the view ----------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Memory,
    Stored,
    Postings,
    NodePostings,
    /// E191: a word's list split by the first `n` hex digits of the
    /// fragment id, so an edit rewrites only the buckets it touches.
    Bucketed(usize),
    Native,
}

impl Mode {
    fn postings(self) -> bool {
        matches!(
            self,
            Mode::Postings | Mode::NodePostings | Mode::Bucketed(_)
        )
    }

    fn name(self) -> &'static str {
        match self {
            Mode::Memory => "memory",
            Mode::Stored => "stored",
            Mode::Postings => "stored+postings",
            Mode::NodePostings => "stored+node-postings",
            Mode::Bucketed(1) => "stored+postings/16",
            Mode::Bucketed(_) => "stored+postings/256",
            Mode::Native => "native",
        }
    }
}

const WASM: &str = "functions/summary.wasm";

fn module() -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join(WASM)).unwrap()
}

/// What a mode keeps: stored modes the view's root, `memory` its cache.
struct View {
    mode: Mode,
    func: Id,
    root: Id,
    /// How many file units use each fragment (the view names those > 0).
    uses: HashMap<Id, usize>,
    /// `memory`: summaries by fragment, and fragments by word.
    cache: HashMap<Id, Summary>,
    by_word: HashMap<Vec<u8>, BTreeSet<Id>>,
}

fn flabel(f: &Id) -> String {
    format!("f/{}", f.to_hex())
}

fn wlabel(w: &[u8]) -> String {
    format!("w/{}", String::from_utf8_lossy(w))
}

/// A node posting: one node per (word, fragment), no content.
fn plabel(w: &[u8], f: &Id) -> String {
    format!("w/{}/{}", String::from_utf8_lossy(w), f.to_hex())
}

/// A bucket of a word's list: fragments whose id starts with `b`.
fn blabel(w: &[u8], f: &Id, n: usize) -> String {
    format!("w/{}/{}", String::from_utf8_lossy(w), &f.to_hex()[..n])
}

/// The fragments holding `wd`, from any kind of postings.
fn candidates<S: Store>(repo: &Repo<S>, root: &Id, mode: Mode, wd: &[u8]) -> BTreeSet<Id> {
    if let Mode::Bucketed(_) = mode {
        let p = format!("{}/", wlabel(wd));
        let mut out = BTreeSet::new();
        for n in repo.graph_prefix(root, &p).unwrap() {
            out.extend(ids_of(&repo.read_content(&content_of(&n)).unwrap()));
        }
        return out;
    }
    if mode == Mode::NodePostings {
        let p = format!("{}/", wlabel(wd));
        return repo
            .graph_prefix(root, &p)
            .unwrap()
            .into_iter()
            .map(|n| n.label[p.len()..].parse().unwrap())
            .collect();
    }
    repo.graph_node(root, &wlabel(wd))
        .unwrap()
        .map_or(BTreeSet::new(), |n| {
            ids_of(&repo.read_content(&content_of(&n)).unwrap())
        })
}

fn node(label: String, content: Id) -> Node {
    Node {
        label,
        target: Target::Content(Arg::Whole(content)),
        edges: vec![],
    }
}

fn content_of(n: &Node) -> Id {
    match &n.target {
        Target::Content(a) => a.id(),
        _ => panic!("{}: no content", n.label),
    }
}

fn ids_of(b: &[u8]) -> BTreeSet<Id> {
    b.chunks(32)
        .map(|c| Id::from_bytes(c.try_into().unwrap()))
        .collect()
}

impl View {
    fn new<S: Store>(repo: &mut Repo<S>, mode: Mode) -> View {
        let func = if mode == Mode::Stored || mode.postings() {
            repo.put_content(&module()).unwrap().id
        } else {
            Id::of(b"none")
        };
        let root = repo.put_graph_with(vec![], Keys::Any).unwrap();
        View {
            mode,
            func,
            root,
            uses: HashMap::new(),
            cache: HashMap::new(),
            by_word: HashMap::new(),
        }
    }

    /// The summary content of fragment `f`, made and stored.
    fn summarize<S: Store>(&self, repo: &mut Repo<S>, f: &Id) -> Id {
        match self.mode {
            Mode::Stored | Mode::Postings | Mode::NodePostings | Mode::Bucketed(_) => {
                repo.apply(self.func, vec![Arg::Whole(*f)]).unwrap()
            }
            Mode::Native => {
                let s = Summary::of(&repo.read_content(f).unwrap());
                repo.put_content(&encode(&s)).unwrap().id
            }
            Mode::Memory => unreachable!(),
        }
    }

    /// Bring the view up to date with fragments now used (`added`) and no
    /// longer used (`removed`).
    fn update<S: Store>(&mut self, repo: &mut Repo<S>, added: &[Id], removed: &[Id]) {
        if self.mode == Mode::Memory {
            for f in added {
                if !self.cache.contains_key(f) {
                    let s = Summary::of(&repo.read_content(f).unwrap());
                    for w in s.words.keys() {
                        self.by_word.entry(w.clone()).or_default().insert(*f);
                    }
                    self.cache.insert(*f, s);
                }
            }
            for f in removed {
                if let Some(s) = self.cache.remove(f) {
                    for w in s.words.keys() {
                        let e = self.by_word.get_mut(w).unwrap();
                        e.remove(f);
                        if e.is_empty() {
                            self.by_word.remove(w);
                        }
                    }
                }
            }
            return;
        }
        let mut edits: BTreeMap<String, Option<Node>> = BTreeMap::new();
        let mut plus: BTreeMap<Vec<u8>, Vec<Id>> = BTreeMap::new();
        let mut minus: BTreeMap<Vec<u8>, Vec<Id>> = BTreeMap::new();
        for f in added {
            let sid = self.summarize(repo, f);
            edits.insert(flabel(f), Some(node(flabel(f), sid)));
            if self.mode.postings() {
                for w in decode(&repo.read_content(&sid).unwrap()).words.into_keys() {
                    plus.entry(w).or_default().push(*f);
                }
            }
        }
        if self.mode.postings() && !removed.is_empty() {
            let labels: BTreeSet<String> = removed.iter().map(flabel).collect();
            for n in repo.graph_nodes_in(&self.root, &labels).unwrap() {
                let f: Id = n.label[2..].parse().unwrap();
                let s = decode(&repo.read_content(&content_of(&n)).unwrap());
                for w in s.words.into_keys() {
                    minus.entry(w).or_default().push(f);
                }
            }
        }
        for f in removed {
            edits.insert(flabel(f), None);
        }
        if self.mode == Mode::NodePostings {
            for (w, fs) in &plus {
                for f in fs {
                    let l = plabel(w, f);
                    edits.insert(
                        l.clone(),
                        Some(Node {
                            label: l,
                            target: Target::None,
                            edges: vec![],
                        }),
                    );
                }
            }
            for (w, fs) in &minus {
                for f in fs {
                    edits.insert(plabel(w, f), None);
                }
            }
        }
        if let Mode::Bucketed(n) = self.mode {
            let mut lists: BTreeMap<String, BTreeSet<Id>> = BTreeMap::new();
            for (w, fs) in plus.iter().chain(&minus) {
                for f in fs {
                    lists.entry(blabel(w, f, n)).or_default();
                }
            }
            let labels: BTreeSet<String> = lists.keys().cloned().collect();
            for nd in repo.graph_nodes_in(&self.root, &labels).unwrap() {
                let ids = ids_of(&repo.read_content(&content_of(&nd)).unwrap());
                lists.insert(nd.label, ids);
            }
            for (w, fs) in &plus {
                for f in fs {
                    lists.get_mut(&blabel(w, f, n)).unwrap().insert(*f);
                }
            }
            for (w, fs) in &minus {
                for f in fs {
                    lists.get_mut(&blabel(w, f, n)).unwrap().remove(f);
                }
            }
            for (label, ids) in lists {
                if ids.is_empty() {
                    edits.insert(label, None);
                } else {
                    let b: Vec<u8> = ids.iter().flat_map(|i| *i.as_bytes()).collect();
                    let id = repo.put_content(&b).unwrap().id;
                    edits.insert(label.clone(), Some(node(label, id)));
                }
            }
        }
        if self.mode == Mode::Postings {
            let words: BTreeSet<String> =
                plus.keys().chain(minus.keys()).map(|w| wlabel(w)).collect();
            let mut lists: HashMap<String, BTreeSet<Id>> = repo
                .graph_nodes_in(&self.root, &words)
                .unwrap()
                .into_iter()
                .map(|n| {
                    let ids = ids_of(&repo.read_content(&content_of(&n)).unwrap());
                    (n.label, ids)
                })
                .collect();
            for (w, fs) in &plus {
                lists.entry(wlabel(w)).or_default().extend(fs);
            }
            for (w, fs) in &minus {
                let e = lists.entry(wlabel(w)).or_default();
                for f in fs {
                    e.remove(f);
                }
            }
            for (label, ids) in lists {
                if ids.is_empty() {
                    edits.insert(label, None);
                } else {
                    let b: Vec<u8> = ids.iter().flat_map(|i| *i.as_bytes()).collect();
                    let id = repo.put_content(&b).unwrap().id;
                    edits.insert(label.clone(), Some(node(label, id)));
                }
            }
        }
        self.root = repo.graph_edit_with(&self.root, edits, Keys::Any).unwrap();
    }

    /// Count uses of each changed file's units, old and new: the fragments
    /// that start and stop being used.
    fn diff(
        &mut self,
        old: &BTreeMap<String, Vec<Id>>,
        new: &BTreeMap<String, Vec<Id>>,
        paths: &[String],
    ) -> (Vec<Id>, Vec<Id>) {
        let mut before: BTreeMap<Id, usize> = BTreeMap::new();
        for p in paths {
            for f in old.get(p).into_iter().flatten().chain(&new[p]) {
                before
                    .entry(*f)
                    .or_insert_with(|| self.uses.get(f).copied().unwrap_or(0));
            }
        }
        for p in paths {
            for f in old.get(p).into_iter().flatten() {
                *self.uses.get_mut(f).unwrap() -= 1;
            }
            for f in &new[p] {
                *self.uses.entry(*f).or_default() += 1;
            }
        }
        let (mut added, mut removed) = (Vec::new(), Vec::new());
        for (f, b) in before {
            let n = self.uses[&f];
            if n == 0 {
                self.uses.remove(&f);
            }
            if b == 0 && n > 0 {
                added.push(f);
            } else if b > 0 && n == 0 {
                removed.push(f);
            }
        }
        (added, removed)
    }
}

/// Edits after the corpus is stored, in every mode, with the write options
/// E178 found fast: whether a dropped encoding is needed answered from an
/// index (`needed_index`, E129; otherwise each chunk a view edit replaces
/// reads the whole store), seeds only from what an edit reads (an empty
/// `seed_scope` with `replace_hints`, E141), each old chunk read at most
/// once (`read_memo`, E138).
fn fast_writes<S: Store>(repo: &mut Repo<S>) {
    repo.needed_index = true;
    repo.set_seed_scope(Some(vec![]));
    repo.replace_hints = true;
    repo.read_memo = true;
}

/// Commit the file tree, with the view for stored modes.
fn commit<S: Store>(repo: &mut Repo<S>, files: Id, view: &View, msg: &str) {
    let tree = if view.mode == Mode::Memory {
        files
    } else {
        repo.put(
            &Object::tree(vec![
                TreeEntry {
                    name: "files".into(),
                    kind: Kind::Tree,
                    id: files,
                },
                TreeEntry {
                    name: "summaries".into(),
                    kind: Kind::Graph,
                    id: view.root,
                },
            ])
            .unwrap(),
        )
        .unwrap()
    };
    repo.commit("main", tree, msg).unwrap();
}

/// The head's file tree and view root.
fn head<S: Store>(repo: &Repo<S>) -> Option<(Id, Option<Id>)> {
    let c = repo.store().get_ref("main").unwrap()?;
    let Object::Commit(c) = repo.get(&c).unwrap() else {
        panic!()
    };
    let Object::Tree(es) = repo.get(&c.tree).unwrap() else {
        panic!()
    };
    let find = |n: &str| es.iter().find(|e| e.name == n).map(|e| e.id);
    match find("files") {
        Some(f) => Some((f, find("summaries"))),
        None => Some((c.tree, None)),
    }
}

/// Every fragment's summary, as the view (or cache) holds it.
fn summaries<S: Store>(repo: &Repo<S>, v: &View) -> HashMap<Id, Summary> {
    if v.mode == Mode::Memory {
        return v.cache.clone();
    }
    repo.graph_prefix(&v.root, "f/")
        .unwrap()
        .into_iter()
        .map(|n| {
            let f: Id = n.label[2..].parse().unwrap();
            (f, decode(&repo.read_content(&content_of(&n)).unwrap()))
        })
        .collect()
}

fn files_and_agg(
    units: &BTreeMap<String, Vec<Id>>,
    sums: &HashMap<Id, Summary>,
) -> (BTreeMap<String, Summary>, Agg) {
    let mut files = BTreeMap::new();
    let mut agg = Agg::default();
    for (p, us) in units {
        let mut s = Summary::default();
        for u in us {
            s.add(&sums[u], 1);
        }
        agg.apply(&s, 1);
        files.insert(p.clone(), s);
    }
    (files, agg)
}

/// Exactness of a stored view: it names exactly the live fragments, each
/// summary equals the native function's bytes, and postings (if any) are
/// exactly the summaries' words. Returns the number of summaries checked.
fn check_view<S: Store>(repo: &Repo<S>, root: &Id, units: &BTreeMap<String, Vec<Id>>) -> usize {
    let live: BTreeSet<Id> = units.values().flatten().copied().collect();
    let nodes = repo.graph_prefix(root, "f/").unwrap();
    let named: BTreeSet<Id> = nodes
        .iter()
        .map(|n| n.label[2..].parse().unwrap())
        .collect();
    assert_eq!(named, live, "view names exactly the head's fragments");
    let mut want: BTreeMap<String, BTreeSet<Id>> = BTreeMap::new();
    for n in &nodes {
        let f: Id = n.label[2..].parse().unwrap();
        let b = repo.read_content(&content_of(n)).unwrap();
        assert_eq!(b, encode(&Summary::of(&repo.read_content(&f).unwrap())));
        for w in decode(&b).words.into_keys() {
            want.entry(wlabel(&w)).or_default().insert(f);
        }
    }
    let mut got: BTreeMap<String, BTreeSet<Id>> = BTreeMap::new();
    for n in repo.graph_prefix(root, "w/").unwrap() {
        if n.target == Target::None {
            let (w, f) = n.label.rsplit_once('/').unwrap();
            got.entry(w.to_string())
                .or_default()
                .insert(f.parse().unwrap());
        } else {
            // A whole list `w/<word>`, or one bucket `w/<word>/<prefix>`.
            let ids = ids_of(&repo.read_content(&content_of(&n)).unwrap());
            let w = match n.label[2..].split_once('/') {
                Some((w, _)) => format!("w/{w}"),
                None => n.label.clone(),
            };
            got.entry(w).or_default().extend(ids);
        }
    }
    if !got.is_empty() {
        assert_eq!(got, want, "postings match the summaries");
    }
    nodes.len()
}

// ---- goal queries (E183) -------------------------------------------------------

fn mentions(blk: &[u8], wd: &[u8]) -> bool {
    blk.split(|c| !c.is_ascii_alphanumeric())
        .any(|t| t.len() >= 3 && t.eq_ignore_ascii_case(wd))
}

/// Per block holding the word: (money count, cents, dates).
fn answer(b: &[u8], wd: &[u8]) -> (u32, i64, u32) {
    let mut a = (0, 0, 0);
    for r in blocks(b) {
        if mentions(&b[r.clone()], wd) {
            let (n, c) = money(&b[r.clone()]);
            a.0 += n;
            a.1 += c;
            a.2 += dates(&b[r]);
        }
    }
    a
}

type Answer = BTreeMap<String, (u32, i64, u32)>;

/// A goal query from the fragments that hold the word.
fn query<S: Store>(
    repo: &Repo<S>,
    units: &BTreeMap<String, Vec<Id>>,
    cand: &BTreeSet<Id>,
    wd: &[u8],
) -> Answer {
    let per: HashMap<Id, (u32, i64, u32)> = cand
        .iter()
        .map(|f| (*f, answer(&repo.read_content(f).unwrap(), wd)))
        .collect();
    let mut got = Answer::new();
    for (p, us) in units {
        let mut a = (0, 0, 0);
        let mut hit = false;
        for u in us {
            if let Some(x) = per.get(u) {
                hit = true;
                a.0 += x.0;
                a.1 += x.1;
                a.2 += x.2;
            }
        }
        if hit {
            got.insert(p.clone(), a);
        }
    }
    got
}

/// The truth: read every file.
fn query_files<S: Store>(repo: &Repo<S>, units: &BTreeMap<String, Vec<Id>>, wd: &[u8]) -> Answer {
    let mut out = Answer::new();
    for (p, us) in units {
        let b: Vec<u8> = us
            .iter()
            .flat_map(|u| repo.read_content(u).unwrap())
            .collect();
        let a = answer(&b, wd);
        if a != (0, 0, 0) || mentions(&b, wd) {
            out.insert(p.clone(), a);
        }
    }
    out
}

fn pick_words(agg: &Agg) -> Vec<Vec<u8>> {
    let mut vocab: Vec<&Vec<u8>> = agg
        .df
        .iter()
        .filter(|(k, d)| (5..=120).contains(*d) && k.iter().all(u8::is_ascii_alphabetic))
        .map(|e| e.0)
        .collect();
    vocab.sort();
    let mut x: u64 = 0x9e3779b97f4a7c15;
    let mut words: Vec<Vec<u8>> = Vec::new();
    while words.len() < 12 {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let wd = vocab[(x >> 33) as usize % vocab.len()].clone();
        if !words.contains(&wd) {
            words.push(wd);
        }
    }
    words
}

// ---- E186 / E187 -------------------------------------------------------------

fn line(what: &str, extra: String) {
    println!("{{\"what\":\"{what}\",{extra}}}");
}

thread_local! {
    static KIND: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// A line tagged with the store it ran on.
fn sline(what: &str, extra: String) {
    let k = KIND.with(|k| k.borrow().clone());
    line(what, format!("\"store\":\"{k}\",{extra}"));
}

fn store_bytes<S: Store>(s: &S) -> usize {
    s.ids()
        .unwrap()
        .iter()
        .map(|i| s.stored_len(i).unwrap())
        .sum()
}

struct Run {
    /// Per edit: bytes written for the whole edit and commit.
    written: Vec<u64>,
}

fn run(
    kind: &str,
    mode: Mode,
    files: &[(String, Vec<u8>)],
    dir: &Path,
    baseline: Option<&Run>,
) -> Run {
    KIND.with(|k| *k.borrow_mut() = kind.to_string());
    let c = Arc::new(Counts::default());
    let store = Counted {
        s: Dyn(open_store(kind, dir)),
        c: c.clone(),
    };
    let mut w = setup(store, files);
    fast_writes(&mut w.repo);
    let corpus_written = c.written.load(Relaxed);
    let mut v = View::new(&mut w.repo, mode);

    // Build.
    let (w0, r0) = (c.written.load(Relaxed), c.read.load(Relaxed));
    let t = Instant::now();
    let (added, _) = v.diff(
        &BTreeMap::new(),
        &w.units.clone(),
        &w.units.keys().cloned().collect::<Vec<_>>(),
    );
    v.update(&mut w.repo, &added, &[]);
    let build_ms = ms(t);
    commit(&mut w.repo, w.tree, &v, "corpus");
    let (build_w, build_r) = (c.written.load(Relaxed) - w0, c.read.load(Relaxed) - r0);
    let sums = summaries(&w.repo, &v);
    let (truth_files, truth_agg, _) = full(&w);
    assert_eq!(files_and_agg(&w.units, &sums), (truth_files, truth_agg));
    let fragments = sums.len();
    let storage_build = store_bytes(w.repo.store());
    sline(
        "build",
        format!(
            "\"mode\":\"{}\",\"fragments\":{fragments},\"ms\":{build_ms:.2},\"read\":{build_r},\"written_beyond_corpus\":{build_w},\"corpus_written\":{corpus_written},\"store_bytes\":{storage_build}",
            mode.name()
        ),
    );

    // Edit stream.
    let mut written = Vec::new();
    let (mut tot_ms, mut tot_r, mut tot_added, mut tot_removed) = (0.0, 0u64, 0, 0);
    for step in 0..22 {
        let old = w.units.clone();
        let w0 = c.written.load(Relaxed);
        let changed = edit(&mut w, step, files);
        let (r0, t) = (c.read.load(Relaxed), Instant::now());
        let (added, removed) = v.diff(&old, &w.units, &changed);
        v.update(&mut w.repo, &added, &removed);
        let up_ms = ms(t);
        let up_r = c.read.load(Relaxed) - r0;
        commit(&mut w.repo, w.tree, &v, "edit");
        let wr = c.written.load(Relaxed) - w0;
        written.push(wr);
        let extra = baseline.map_or(0, |b| wr as i64 - b.written[step] as i64);
        let sums = summaries(&w.repo, &v);
        let (tf, ta, _) = full(&w);
        assert_eq!(
            files_and_agg(&w.units, &sums),
            (tf, ta),
            "{} step {step}",
            mode.name()
        );
        sline(
            "edit",
            format!(
                "\"mode\":\"{}\",\"step\":{step},\"files_changed\":{},\"added\":{},\"removed\":{},\"upkeep_ms\":{up_ms:.3},\"upkeep_read\":{up_r},\"written\":{wr},\"written_beyond_memory\":{extra}",
                mode.name(),
                changed.len(),
                added.len(),
                removed.len()
            ),
        );
        tot_ms += up_ms;
        tot_r += up_r;
        tot_added += added.len();
        tot_removed += removed.len();
    }
    let checked = if mode == Mode::Memory {
        0
    } else {
        check_view(&w.repo, &v.root, &w.units)
    };
    let storage_end = store_bytes(w.repo.store());
    let tot_w: u64 = written.iter().sum();
    let extra_w = baseline.map_or(0, |b| tot_w as i64 - b.written.iter().sum::<u64>() as i64);
    sline(
        "stream",
        format!(
            "\"mode\":\"{}\",\"edits\":22,\"added\":{tot_added},\"removed\":{tot_removed},\"upkeep_ms\":{tot_ms:.2},\"upkeep_read\":{tot_r},\"written\":{tot_w},\"written_beyond_memory\":{extra_w},\"store_bytes\":{storage_end},\"summaries_checked\":{checked},\"exact\":true",
            mode.name()
        ),
    );

    // gc keeps the view; fsck (and fsck_deep for derivations).
    let t = Instant::now();
    let deleted = w.repo.gc().unwrap();
    let gc_ms = ms(t);
    if mode != Mode::Memory {
        check_view(&w.repo, &v.root, &w.units);
    }
    assert!(w.repo.fsck().unwrap().is_empty());
    let t = Instant::now();
    assert!(w.repo.fsck_deep().unwrap().is_empty());
    let deep_ms = ms(t);
    sline(
        "gc",
        format!(
            "\"mode\":\"{}\",\"deleted\":{deleted},\"gc_ms\":{gc_ms:.1},\"store_bytes\":{},\"view_intact\":true,\"fsck_deep_ms\":{deep_ms:.1}",
            mode.name(),
            store_bytes(w.repo.store())
        ),
    );
    let words = pick_words(&full(&w).1);
    let truth: Vec<Answer> = words
        .iter()
        .map(|wd| query_files(&w.repo, &w.units, wd))
        .collect();
    let warm_cache = (v.cache.clone(), v.by_word.clone());
    drop(w);
    if kind == "mem" {
        return Run { written };
    }

    // Restart: open the store as a new process would.
    let c = Arc::new(Counts::default());
    let t = Instant::now();
    let repo = Repo::new(Counted {
        s: Dyn(open_store(kind, dir)),
        c: c.clone(),
    });
    let (files_tree, root) = head(&repo).unwrap();
    let units = units_of(&repo.fragment_index(&files_tree).unwrap());
    let open_ms = ms(t);
    let open_r = c.read.load(Relaxed);
    // First goal query after the restart, and the corpus aggregates.
    let (r0, t) = (c.read.load(Relaxed), Instant::now());
    let (cand0, agg) = match mode {
        Mode::Memory => {
            // Rebuild the cache from every fragment.
            let mut by_word: HashMap<Vec<u8>, BTreeSet<Id>> = HashMap::new();
            let mut sums = HashMap::new();
            for f in units.values().flatten() {
                if !sums.contains_key(f) {
                    let s = Summary::of(&repo.read_content(f).unwrap());
                    for wd in s.words.keys() {
                        by_word.entry(wd.clone()).or_default().insert(*f);
                    }
                    sums.insert(*f, s);
                }
            }
            (
                by_word.remove(&words[0]).unwrap_or_default(),
                files_and_agg(&units, &sums).1,
            )
        }
        Mode::Postings | Mode::NodePostings | Mode::Bucketed(_) => (
            candidates(&repo, &root.unwrap(), mode, &words[0]),
            Agg::default(),
        ),
        _ => {
            let v = View {
                root: root.unwrap(),
                ..View::new_empty(mode)
            };
            let sums = summaries(&repo, &v);
            let cand = sums
                .iter()
                .filter(|(_, s)| s.words.contains_key(&words[0]))
                .map(|e| *e.0)
                .collect();
            (cand, files_and_agg(&units, &sums).1)
        }
    };
    let got = query(&repo, &units, &cand0, &words[0]);
    let first_ms = ms(t);
    let first_r = c.read.load(Relaxed) - r0;
    assert_eq!(got, truth[0]);
    if !mode.postings() {
        assert_eq!(agg, full_agg(&repo, &units));
    }
    // Aggregates after the restart, for postings mode (reads every summary).
    let (r0, t) = (c.read.load(Relaxed), Instant::now());
    let agg_r = if mode.postings() {
        let v = View {
            root: root.unwrap(),
            ..View::new_empty(mode)
        };
        let a = files_and_agg(&units, &summaries(&repo, &v)).1;
        assert_eq!(a, full_agg(&repo, &units));
        c.read.load(Relaxed) - r0
    } else {
        0
    };
    let agg_ms = if mode.postings() { ms(t) } else { 0.0 };
    sline(
        "restart",
        format!(
            "\"mode\":\"{}\",\"open_ms\":{open_ms:.2},\"open_read\":{open_r},\"first_query_ms\":{first_ms:.2},\"first_query_read\":{first_r},\"aggregates_after_ms\":{agg_ms:.2},\"aggregates_after_read\":{agg_r}",
            mode.name()
        ),
    );

    // Warm goal queries: candidates from the cache, a summary scan, or postings.
    let by_word: HashMap<Vec<u8>, BTreeSet<Id>> = match mode {
        Mode::Memory => warm_cache.1,
        Mode::Postings | Mode::NodePostings | Mode::Bucketed(_) => HashMap::new(),
        _ => {
            let v = View {
                root: root.unwrap(),
                ..View::new_empty(mode)
            };
            let mut m: HashMap<Vec<u8>, BTreeSet<Id>> = HashMap::new();
            for (f, s) in summaries(&repo, &v) {
                for wd in s.words.into_keys() {
                    m.entry(wd).or_default().insert(f);
                }
            }
            m
        }
    };
    let (r0, t) = (c.read.load(Relaxed), Instant::now());
    for (wd, want) in words.iter().zip(&truth) {
        let cand = if mode.postings() {
            candidates(&repo, &root.unwrap(), mode, wd)
        } else {
            by_word.get(wd).cloned().unwrap_or_default()
        };
        assert_eq!(&query(&repo, &units, &cand, wd), want);
    }
    let q = words.len() as f64;
    sline(
        "warm query",
        format!(
            "\"mode\":\"{}\",\"words\":{},\"ms\":{:.3},\"read\":{},\"exact\":true",
            mode.name(),
            words.len(),
            ms(t) / q,
            (c.read.load(Relaxed) - r0) / words.len() as u64
        ),
    );
    Run { written }
}

fn full_agg<S: Store>(repo: &Repo<S>, units: &BTreeMap<String, Vec<Id>>) -> Agg {
    let mut agg = Agg::default();
    for us in units.values() {
        let b: Vec<u8> = us
            .iter()
            .flat_map(|u| repo.read_content(u).unwrap())
            .collect();
        agg.apply(&Summary::of(&b), 1);
    }
    agg
}

impl View {
    fn new_empty(mode: Mode) -> View {
        View {
            mode,
            func: Id::of(b"none"),
            root: Id::of(b"none"),
            uses: HashMap::new(),
            cache: HashMap::new(),
            by_word: HashMap::new(),
        }
    }
}

/// The WASM function's output equals the native summary for every block
/// of the corpus (and the function's cost).
fn check_function(files: &[(String, Vec<u8>)]) {
    let mut repo = Repo::new(ikam_kernel::MemStore::default());
    let func = repo.put_content(&module()).unwrap().id;
    let (mut n, mut work, mut bytes) = (0, 0usize, 0usize);
    let t = Instant::now();
    for (_, b) in corpus(files) {
        for r in blocks(&b) {
            let id = repo.put_content(&b[r.clone()]).unwrap().id;
            let (out, wk) = repo.evaluate(&func, &[Arg::Whole(id)]).unwrap();
            assert_eq!(out, encode(&Summary::of(&b[r.clone()])));
            n += 1;
            work += wk;
            bytes += r.len();
        }
    }
    line(
        "function",
        format!(
            "\"module_bytes\":{},\"blocks\":{n},\"input_bytes\":{bytes},\"ms\":{:.1},\"decode_work\":{work},\"matches_native\":true",
            module().len(),
            ms(t)
        ),
    );
}

// ---- E188 --------------------------------------------------------------------

fn open_store(kind: &str, dir: &Path) -> Box<dyn Store> {
    match kind {
        "fs" => Box::new(FsStore::open(dir).unwrap()),
        "mem" => Box::new(ikam_kernel::MemStore::default()),
        _ => Box::new(PackStore::open(dir).unwrap()),
    }
}

/// One code path for both stores.
struct Dyn(Box<dyn Store>);

impl Store for Dyn {
    fn write(&mut self, id: Id, e: &[u8]) -> Result<bool, Error> {
        self.0.write(id, e)
    }
    fn replace(&mut self, id: Id, e: &[u8]) -> Result<(), Error> {
        self.0.replace(id, e)
    }
    fn read(&self, id: &Id) -> Result<Vec<u8>, Error> {
        self.0.read(id)
    }
    fn read_head(&self, id: &Id, n: usize) -> Result<Vec<u8>, Error> {
        self.0.read_head(id, n)
    }
    fn stored_len(&self, id: &Id) -> Result<usize, Error> {
        self.0.stored_len(id)
    }
    fn has(&self, id: &Id) -> bool {
        self.0.has(id)
    }
    fn ids(&self) -> Result<Vec<Id>, Error> {
        self.0.ids()
    }
    fn ids_with_prefix(&self, p: &[u8]) -> Result<Vec<Id>, Error> {
        self.0.ids_with_prefix(p)
    }
    fn delete(&mut self, id: &Id) -> Result<(), Error> {
        self.0.delete(id)
    }
    fn get_ref(&self, n: &str) -> Result<Option<Id>, Error> {
        self.0.get_ref(n)
    }
    fn set_ref(&mut self, n: &str, e: Option<Id>, new: Id) -> Result<(), Error> {
        self.0.set_ref(n, e, new)
    }
    fn refs(&self) -> Result<Vec<(String, Id)>, Error> {
        self.0.refs()
    }
}

/// The child: build (if no head) and edit forever, one commit per edit.
fn child(kind: &str, dir: &Path) {
    let files = md_files();
    let shared = shared_of(&corpus(&files));
    let repo = Repo::new(Dyn(open_store(kind, dir)));
    let (mut w, mut v, mut step) = match head(&repo) {
        None => {
            drop(repo);
            let mut w = setup(Dyn(open_store(kind, dir)), &files);
            fast_writes(&mut w.repo);
            let mut v = View::new(&mut w.repo, Mode::Postings);
            let keys: Vec<String> = w.units.keys().cloned().collect();
            let (added, _) = v.diff(&BTreeMap::new(), &w.units.clone(), &keys);
            v.update(&mut w.repo, &added, &[]);
            commit(&mut w.repo, w.tree, &v, "corpus");
            (w, v, 0)
        }
        Some((files_tree, root)) => {
            let mut repo = repo;
            repo.short_records = true;
            fast_writes(&mut repo);
            let steps = repo.log(repo.resolve("main").unwrap()).unwrap().len() - 1;
            let w = world(repo, files_tree, shared);
            let mut v = View::new_empty(Mode::Postings);
            v.func = Id::of_content(&module());
            v.root = root.unwrap();
            for f in w.units.values().flatten() {
                *v.uses.entry(*f).or_default() += 1;
            }
            (w, v, steps)
        }
    };
    loop {
        let old = w.units.clone();
        let changed = edit(&mut w, step, &files);
        let (added, removed) = v.diff(&old, &w.units, &changed);
        v.update(&mut w.repo, &added, &removed);
        commit(&mut w.repo, w.tree, &v, "edit");
        step += 1;
    }
}

fn crash() {
    let exe = std::env::current_exe().unwrap();
    for kind in ["fs", "pack"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir: PathBuf = tmp.path().join("store");
        let mut x: u64 = 0x5851f42d4c957f2d;
        let (mut kills, mut checks, mut last_steps, mut check_ms) = (0, 0, 0, 0.0);
        let mut no_head = 0;
        for _ in 0..30 {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let wait = 150 + (x >> 33) % 900;
            let mut ch = std::process::Command::new(&exe)
                .args(["child", kind, dir.to_str().unwrap()])
                .spawn()
                .unwrap();
            std::thread::sleep(Duration::from_millis(wait));
            ch.kill().unwrap();
            ch.wait().unwrap();
            kills += 1;
            let t = Instant::now();
            let repo = Repo::new(Dyn(open_store(kind, &dir)));
            assert!(
                repo.fsck().unwrap().is_empty(),
                "{kind}: fsck after kill {kills}"
            );
            match head(&repo) {
                None => no_head += 1,
                Some((files_tree, root)) => {
                    let units = units_of(&repo.fragment_index(&files_tree).unwrap());
                    let n = check_view(&repo, &root.unwrap(), &units);
                    assert!(n > 0);
                    last_steps = repo.log(repo.resolve("main").unwrap()).unwrap().len() - 1;
                    checks += 1;
                }
            }
            check_ms += ms(t);
        }
        line(
            "kills",
            format!(
                "\"store\":\"{kind}\",\"kills\":{kills},\"checked_heads\":{checks},\"killed_before_first_commit\":{no_head},\"edits_committed\":{last_steps},\"check_ms_mean\":{:.1},\"all_exact\":true",
                check_ms / kills as f64
            ),
        );
    }
}

fn p50<T: Copy + PartialOrd>(v: &[T]) -> T {
    let mut v = v.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

// ---- E189: scale -------------------------------------------------------------

fn lcg(x: &mut u64) -> u64 {
    *x = x
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *x >> 33
}

/// A synthetic word: a fixed spelling for each index (letters only).
fn word(i: u64) -> String {
    const C: &[u8] = b"bcdfghklmnprstvz";
    const V: &[u8] = b"aeiou";
    let mut s = String::new();
    let mut k = i + 1;
    while k > 0 {
        s.push(C[(k % 16) as usize] as char);
        s.push(V[((k / 16) % 5) as usize] as char);
        k /= 80;
    }
    s.push('n');
    s
}

/// `files` synthetic md files of 8 paragraphs each: words drawn from a
/// 20,000-word vocabulary with a skewed (product of two uniforms) rank, a
/// dollar figure in a third of paragraphs and a date in a fifth.
fn synthetic(files: usize) -> Vec<(String, Vec<u8>)> {
    let mut x = 0x2545f4914f6cdd1d;
    (0..files)
        .map(|i| {
            let mut b = format!("# Report {i}\n\n");
            for _ in 0..8 {
                let n = 30 + lcg(&mut x) % 30;
                let ws: Vec<String> = (0..n)
                    .map(|_| word((lcg(&mut x) % 20_000) * (lcg(&mut x) % 20_000) / 20_000))
                    .collect();
                b.push_str(&ws.join(" "));
                if lcg(&mut x).is_multiple_of(3) {
                    b.push_str(&format!(
                        " ${},{:03}.{:02}",
                        lcg(&mut x) % 900,
                        lcg(&mut x) % 1000,
                        lcg(&mut x) % 100
                    ));
                }
                if lcg(&mut x).is_multiple_of(5) {
                    b.push_str(&format!(
                        " 2026-{:02}-{:02}",
                        1 + lcg(&mut x) % 12,
                        1 + lcg(&mut x) % 28
                    ));
                }
                b.push_str(".\n\n");
            }
            (format!("r/{:03}/{i}.md", i % 997), b.into_bytes())
        })
        .collect()
}

fn scale(nfiles: usize) {
    let files = synthetic(nfiles);
    let bytes: usize = files.iter().map(|f| f.1.len()).sum();
    let tmp = tempfile::tempdir().unwrap();
    for mode in [
        Mode::Memory,
        Mode::Native,
        Mode::Stored,
        Mode::Postings,
        Mode::Bucketed(1),
        Mode::Bucketed(2),
    ] {
        KIND.with(|k| *k.borrow_mut() = format!("pack/{nfiles}"));
        let dir = tmp.path().join(mode.name());
        let c = Arc::new(Counts::default());
        let mut repo = Repo::new(Counted {
            s: Dyn(open_store("pack", &dir)),
            c: c.clone(),
        });
        repo.short_records = true;
        let t = Instant::now();
        let mut edits = Vec::new();
        for (p, b) in &files {
            let id = repo.put_fragmented(b, None).unwrap();
            edits.push(ikam_kernel::Edit::Put {
                path: p.clone(),
                id,
            });
        }
        let tree = repo.edit_tree(None, &edits).unwrap();
        let corpus_ms = ms(t);
        fast_writes(&mut repo);
        let mut w = world(repo, tree, Default::default());
        let mut v = View::new(&mut w.repo, mode);
        let keys: Vec<String> = w.units.keys().cloned().collect();
        let (w0, t) = (c.written.load(Relaxed), Instant::now());
        let (added, _) = v.diff(&BTreeMap::new(), &w.units.clone(), &keys);
        v.update(&mut w.repo, &added, &[]);
        commit(&mut w.repo, w.tree, &v, "corpus");
        sline(
            "scale build",
            format!(
                "\"mode\":\"{}\",\"files\":{nfiles},\"bytes\":{bytes},\"fragments\":{},\"corpus_ms\":{corpus_ms:.0},\"view_ms\":{:.0},\"view_written\":{}",
                mode.name(),
                added.len(),
                ms(t),
                c.written.load(Relaxed) - w0
            ),
        );
        // 20 single-file edits.
        let (mut up_ms, mut up_w, mut up_r) = (Vec::new(), Vec::new(), Vec::new());
        let mut x = 0x9e3779b97f4a7c15u64;
        for step in 0..20 {
            let path = keys[lcg(&mut x) as usize % keys.len()].clone();
            let mut b: Vec<u8> = w.units[&path]
                .iter()
                .flat_map(|u| w.repo.read_content(u).unwrap())
                .collect();
            let rs = blocks(&b);
            let r = rs[rs.len() / 2].clone();
            let at = r.start + b[r].iter().position(|c| *c == b'.').unwrap();
            b.splice(at..at, format!(" revised{step} ${step},500").bytes());
            let id = w.repo.put_fragmented(&b, None).unwrap();
            let old = w.units.clone();
            w.tree = w
                .repo
                .edit_tree(
                    Some(w.tree),
                    &[ikam_kernel::Edit::Put {
                        path: path.clone(),
                        id,
                    }],
                )
                .unwrap();
            w.units
                .insert(path.clone(), w.repo.fragments_of(&id).unwrap().unwrap());
            let (w0, r0, t) = (
                c.written.load(Relaxed),
                c.read.load(Relaxed),
                Instant::now(),
            );
            let (added, removed) = v.diff(&old, &w.units, std::slice::from_ref(&path));
            v.update(&mut w.repo, &added, &removed);
            commit(&mut w.repo, w.tree, &v, "edit");
            up_ms.push(ms(t));
            up_w.push(c.written.load(Relaxed) - w0);
            up_r.push(c.read.load(Relaxed) - r0);
        }
        sline(
            "scale edits",
            format!(
                "\"mode\":\"{}\",\"edits\":20,\"upkeep_ms_p50\":{:.2},\"upkeep_ms_max\":{:.1},\"upkeep_read_p50\":{},\"upkeep_read_max\":{},\"written_p50_incl_commit\":{}",
                mode.name(),
                p50(&up_ms),
                up_ms.iter().cloned().fold(0.0, f64::max),
                p50(&up_r),
                up_r.iter().max().unwrap(),
                p50(&up_w)
            ),
        );
        let units = w.units.clone();
        let words = pick_words(&full_agg(&w.repo, &units));
        let truth = query_files(&w.repo, &units, &words[0]);
        let store_b = store_bytes(w.repo.store());
        drop(w);
        // Restart: first goal query.
        let c = Arc::new(Counts::default());
        let t = Instant::now();
        let repo = Repo::new(Counted {
            s: Dyn(open_store("pack", &dir)),
            c: c.clone(),
        });
        let (files_tree, root) = head(&repo).unwrap();
        repo.derivations(&files_tree).unwrap();
        let ledger_ms = ms(t);
        let units2 = units_of(&repo.fragment_index(&files_tree).unwrap());
        assert_eq!(units2, units);
        let open_ms = ms(t);
        let (r0, t) = (c.read.load(Relaxed), Instant::now());
        let cand: BTreeSet<Id> = match mode {
            Mode::Memory => units
                .values()
                .flatten()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter(|f| {
                    Summary::of(&repo.read_content(f).unwrap())
                        .words
                        .contains_key(&words[0])
                })
                .copied()
                .collect(),
            Mode::Postings | Mode::Bucketed(_) => {
                candidates(&repo, &root.unwrap(), mode, &words[0])
            }
            _ => {
                let v = View {
                    root: root.unwrap(),
                    ..View::new_empty(mode)
                };
                summaries(&repo, &v)
                    .into_iter()
                    .filter(|(_, s)| s.words.contains_key(&words[0]))
                    .map(|e| e.0)
                    .collect()
            }
        };
        assert_eq!(query(&repo, &units, &cand, &words[0]), truth);
        sline(
            "scale restart",
            format!(
                "\"mode\":\"{}\",\"open_ms\":{open_ms:.0},\"ledger_ms\":{ledger_ms:.0},\"objects\":{},\"first_query_ms\":{:.1},\"first_query_read\":{},\"store_bytes\":{store_b},\"exact\":true",
                mode.name(),
                repo.store().ids().unwrap().len(),
                ms(t),
                c.read.load(Relaxed) - r0
            ),
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("child") => return child(&args[2], Path::new(&args[3])),
        Some("crash") => return crash(),
        Some("scale") => return scale(args[2].parse().unwrap()),
        _ => {}
    }
    let files = md_files();
    check_function(&files);
    let tmp = tempfile::tempdir().unwrap();
    for kind in ["fs", "pack", "mem"] {
        let dir = tmp.path().join(kind);
        let base = run(kind, Mode::Memory, &files, &dir.join("memory"), None);
        for mode in [
            Mode::Stored,
            Mode::Postings,
            Mode::NodePostings,
            Mode::Bucketed(1),
            Mode::Bucketed(2),
            Mode::Native,
        ] {
            run(kind, mode, &files, &dir.join(mode.name()), Some(&base));
        }
    }
}
