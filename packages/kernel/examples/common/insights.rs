//! Shared by E182-E183 (`fragment_insights.rs`) and E186-E188
//! (`stored_summaries.rs`): the corpus, per-fragment summaries, the edit
//! stream and its world, generic over the store.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use ikam_kernel::{Edit, FragmentIndex, Id, Repo, Store, blocks};

pub fn md_files() -> Vec<(String, Vec<u8>)> {
    let root: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/cases")
        .canonicalize()
        .unwrap();
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if !p.to_string_lossy().contains("_image_tools") {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == "md") {
                let name = p
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((name, std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

pub const NOTICE: &[u8] =
    b"Notice: figures in this document are unaudited and may change after the quarterly review.\n\n";
pub const FAN: usize = 64;

pub fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Summary {
    pub words: HashMap<Vec<u8>, u32>,
    pub money_n: u32,
    pub money_cents: i64,
    pub dates: u32,
    pub headings: u32,
    pub items: u32,
}

impl Summary {
    pub fn of(b: &[u8]) -> Summary {
        let mut s = Summary::default();
        for w in b
            .split(|c| !c.is_ascii_alphanumeric())
            .filter(|w| w.len() >= 3)
        {
            *s.words.entry(w.to_ascii_lowercase()).or_default() += 1;
        }
        let (n, c) = money(b);
        s.money_n = n;
        s.money_cents = c;
        s.dates = dates(b);
        for line in b.split(|&c| c == b'\n') {
            let t = line.trim_ascii_start();
            if t.starts_with(b"#") {
                s.headings += 1;
            } else if t.starts_with(b"- ")
                || t.starts_with(b"* ")
                || (t.first().is_some_and(u8::is_ascii_digit)
                    && t.iter()
                        .find(|c| !c.is_ascii_digit())
                        .is_some_and(|&c| c == b'.' || c == b')'))
            {
                s.items += 1;
            }
        }
        s
    }

    pub fn add(&mut self, o: &Summary, sign: i64) {
        for (w, n) in &o.words {
            let e = self.words.entry(w.clone()).or_default();
            *e = (*e as i64 + sign * *n as i64) as u32;
            if *e == 0 {
                self.words.remove(w);
            }
        }
        self.money_n = (self.money_n as i64 + sign * o.money_n as i64) as u32;
        self.money_cents += sign * o.money_cents;
        self.dates = (self.dates as i64 + sign * o.dates as i64) as u32;
        self.headings = (self.headings as i64 + sign * o.headings as i64) as u32;
        self.items = (self.items as i64 + sign * o.items as i64) as u32;
    }
}

/// Dollar figures: `$` then digits with optional thousands commas and up to
/// two decimals.
pub fn money(b: &[u8]) -> (u32, i64) {
    let (mut n, mut total) = (0, 0i64);
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
            let mut j = i + 1;
            let mut v: i64 = 0;
            while j < b.len()
                && (b[j].is_ascii_digit()
                    || (b[j] == b',' && b.get(j + 1).is_some_and(u8::is_ascii_digit)))
            {
                if b[j] != b',' {
                    v = v.saturating_mul(10).saturating_add((b[j] - b'0') as i64);
                }
                j += 1;
            }
            let mut cents = v.saturating_mul(100);
            if b.get(j) == Some(&b'.') && b.get(j + 1).is_some_and(u8::is_ascii_digit) {
                let d1 = (b[j + 1] - b'0') as i64;
                let d2 = b
                    .get(j + 2)
                    .filter(|c| c.is_ascii_digit())
                    .map_or(0, |c| (c - b'0') as i64);
                cents += d1 * 10 + d2;
                j += 2;
            }
            n += 1;
            total += cents;
            i = j;
        } else {
            i += 1;
        }
    }
    (n, total)
}

pub fn dates(b: &[u8]) -> u32 {
    b.windows(10)
        .filter(|w| {
            w[..4].iter().all(u8::is_ascii_digit)
                && w[4] == b'-'
                && w[5..7].iter().all(u8::is_ascii_digit)
                && w[7] == b'-'
                && w[8..].iter().all(u8::is_ascii_digit)
        })
        .count() as u32
}

/// Corpus aggregates from file summaries.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Agg {
    pub total: HashMap<Vec<u8>, u64>,
    pub df: HashMap<Vec<u8>, u32>,
    pub money_cents: i64,
    pub money_n: u64,
}

impl Agg {
    pub fn apply(&mut self, s: &Summary, sign: i64) {
        for (w, n) in &s.words {
            let e = self.total.entry(w.clone()).or_default();
            *e = (*e as i64 + sign * *n as i64) as u64;
            if *e == 0 {
                self.total.remove(w);
            }
            let d = self.df.entry(w.clone()).or_default();
            *d = (*d as i64 + sign) as u32;
            if *d == 0 {
                self.df.remove(w);
            }
        }
        self.money_cents += sign * s.money_cents;
        self.money_n = (self.money_n as i64 + sign * s.money_n as i64) as u64;
    }

    pub fn top<V: Copy + Ord>(m: &HashMap<Vec<u8>, V>) -> Vec<Vec<u8>> {
        let mut v: Vec<_> = m.iter().map(|(w, n)| (*n, w.clone())).collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        v.into_iter().take(20).map(|e| e.1).collect()
    }
}

pub struct World<S: Store> {
    pub repo: Repo<S>,
    pub tree: Id,
    pub ix: FragmentIndex,
    pub shared: HashSet<Id>,
    /// Each file's units now.
    pub units: BTreeMap<String, Vec<Id>>,
}

pub fn units_of(ix: &FragmentIndex) -> BTreeMap<String, Vec<Id>> {
    ix.files
        .iter()
        .map(|(p, (id, fs))| {
            (
                p.clone(),
                if fs.is_empty() { vec![*id] } else { fs.clone() },
            )
        })
        .collect()
}

/// The corpus: the cases' md files plus the notice in `FAN` copies.
pub fn corpus(files: &[(String, Vec<u8>)]) -> Vec<(String, Vec<u8>)> {
    let mut corpus = files.to_vec();
    for (i, (p, b)) in files.iter().cycle().take(FAN).enumerate() {
        let rs = blocks(b);
        let at = rs.get(rs.len() / 2).map_or(b.len(), |r| r.start);
        let mut v = b[..at].to_vec();
        v.extend_from_slice(NOTICE);
        v.extend_from_slice(&b[at..]);
        corpus.push((format!("fan/{i}/{p}"), v));
    }
    corpus
}

/// Blocks found in more than one file of the corpus.
pub fn shared_of(corpus: &[(String, Vec<u8>)]) -> HashSet<Id> {
    let mut seen: HashMap<Id, usize> = HashMap::new();
    for (_, b) in corpus {
        let mine: HashSet<Id> = blocks(b)
            .into_iter()
            .map(|r| Id::of_content(&b[r]))
            .collect();
        for id in mine {
            *seen.entry(id).or_default() += 1;
        }
    }
    seen.into_iter().filter(|e| e.1 > 1).map(|e| e.0).collect()
}

/// The corpus stored in `store` as fragments (not committed).
pub fn setup<S: Store>(store: S, files: &[(String, Vec<u8>)]) -> World<S> {
    let corpus = corpus(files);
    let shared = shared_of(&corpus);
    let mut repo = Repo::new(store);
    repo.short_records = true;
    let mut edits = Vec::new();
    for (p, b) in &corpus {
        let id = repo.put_fragmented(b, Some(&shared)).unwrap();
        edits.push(Edit::Put {
            path: p.clone(),
            id,
        });
    }
    let tree = repo.edit_tree(None, &edits).unwrap();
    world(repo, tree, shared)
}

/// A world over a stored file tree.
pub fn world<S: Store>(repo: Repo<S>, tree: Id, shared: HashSet<Id>) -> World<S> {
    let ix = repo.fragment_index(&tree).unwrap();
    let units = units_of(&ix);
    World {
        repo,
        tree,
        ix,
        shared,
        units,
    }
}

/// Everything from every file: (file summaries, aggregates, bytes read).
pub fn full<S: Store>(w: &World<S>) -> (BTreeMap<String, Summary>, Agg, usize) {
    let mut read = 0;
    let mut files = BTreeMap::new();
    let mut agg = Agg::default();
    for (p, us) in &w.units {
        let mut b = Vec::new();
        for u in us {
            b.extend_from_slice(&w.repo.read_content(u).unwrap());
        }
        read += b.len();
        let s = Summary::of(&b);
        agg.apply(&s, 1);
        files.insert(p.clone(), s);
    }
    (files, agg, read)
}

/// One edit to the file tree (not committed); returns the paths it
/// changed (their units are updated).
pub fn edit<S: Store>(w: &mut World<S>, step: usize, files: &[(String, Vec<u8>)]) -> Vec<String> {
    let (edits, kind) = if step == 0 {
        let new: Vec<u8> = [NOTICE.trim_ascii_end(), b" (revised; $2,500 reserve)\n\n"].concat();
        (
            w.repo
                .propagate(&mut w.ix, &Id::of_content(NOTICE), &new)
                .unwrap(),
            "propagate",
        )
    } else if step == 1 {
        // A block several case files share (not the notice): the most
        // shared one with a word in it.
        let mut best: Option<(usize, Id)> = None;
        for (f, ps) in &w.ix.by_fragment {
            let real = ps.iter().filter(|p| !p.starts_with("fan/")).count();
            if real > 1 && best.is_none_or(|b| real > b.0 || (real == b.0 && *f < b.1)) {
                let b = w.repo.read_content(f).unwrap();
                if b.iter().any(u8::is_ascii_alphabetic) && b.len() > 20 {
                    best = Some((real, *f));
                }
            }
        }
        let (_, f) = best.unwrap();
        let mut b = w.repo.read_content(&f).unwrap();
        let at = b.iter().position(|c| *c == b'\n').unwrap_or(b.len());
        b.splice(at..at, b" (owner: finance, $750)".iter().copied());
        (w.repo.propagate(&mut w.ix, &f, &b).unwrap(), "propagate")
    } else {
        let mut x: u64 = 0x2545f4914f6cdd1d ^ step as u64;
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let path = &files[(x >> 33) as usize % files.len()].0;
        let mut b = Vec::new();
        for u in &w.units[path] {
            b.extend_from_slice(&w.repo.read_content(u).unwrap());
        }
        let rs = blocks(&b);
        let r = rs[rs.len() / 2].clone();
        let ins = format!(" revised{step} (approved $1,{step:03}.50)");
        let at = r.start
            + b[r.clone()]
                .iter()
                .position(|c| *c == b'\n')
                .unwrap_or(r.len());
        b.splice(at..at, ins.bytes());
        let id = w.repo.put_fragmented(&b, Some(&w.shared)).unwrap();
        (
            vec![Edit::Put {
                path: path.clone(),
                id,
            }],
            "file",
        )
    };
    let _ = kind;
    let mut changed = Vec::new();
    for e in &edits {
        if let Edit::Put { path, id } = e {
            let us = w
                .repo
                .fragments_of(id)
                .unwrap()
                .unwrap_or_else(|| vec![*id]);
            w.units.insert(path.clone(), us);
            changed.push(path.clone());
        }
    }
    w.tree = w.repo.edit_tree(Some(w.tree), &edits).unwrap();
    changed
}
