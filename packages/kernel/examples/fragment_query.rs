//! E160: queries and insights over fragments, and keeping them fresh after
//! a propagated edit.
//!
//! `cargo run --release --example fragment_query`
//!
//! Store: the cases' md files plus a notice paragraph inserted into 64
//! copies (`fan/`), stored as renders with shared blocks standing alone
//! (E157). A file's units are its top-level fragments (a file of one
//! fragment is its own unit).
//!
//! Search (which files contain a byte pattern), for 40 patterns drawn from
//! the corpus plus the notice phrase, each answer checked against a scan:
//! - `scan`: `Repo::search` with no index (decodes every file);
//! - `stored`: `build_search_index` (E058/E094, whichever it picks);
//! - `frag-scan`: scan each distinct fragment once and each distinct seam
//!   (the pattern's length less one on each side of a fragment boundary);
//! - `frag-tri`: a trigram index over fragments (and seams), candidates
//!   scanned.
//!
//! Insights: per-fragment word counts, summed through each file's fragment
//! list, so "how often does each file use this word" and "which files use
//! it" never read a file; checked against counting each file directly.
//!
//! Freshness after the notice is revised (one propagate over 64 files):
//! the stored index is either rebuilt or left stale (stale files are then
//! scanned); the fragment indexes add the one new fragment and its seams.
//!
//! Goal-scoped reads: for a word, the bytes read to return the paragraphs
//! that hold it (fragments) vs the files that hold it (whole files).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use ikam_kernel::{Edit, FragmentIndex, Id, MemStore, Repo, blocks};

fn md_files() -> Vec<(String, Vec<u8>)> {
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

const NOTICE: &[u8] =
    b"Notice: figures in this document are unaudited and may change after the quarterly review.\n\n";
const FAN: usize = 64;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn trigrams(b: &[u8]) -> HashSet<[u8; 3]> {
    b.windows(3).map(|w| [w[0], w[1], w[2]]).collect()
}

fn words(b: &[u8]) -> impl Iterator<Item = &[u8]> {
    b.split(|c| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 3)
}

fn word_counts(b: &[u8]) -> HashMap<Vec<u8>, u32> {
    let mut m = HashMap::new();
    for w in words(b) {
        *m.entry(w.to_ascii_lowercase()).or_default() += 1;
    }
    m
}

/// Each file's units: its top-level fragments, or itself.
fn units(ix: &FragmentIndex) -> BTreeMap<String, Vec<Id>> {
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

/// Fragments and seams, scanned once each: which files hold `pat`.
struct FragSearch {
    frag: HashMap<Id, Vec<u8>>,
    /// Seams by neighbouring pair: the last and first `SEAM` bytes.
    seam: HashMap<(Id, Id), Vec<u8>>,
    tri: HashMap<[u8; 3], Vec<u32>>,
    keys: Vec<Key>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Key {
    Frag(Id),
    Seam(Id, Id),
}

const SEAM: usize = 63;

impl FragSearch {
    fn new() -> Self {
        FragSearch {
            frag: HashMap::new(),
            seam: HashMap::new(),
            tri: HashMap::new(),
            keys: vec![],
        }
    }

    fn add_key(&mut self, k: Key, bytes: &[u8]) {
        let n = self.keys.len() as u32;
        self.keys.push(k);
        for t in trigrams(bytes) {
            self.tri.entry(t).or_default().push(n);
        }
    }

    /// Add whatever of `units` is new; returns bytes read.
    fn add(&mut self, repo: &Repo<MemStore>, units: &BTreeMap<String, Vec<Id>>) -> usize {
        let mut read = 0;
        for fs in units.values() {
            for f in fs {
                if !self.frag.contains_key(f) {
                    let b = repo.read_content(f).unwrap();
                    read += b.len();
                    self.add_key(Key::Frag(*f), &b);
                    self.frag.insert(*f, b);
                }
            }
            for w in fs.windows(2) {
                let k = (w[0], w[1]);
                if !self.seam.contains_key(&k) {
                    let (a, b) = (&self.frag[&w[0]], &self.frag[&w[1]]);
                    let mut s = a[a.len().saturating_sub(SEAM)..].to_vec();
                    s.extend_from_slice(&b[..b.len().min(SEAM)]);
                    self.add_key(Key::Seam(w[0], w[1]), &s);
                    self.seam.insert(k, s);
                }
            }
        }
        read
    }

    fn bytes(&self, k: &Key) -> &[u8] {
        match k {
            Key::Frag(f) => &self.frag[f],
            Key::Seam(a, b) => &self.seam[&(*a, *b)],
        }
    }

    /// Files holding `pat` (at most SEAM + 1 bytes): returns paths, bytes
    /// scanned.
    fn search(
        &self,
        pat: &[u8],
        units: &BTreeMap<String, Vec<Id>>,
        use_tri: bool,
    ) -> (Vec<String>, usize) {
        let has = |b: &[u8]| b.windows(pat.len()).any(|w| w == pat);
        let mut scanned = 0;
        let mut hits: HashSet<Key> = HashSet::new();
        if use_tri && pat.len() >= 3 {
            let mut cand: Option<HashSet<u32>> = None;
            for t in trigrams(pat) {
                let s: HashSet<u32> = self
                    .tri
                    .get(&t)
                    .map(|v| v.iter().copied().collect())
                    .unwrap_or_default();
                cand = Some(match cand {
                    None => s,
                    Some(c) => c.intersection(&s).copied().collect(),
                });
            }
            for n in cand.unwrap_or_default() {
                let k = self.keys[n as usize];
                let b = self.bytes(&k);
                scanned += b.len();
                if has(b) {
                    hits.insert(k);
                }
            }
        } else {
            for k in &self.keys {
                let b = self.bytes(k);
                scanned += b.len();
                if has(b) {
                    hits.insert(*k);
                }
            }
        }
        let out = units
            .iter()
            .filter(|(_, fs)| {
                fs.iter().any(|f| hits.contains(&Key::Frag(*f)))
                    || fs.windows(2).any(|w| hits.contains(&Key::Seam(w[0], w[1])))
            })
            .map(|e| e.0.clone())
            .collect();
        (out, scanned)
    }

    /// The trigram index's encoded size, as the stored trigram index
    /// encodes it: per trigram, 3 bytes and delta varints.
    fn tri_bytes(&self) -> usize {
        let varint = |mut v: u32| {
            let mut n = 1;
            while v >= 128 {
                v >>= 7;
                n += 1;
            }
            n
        };
        let keys = self.keys.len() * 33;
        let lists: usize = self
            .tri
            .values()
            .map(|v| {
                let mut prev = 0;
                3 + varint(v.len() as u32)
                    + v.iter()
                        .map(|&x| {
                            let d = x - prev;
                            prev = x;
                            varint(d)
                        })
                        .sum::<usize>()
            })
            .sum();
        keys + lists
    }
}

struct Store {
    repo: Repo<MemStore>,
    tree: Id,
    ix: FragmentIndex,
}

fn setup(files: &[(String, Vec<u8>)]) -> Store {
    let mut corpus = files.to_vec();
    for (i, (p, b)) in files.iter().cycle().take(FAN).enumerate() {
        let rs = blocks(b);
        let at = rs.get(rs.len() / 2).map_or(b.len(), |r| r.start);
        let mut v = b[..at].to_vec();
        v.extend_from_slice(NOTICE);
        v.extend_from_slice(&b[at..]);
        corpus.push((format!("fan/{i}/{p}"), v));
    }
    let mut seen: HashMap<Id, usize> = HashMap::new();
    for (_, b) in &corpus {
        let mine: HashSet<Id> = blocks(b)
            .into_iter()
            .map(|r| Id::of_content(&b[r]))
            .collect();
        for id in mine {
            *seen.entry(id).or_default() += 1;
        }
    }
    let shared: HashSet<Id> = seen.into_iter().filter(|e| e.1 > 1).map(|e| e.0).collect();
    let mut repo = Repo::new(MemStore::default());
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
    repo.commit("main", tree, "corpus").unwrap();
    let ix = repo.fragment_index(&tree).unwrap();
    Store { repo, tree, ix }
}

/// 40 patterns from the corpus (a word or two neighbouring words, picked
/// by a fixed LCG), plus the notice phrase.
fn patterns(files: &[(String, Vec<u8>)]) -> Vec<Vec<u8>> {
    let all: Vec<&[u8]> = files.iter().flat_map(|f| words(&f.1)).collect();
    let mut x: u64 = 0x9e3779b97f4a7c15;
    let mut out = Vec::new();
    while out.len() < 40 {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let i = (x >> 33) as usize % (all.len() - 1);
        let w = if out.len() % 2 == 0 {
            all[i].to_vec()
        } else {
            [all[i], b" ", all[i + 1]].concat()
        };
        if w.len() >= 5 && w.len() <= SEAM + 1 && !out.contains(&w) {
            out.push(w);
        }
    }
    out.push(b"unaudited and may change".to_vec());
    out
}

fn main() {
    let files = md_files();
    let pats = patterns(&files);
    let mut s = setup(&files);
    let paths_of = |ix: &FragmentIndex, ids: &[Id]| -> Vec<String> {
        let set: HashSet<&Id> = ids.iter().collect();
        ix.files
            .iter()
            .filter(|(_, v)| set.contains(&v.0))
            .map(|e| e.0.clone())
            .collect()
    };
    let truth = |repo: &Repo<MemStore>, ix: &FragmentIndex, p: &[u8]| -> Vec<String> {
        ix.files
            .iter()
            .filter(|(_, v)| {
                repo.read_content(&v.0)
                    .unwrap()
                    .windows(p.len())
                    .any(|w| w == p)
            })
            .map(|e| e.0.clone())
            .collect()
    };
    let file_ids = |ix: &FragmentIndex| -> Vec<Id> { ix.files.values().map(|v| v.0).collect() };
    let ids = file_ids(&s.ix);
    let total: usize = ids
        .iter()
        .map(|id| s.repo.read_content(id).unwrap().len())
        .sum();
    let truths: Vec<Vec<String>> = pats.iter().map(|p| truth(&s.repo, &s.ix, p)).collect();
    let line = |what: &str, extra: String| println!("{{\"what\":\"{what}\",{extra}}}");
    line(
        "corpus",
        format!(
            "\"files\":{},\"bytes\":{total},\"patterns\":{}",
            ids.len(),
            pats.len()
        ),
    );

    // Search.
    let t = Instant::now();
    let mut work = 0;
    for (p, tr) in pats.iter().zip(&truths) {
        let (hit, w) = s.repo.search(p, &ids, None).unwrap();
        work += w;
        assert_eq!(&paths_of(&s.ix, &hit), tr);
    }
    line(
        "search scan",
        format!(
            "\"ms_per_query\":{:.3},\"decode_per_query\":{}",
            ms(t) / pats.len() as f64,
            work / pats.len()
        ),
    );

    let t = Instant::now();
    let idx_id = s.repo.build_search_index(&ids).unwrap();
    let build = ms(t);
    let idx_bytes = s.repo.read_content(&idx_id).unwrap().len();
    let index = s.repo.search_index().unwrap();
    let t = Instant::now();
    let mut work = 0;
    for (p, tr) in pats.iter().zip(&truths) {
        let (hit, w) = s.repo.search(p, &ids, index.as_ref()).unwrap();
        work += w;
        assert_eq!(&paths_of(&s.ix, &hit), tr);
    }
    line(
        "search stored",
        format!(
            "\"build_ms\":{build:.2},\"index_bytes\":{idx_bytes},\"ms_per_query\":{:.3},\"decode_per_query\":{}",
            ms(t) / pats.len() as f64,
            work / pats.len()
        ),
    );

    let u = units(&s.ix);
    let mut fsr = FragSearch::new();
    let t = Instant::now();
    let read = fsr.add(&s.repo, &u);
    let build = ms(t);
    let frag_bytes: usize = fsr.frag.values().map(|b| b.len()).sum();
    let seam_bytes: usize = fsr.seam.values().map(|b| b.len()).sum();
    for tri in [false, true] {
        let t = Instant::now();
        let mut scanned = 0;
        for (p, tr) in pats.iter().zip(&truths) {
            let (hit, n) = fsr.search(p, &u, tri);
            scanned += n;
            assert_eq!(&hit, tr, "{}", String::from_utf8_lossy(p));
        }
        let name = if tri {
            "search frag-tri"
        } else {
            "search frag-scan"
        };
        line(
            name,
            format!(
                "\"build_ms\":{build:.2},\"read_to_build\":{read},\"fragments\":{},\"fragment_bytes\":{frag_bytes},\"seams\":{},\"seam_bytes\":{seam_bytes},\"index_bytes\":{},\"ms_per_query\":{:.3},\"scanned_per_query\":{}",
                fsr.frag.len(),
                fsr.seam.len(),
                if tri { fsr.tri_bytes() } else { 0 },
                ms(t) / pats.len() as f64,
                scanned / pats.len()
            ),
        );
    }

    // Insights: per-fragment word counts.
    let t = Instant::now();
    let mut per_frag: HashMap<Id, HashMap<Vec<u8>, u32>> = HashMap::new();
    for f in fsr.frag.keys() {
        per_frag.insert(*f, word_counts(&fsr.frag[f]));
    }
    let build = ms(t);
    let t = Instant::now();
    let mut direct: BTreeMap<String, HashMap<Vec<u8>, u32>> = BTreeMap::new();
    for (p, (id, _)) in &s.ix.files {
        direct.insert(p.clone(), word_counts(&s.repo.read_content(id).unwrap()));
    }
    let direct_ms = ms(t);
    let sum_file = |per: &HashMap<Id, HashMap<Vec<u8>, u32>>, fs: &[Id]| {
        let mut m: HashMap<Vec<u8>, u32> = HashMap::new();
        for f in fs {
            for (w, n) in &per[f] {
                *m.entry(w.clone()).or_default() += n;
            }
        }
        m
    };
    let t = Instant::now();
    for (p, fs) in &u {
        assert_eq!(sum_file(&per_frag, fs), direct[p], "{p}");
    }
    let sum_ms = ms(t);
    // One word query: which files use it, how often, from counts alone.
    let word = b"figures".to_vec();
    let t = Instant::now();
    let users: Vec<(&String, u32)> = u
        .iter()
        .filter_map(|(p, fs)| {
            let n: u32 = fs.iter().filter_map(|f| per_frag[f].get(&word)).sum();
            (n > 0).then_some((p, n))
        })
        .collect();
    let q_ms = ms(t);
    let t = Instant::now();
    let users_direct = direct.iter().filter(|(_, m)| m.contains_key(&word)).count();
    let qd_ms = ms(t);
    assert_eq!(users.len(), users_direct);
    line(
        "insight word counts",
        format!(
            "\"fragment_build_ms\":{build:.2},\"file_build_ms\":{direct_ms:.2},\"all_files_from_fragments_ms\":{sum_ms:.2},\"word_query_ms\":{q_ms:.3},\"word_query_direct_ms\":{qd_ms:.3},\"files_using_word\":{}",
            users.len()
        ),
    );

    // Goal-scoped reads: paragraphs holding a word vs the files holding it.
    let mut frag_read = 0;
    let mut file_read = 0;
    for p in &pats {
        let (hit, _) = fsr.search(p, &u, true);
        for path in &hit {
            file_read += s.repo.read_content(&s.ix.files[path].0).unwrap().len();
        }
        let mut seen = HashSet::new();
        for path in &hit {
            for f in &u[path] {
                if seen.insert(*f) && fsr.frag[f].windows(p.len()).any(|w| w == &p[..]) {
                    frag_read += fsr.frag[f].len();
                }
            }
        }
    }
    line(
        "goal reads",
        format!(
            "\"fragment_bytes_per_query\":{},\"file_bytes_per_query\":{}",
            frag_read / pats.len(),
            file_read / pats.len()
        ),
    );

    // Freshness after one propagated edit.
    let old = Id::of_content(NOTICE);
    let new: Vec<u8> = [NOTICE.trim_ascii_end(), b" (revised)\n\n"].concat();
    let edits = s.repo.propagate(&mut s.ix, &old, &new).unwrap();
    let tree = s.repo.edit_tree(Some(s.tree), &edits).unwrap();
    s.repo.commit("main", tree, "notice").unwrap();
    let ids2 = file_ids(&s.ix);
    let u2 = units(&s.ix);
    let mut pats2 = pats.clone();
    pats2.push(b"(revised)".to_vec());
    let truths2: Vec<Vec<String>> = pats2.iter().map(|p| truth(&s.repo, &s.ix, p)).collect();
    // Stale stored index: files it does not cover are scanned.
    let t = Instant::now();
    let mut work = 0;
    for (p, tr) in pats2.iter().zip(&truths2) {
        let (hit, w) = s.repo.search(p, &ids2, index.as_ref()).unwrap();
        work += w;
        assert_eq!(&paths_of(&s.ix, &hit), tr);
    }
    line(
        "after edit stored stale",
        format!(
            "\"ms_per_query\":{:.3},\"decode_per_query\":{}",
            ms(t) / pats2.len() as f64,
            work / pats2.len()
        ),
    );
    let t = Instant::now();
    s.repo.build_search_index(&ids2).unwrap();
    let rebuild = ms(t);
    let index2 = s.repo.search_index().unwrap();
    let t = Instant::now();
    for (p, tr) in pats2.iter().zip(&truths2) {
        assert_eq!(
            &paths_of(&s.ix, &s.repo.search(p, &ids2, index2.as_ref()).unwrap().0),
            tr
        );
    }
    line(
        "after edit stored rebuilt",
        format!(
            "\"rebuild_ms\":{rebuild:.2},\"ms_per_query\":{:.3}",
            ms(t) / pats2.len() as f64
        ),
    );
    let t = Instant::now();
    let read = fsr.add(&s.repo, &u2);
    let mut fresh = 0;
    for f in u2.values().flatten() {
        if !per_frag.contains_key(f) {
            per_frag.insert(*f, word_counts(&fsr.frag[f]));
            fresh += 1;
        }
    }
    let upd = ms(t);
    for (p, tr) in pats2.iter().zip(&truths2) {
        assert_eq!(&fsr.search(p, &u2, true).0, tr);
    }
    for (p, fs) in &u2 {
        let id = s.ix.files[p].0;
        assert_eq!(
            sum_file(&per_frag, fs),
            word_counts(&s.repo.read_content(&id).unwrap())
        );
    }
    line(
        "after edit fragments updated",
        format!(
            "\"update_ms\":{upd:.3},\"read\":{read},\"new_fragments\":{fresh},\"files_reached\":{}",
            edits.len()
        ),
    );
}
