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

#[path = "common/insights.rs"]
mod insights;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use ikam_kernel::{Id, MemStore, Repo, blocks};
use insights::*;

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

fn line(what: &str, extra: String) {
    println!("{{\"what\":\"{what}\",{extra}}}");
}

fn main() {
    let files = md_files();
    let mut w = setup(MemStore::default(), &files);
    w.repo.commit("main", w.tree, "corpus").unwrap();
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
        w.repo.commit("main", w.tree, "edit").unwrap();
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
