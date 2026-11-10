//! E182 and E183: insights kept per fragment, kept fresh through
//! propagated edits, and goal-scoped insight queries.
//!
//! `cargo run --release --example fragment_insights`
//!
//! Store: E160's corpus (the cases' md files plus a notice paragraph in 64
//! copies under `fan/`, 407 files), md as render-runs (E157). A file's units
//! are its top-level fragments (a file of one fragment is its own unit).
//!
//! E182, summaries. Per fragment: word counts (lowercased words of three or
//! more letters or digits), dollar figures (count and total in cents),
//! ISO dates, headings, list items. A file's summary is the sum over its
//! fragments; the corpus aggregates are each word's total and document
//! frequency, the top 20 words by each, and the dollar total. Fragments are
//! paragraph-aligned and these tokens never cross a blank line, so sums are
//! exact; every refresh is checked against a full recompute.
//!
//! Edit stream (22 edits): the notice revised by one `propagate` (64
//! files); a block that several case files share revised by one
//! `propagate`; then 20 single-file edits (a block in a file picked by a
//! fixed LCG gets a word and a dollar figure). Policies:
//! - `full`: recompute everything from every file after each edit;
//! - `eager`: after each edit, summarise only the fragments not seen
//!   before, and apply each changed file's difference to the aggregates;
//! - `lazy`: mark changed files only; measured for staleness after each edit
//!   (files whose summary is wrong, top-20 overlap, dollar total and
//!   document-frequency errors), and for the cost of one refresh at the end.
//!
//! E183, goal-scoped queries. For 12 words (picked by a fixed LCG among
//! words found in 5 to 120 files): the dollar figures and dates in the
//! paragraphs that mention the word, per file and in all. `fragments`
//! reads only the fragments whose summary holds the word; `files` reads
//! every file. Both answers checked equal. Plus a query answered from
//! summaries alone (files that mention the word and cite over $100,000).

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

#[derive(Clone, Default, PartialEq, Debug)]
struct Summary {
    words: HashMap<Vec<u8>, u32>,
    money_n: u32,
    money_cents: i64,
    dates: u32,
    headings: u32,
    items: u32,
}

impl Summary {
    fn of(b: &[u8]) -> Summary {
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

    fn add(&mut self, o: &Summary, sign: i64) {
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
fn money(b: &[u8]) -> (u32, i64) {
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

fn dates(b: &[u8]) -> u32 {
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
struct Agg {
    total: HashMap<Vec<u8>, u64>,
    df: HashMap<Vec<u8>, u32>,
    money_cents: i64,
    money_n: u64,
}

impl Agg {
    fn apply(&mut self, s: &Summary, sign: i64) {
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

    fn top<V: Copy + Ord>(m: &HashMap<Vec<u8>, V>) -> Vec<Vec<u8>> {
        let mut v: Vec<_> = m.iter().map(|(w, n)| (*n, w.clone())).collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        v.into_iter().take(20).map(|e| e.1).collect()
    }
}

struct World {
    repo: Repo<MemStore>,
    tree: Id,
    ix: FragmentIndex,
    shared: HashSet<Id>,
    /// Each file's units now.
    units: BTreeMap<String, Vec<Id>>,
}

fn units_of(ix: &FragmentIndex) -> BTreeMap<String, Vec<Id>> {
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

fn setup(files: &[(String, Vec<u8>)]) -> World {
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
    let units = units_of(&ix);
    World {
        repo,
        tree,
        ix,
        shared,
        units,
    }
}

/// Fragment summaries, cached by id; returns bytes read for new ones.
#[derive(Clone, Default)]
struct Cache {
    frag: HashMap<Id, Summary>,
    /// Which fragments hold each word (for goal-scoped queries).
    by_word: HashMap<Vec<u8>, HashSet<Id>>,
}

impl Cache {
    fn ensure(&mut self, repo: &Repo<MemStore>, ids: &[Id]) -> usize {
        let mut read = 0;
        for f in ids {
            if !self.frag.contains_key(f) {
                let b = repo.read_content(f).unwrap();
                read += b.len();
                let s = Summary::of(&b);
                for w in s.words.keys() {
                    self.by_word.entry(w.clone()).or_default().insert(*f);
                }
                self.frag.insert(*f, s);
            }
        }
        read
    }

    fn file(&self, units: &[Id]) -> Summary {
        let mut s = Summary::default();
        for f in units {
            s.add(&self.frag[f], 1);
        }
        s
    }
}

/// Everything from every file: (file summaries, aggregates, bytes read).
fn full(w: &World) -> (BTreeMap<String, Summary>, Agg, usize) {
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

fn line(what: &str, extra: String) {
    println!("{{\"what\":\"{what}\",{extra}}}");
}

/// One edit; returns the paths it changed (their units are updated).
fn edit(w: &mut World, step: usize, files: &[(String, Vec<u8>)]) -> Vec<String> {
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
    w.repo.commit("main", w.tree, "edit").unwrap();
    changed
}

fn main() {
    let files = md_files();
    let mut w = setup(&files);
    let corpus_bytes: usize = full(&w).2;
    line(
        "corpus",
        format!("\"files\":{},\"bytes\":{corpus_bytes}", w.units.len()),
    );

    // Build: summaries per fragment, then files and aggregates.
    let t = Instant::now();
    let (truth_files, truth_agg, read_full) = full(&w);
    let full_ms = ms(t);
    let mut cache = Cache::default();
    let t = Instant::now();
    let all: Vec<Id> = w.units.values().flatten().copied().collect();
    let read_frag = cache.ensure(&w.repo, &all);
    let mut file_sum: BTreeMap<String, Summary> = BTreeMap::new();
    let mut agg = Agg::default();
    for (p, us) in &w.units {
        let s = cache.file(us);
        agg.apply(&s, 1);
        file_sum.insert(p.clone(), s);
    }
    let frag_ms = ms(t);
    assert_eq!(file_sum, truth_files);
    assert_eq!(agg, truth_agg);
    line(
        "build",
        format!(
            "\"full_ms\":{full_ms:.2},\"full_read\":{read_full},\"fragment_ms\":{frag_ms:.2},\"fragment_read\":{read_frag},\"fragments\":{}",
            cache.frag.len()
        ),
    );

    // Edit stream.
    let mut lazy_cache = cache.clone();
    let lazy_files = file_sum.clone();
    let lazy_agg = agg.clone();
    let mut dirty: HashSet<String> = HashSet::new();
    let (mut tot_full_ms, mut tot_full_read, mut tot_eager_ms, mut tot_eager_read) =
        (0.0, 0usize, 0.0, 0usize);
    for step in 0..22 {
        let changed = edit(&mut w, step, &files);
        dirty.extend(changed.iter().cloned());
        // full
        let t = Instant::now();
        let (tf, ta, r) = full(&w);
        let fm = ms(t);
        // eager
        let t = Instant::now();
        let mut read = 0;
        for p in &changed {
            read += cache.ensure(&w.repo, &w.units[p]);
            let new = cache.file(&w.units[p]);
            let old = file_sum.insert(p.clone(), new.clone()).unwrap();
            agg.apply(&old, -1);
            agg.apply(&new, 1);
        }
        let em = ms(t);
        assert_eq!(file_sum, tf, "step {step}");
        assert_eq!(agg, ta, "step {step}");
        // lazy staleness (nothing refreshed since the build)
        let wrong = lazy_files
            .iter()
            .filter(|(p, s)| tf.get(*p) != Some(*s))
            .count();
        let overlap = |a: &[Vec<u8>], b: &[Vec<u8>]| a.iter().filter(|x| b.contains(x)).count();
        let df_wrong = ta
            .df
            .iter()
            .filter(|(k, v)| lazy_agg.df.get(*k) != Some(*v))
            .count()
            + lazy_agg
                .df
                .keys()
                .filter(|k| !ta.df.contains_key(*k))
                .count();
        line(
            "edit",
            format!(
                "\"step\":{step},\"files_changed\":{},\"full_ms\":{fm:.2},\"full_read\":{r},\"eager_ms\":{em:.3},\"eager_read\":{read},\"lazy_files_wrong\":{wrong},\"lazy_top20_total_overlap\":{},\"lazy_top20_df_overlap\":{},\"lazy_df_wrong\":{df_wrong},\"lazy_money_error_cents\":{}",
                changed.len(),
                overlap(&Agg::top(&lazy_agg.total), &Agg::top(&ta.total)),
                overlap(&Agg::top(&lazy_agg.df), &Agg::top(&ta.df)),
                ta.money_cents - lazy_agg.money_cents
            ),
        );
        tot_full_ms += fm;
        tot_full_read += r;
        tot_eager_ms += em;
        tot_eager_read += read;
    }
    // Lazy refresh at the end: the dirty files only.
    let mut lf = lazy_files;
    let mut la = lazy_agg;
    // The lazy cache holds only the build's fragments.
    let t = Instant::now();
    let mut lazy_read = 0;
    for p in &dirty {
        lazy_read += lazy_cache.ensure(&w.repo, &w.units[p]);
        let new = lazy_cache.file(&w.units[p]);
        let old = lf.insert(p.clone(), new.clone()).unwrap();
        la.apply(&old, -1);
        la.apply(&new, 1);
    }
    let lm = ms(t);
    let (tf, ta, _) = full(&w);
    assert_eq!(lf, tf);
    assert_eq!(la, ta);
    line(
        "stream",
        format!(
            "\"edits\":22,\"full_ms\":{tot_full_ms:.1},\"full_read\":{tot_full_read},\"eager_ms\":{tot_eager_ms:.2},\"eager_read\":{tot_eager_read},\"lazy_refresh_ms\":{lm:.3},\"lazy_refresh_read\":{lazy_read},\"lazy_dirty_files\":{},\"exact\":true",
            dirty.len()
        ),
    );

    // E183: goal-scoped queries.
    let vocab: Vec<&Vec<u8>> = {
        let mut v: Vec<_> = agg
            .df
            .iter()
            .filter(|(k, d)| (5..=120).contains(*d) && k.iter().all(u8::is_ascii_alphabetic))
            .map(|e| e.0)
            .collect();
        v.sort();
        v
    };
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
    let mentions = |blk: &[u8], wd: &[u8]| {
        blk.split(|c| !c.is_ascii_alphanumeric())
            .any(|t| t.len() >= 3 && t.eq_ignore_ascii_case(wd))
    };
    // Per block holding the word: (money count, cents, dates).
    let answer = |b: &[u8], wd: &[u8]| -> (u32, i64, u32) {
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
    };
    let (mut fr_read, mut fl_read, mut fr_ms, mut fl_ms) = (0usize, 0usize, 0.0, 0.0);
    let mut hits_total = 0;
    for wd in &words {
        // files: read every file.
        let t = Instant::now();
        let mut by_file: BTreeMap<String, (u32, i64, u32)> = BTreeMap::new();
        for (p, us) in &w.units {
            let mut b = Vec::new();
            for u in us {
                b.extend_from_slice(&w.repo.read_content(u).unwrap());
            }
            fl_read += b.len();
            let a = answer(&b, wd);
            if a != (0, 0, 0) || mentions(&b, wd) {
                by_file.insert(p.clone(), a);
            }
        }
        fl_ms += ms(t);
        // fragments: only those whose summary holds the word, each once.
        let t = Instant::now();
        let cand = cache.by_word.get(wd).cloned().unwrap_or_default();
        let mut per_frag: HashMap<Id, (u32, i64, u32)> = HashMap::new();
        for f in &cand {
            let b = w.repo.read_content(f).unwrap();
            fr_read += b.len();
            per_frag.insert(*f, answer(&b, wd));
        }
        let mut got: BTreeMap<String, (u32, i64, u32)> = BTreeMap::new();
        for (p, us) in &w.units {
            let mut a = (0, 0, 0);
            let mut hit = false;
            for u in us {
                if let Some(x) = per_frag.get(u) {
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
        fr_ms += ms(t);
        assert_eq!(got, by_file, "{}", String::from_utf8_lossy(wd));
        hits_total += got.len();
    }
    let q = words.len() as f64;
    line(
        "goal query",
        format!(
            "\"words\":{},\"files_per_query\":{:.1},\"files_ms\":{:.3},\"files_read\":{},\"fragments_ms\":{:.3},\"fragments_read\":{},\"exact\":true",
            words.len(),
            hits_total as f64 / q,
            fl_ms / q,
            fl_read / words.len(),
            fr_ms / q,
            fr_read / words.len()
        ),
    );
    // From summaries alone: files that mention the word and cite over $100,000.
    let t = Instant::now();
    let mut n = 0;
    for wd in &words {
        n += file_sum
            .values()
            .filter(|s| s.words.contains_key(wd) && s.money_cents > 10_000_000)
            .count();
    }
    let sm = ms(t);
    let t = Instant::now();
    let mut n2 = 0;
    for wd in &words {
        for us in w.units.values() {
            let mut b = Vec::new();
            for u in us {
                b.extend_from_slice(&w.repo.read_content(u).unwrap());
            }
            let s = Summary::of(&b);
            n2 += (s.words.contains_key(wd) && s.money_cents > 10_000_000) as usize;
        }
    }
    let dm = ms(t);
    assert_eq!(n, n2);
    line(
        "summary query",
        format!(
            "\"ms_per_query\":{:.4},\"read\":0,\"direct_ms_per_query\":{:.2},\"direct_read\":{},\"matches\":{n}",
            sm / q,
            dm / q,
            corpus_bytes
        ),
    );
}
