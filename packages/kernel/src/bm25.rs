//! Ranked keyword search (E324): Okapi BM25 over the rows under a prefix
//! of a database, the keyword list that hybrid ranking (`fusion`) fuses
//! with a vector list. The substring index (`search`) answers which
//! contents hold a pattern exactly; this ranks rows by how well their
//! words match a query.
//!
//! A row's value is read as text; its terms are runs of alphanumeric
//! characters, lowercased, with no stemming or stop words. A term `t` in a
//! row of length `dl` (terms) with frequency `tf` scores
//! `idf(t) * tf * (k1 + 1) / (tf + k1 * (1 - b + b * dl / avgdl))`, with
//! `idf(t) = ln(1 + (N - n + 0.5) / (n + 0.5))` over `N` rows, `n` of them
//! holding `t` (Robertson and Zaragoza, 2009; Lucene's form, whose idf is
//! never negative). `K1` 0.9 and `B` 0.4 are Anserini's defaults.
//!
//! The index is ordinary content, encoded as `rows:varint { key_len key
//! len }` then `terms:varint { term_len term n { row_delta tf } }`
//! (varints). It is built whole: a database update needs a rebuild.
//!
//! E344: `bm25_build_with` can stem terms (`Analyzer`); the index
//! records its analyzer, and a query is cut the same way. A plain index's
//! bytes are as before; a stemmed one starts `0x00 0x01 analyzer` (a plain
//! index never does: zero rows means zero terms). The views below
//! (`Bm25View`, `Bm25Live`) do not stem.
//!
//! E349: `Bm25::with_weights` scores a loaded index with other `k1` and `b`
//! (off by default; the views keep `K1` and `B`).
//!
//! Opt-in: nothing in the kernel builds or reads it unless a caller asks.

use std::collections::{BTreeMap, HashMap};

use crate::group::{put_varint, varint};
use crate::{Db, Error, Id, Repo, Store};

pub const K1: f32 = 0.9;
pub const B: f32 = 0.4;

/// A loaded index: row keys and lengths, and each term's postings
/// `(row, tf)` in row order.
pub struct Bm25 {
    rows: Vec<String>,
    lens: Vec<u32>,
    avg: f32,
    postings: HashMap<String, Vec<(u32, u32)>>,
    analyzer: Analyzer,
    /// `(k1, b)` for scoring; `(K1, B)` unless `with_weights` set others.
    weights: (f32, f32),
}

/// How a row's text and a query become terms (E344).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Analyzer {
    /// `terms` as they are.
    #[default]
    Plain,
    /// Plural endings cut (`stem::plural`).
    Plural,
    /// Porter's stemmer (`stem::porter`).
    Porter,
}

impl Analyzer {
    fn byte(self) -> u8 {
        match self {
            Analyzer::Plain => 0,
            Analyzer::Plural => 1,
            Analyzer::Porter => 2,
        }
    }

    fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            0 => Analyzer::Plain,
            1 => Analyzer::Plural,
            2 => Analyzer::Porter,
            _ => return None,
        })
    }
}

/// `terms`, each cut as `a` says.
pub fn terms_with(text: &str, a: Analyzer) -> impl Iterator<Item = String> + '_ {
    terms(text).map(move |t| match a {
        Analyzer::Plain => t,
        Analyzer::Plural => crate::stem::plural(&t),
        Analyzer::Porter => crate::stem::porter(&t),
    })
}

/// The terms of `text`: alphanumeric runs, lowercased.
pub fn terms(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
}

fn encode(rows: &[(String, Vec<u8>)], analyzer: Analyzer) -> Vec<u8> {
    let mut post: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    let mut out = Vec::new();
    if analyzer != Analyzer::Plain {
        out.extend_from_slice(&[0, 1, analyzer.byte()]);
    }
    put_varint(&mut out, rows.len());
    for (i, (key, value)) in rows.iter().enumerate() {
        let mut tf: BTreeMap<String, usize> = BTreeMap::new();
        let mut len = 0;
        for t in terms_with(&String::from_utf8_lossy(value), analyzer) {
            *tf.entry(t).or_default() += 1;
            len += 1;
        }
        put_varint(&mut out, key.len());
        out.extend_from_slice(key.as_bytes());
        put_varint(&mut out, len);
        tf.into_iter()
            .for_each(|(t, n)| post.entry(t).or_default().push((i, n)));
    }
    put_varint(&mut out, post.len());
    for (t, list) in post {
        put_varint(&mut out, t.len());
        out.extend_from_slice(t.as_bytes());
        put_varint(&mut out, list.len());
        let mut prev = 0;
        for (i, n) in list {
            put_varint(&mut out, i - prev);
            put_varint(&mut out, n);
            prev = i;
        }
    }
    out
}

fn decode(mut b: &[u8]) -> Option<Bm25> {
    let mut analyzer = Analyzer::Plain;
    if let [0, 1, a, rest @ ..] = b {
        analyzer = Analyzer::from_byte(*a)?;
        b = rest;
    }
    let b = &mut b;
    let take = |b: &mut &[u8], n: usize| -> Option<String> {
        let (s, rest) = (b.get(..n)?, b.get(n..)?);
        *b = rest;
        String::from_utf8(s.to_vec()).ok()
    };
    let n = varint(b)?;
    let (mut rows, mut lens) = (Vec::with_capacity(n), Vec::with_capacity(n));
    for _ in 0..n {
        let kl = varint(b)?;
        rows.push(take(b, kl)?);
        lens.push(varint(b)? as u32);
    }
    let mut postings = HashMap::new();
    for _ in 0..varint(b)? {
        let tl = varint(b)?;
        let t = take(b, tl)?;
        let mut list = Vec::new();
        let mut row = 0;
        for _ in 0..varint(b)? {
            row += varint(b)?;
            list.push((row as u32, varint(b)? as u32));
        }
        postings.insert(t, list);
    }
    let avg = lens.iter().map(|&l| l as f32).sum::<f32>() / (n.max(1) as f32);
    Some(Bm25 {
        rows,
        lens,
        avg,
        postings,
        analyzer,
        weights: (K1, B),
    })
}

impl Bm25 {
    /// The same index scoring with `k1` and `b` in place of `K1` and `B`
    /// (E349, off by default). Query time only: the stored index is the
    /// same. On E344's PyPI READMEs, `with_weights(1.2, 0.9)` (stronger
    /// length normalization) found known items better (nDCG@10 +0.030 dev,
    /// +0.039 test) and topics a little worse (-0.011, -0.008).
    pub fn with_weights(mut self, k1: f32, b: f32) -> Self {
        self.weights = (k1, b);
        self
    }

    /// The `(k1, b)` this index scores with.
    pub fn weights(&self) -> (f32, f32) {
        self.weights
    }

    /// The `k` best rows for `query`, best first, ties by key.
    pub fn search(&self, query: &str, k: usize) -> Vec<(String, f32)> {
        let n = self.rows.len() as f32;
        let mut seen = std::collections::HashSet::new();
        let lists: Vec<&Vec<(u32, u32)>> = terms_with(query, self.analyzer)
            .filter_map(|t| self.postings.get(&t).filter(|_| seen.insert(t.clone())))
            .collect();
        // E338: sums in a dense array when the query touches enough rows,
        // else in a map; in the same order either way (so the same floats),
        // and only the best `k` rows' keys copied.
        let touched: usize = lists.iter().map(|l| l.len()).sum();
        if touched * 16 >= self.rows.len() {
            let mut score = vec![0f32; self.rows.len()];
            let hit = self.sum(&lists, n, &mut score);
            self.best(hit, k, |r| score[r as usize])
        } else {
            let mut score: HashMap<u32, f32> = HashMap::new();
            let hit = self.sum(&lists, n, &mut score);
            self.best(hit, k, |r| score[&r])
        }
    }

    /// Add each list's terms into `slot`, as `search` always has; the rows
    /// touched, in first-touch order.
    fn sum(&self, lists: &[&Vec<(u32, u32)>], n: f32, score: &mut impl Sums) -> Vec<u32> {
        let mut hit = Vec::new();
        for list in lists {
            let df = list.len() as f32;
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            for &(row, tf) in list.iter() {
                let (tf, dl) = (tf as f32, self.lens[row as usize] as f32);
                let (k1, b) = self.weights;
                let norm = k1 * (1.0 - b + b * dl / self.avg.max(1.0));
                let s = score.slot(row);
                // Every term adds more than 0 (idf > 0, tf >= 1).
                if *s == 0.0 {
                    hit.push(row);
                }
                *s += idf * tf * (k1 + 1.0) / (tf + norm);
            }
        }
        hit
    }

    /// The `k` best of `hit` by `score`, best first, ties by key.
    fn best(&self, mut hit: Vec<u32>, k: usize, score: impl Fn(u32) -> f32) -> Vec<(String, f32)> {
        let order = |a: &u32, b: &u32| {
            score(*b)
                .total_cmp(&score(*a))
                .then_with(|| self.rows[*a as usize].cmp(&self.rows[*b as usize]))
        };
        if hit.len() > k && k > 0 {
            hit.select_nth_unstable_by(k - 1, order);
        }
        hit.truncate(k);
        hit.sort_by(order);
        hit.into_iter()
            .map(|r| (self.rows[r as usize].clone(), score(r)))
            .collect()
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

impl<S: Store> Repo<S> {
    /// A BM25 index of the rows of `db` under `prefix`, stored as content.
    pub fn bm25_build(&mut self, db: &Db, prefix: &str) -> Result<Id, Error> {
        self.bm25_build_with(db, prefix, Analyzer::Plain)
    }

    /// `bm25_build` with terms cut by `analyzer` (E344).
    pub fn bm25_build_with(
        &mut self,
        db: &Db,
        prefix: &str,
        analyzer: Analyzer,
    ) -> Result<Id, Error> {
        let rows = self.db_prefix(db, prefix, usize::MAX, false)?;
        self.put_derived(&encode(&rows, analyzer))
    }

    pub fn bm25_load(&self, id: &Id) -> Result<Bm25, Error> {
        decode(&self.read_content(id)?).ok_or(Error::Decode("bm25 index"))
    }
}

/// A BM25 index kept as a view of the database (E329): a graph with one
/// node per term (`t US term`, its postings `{ key_len key tf dl }` as
/// content, sorted by key) and a stats node (`s`: rows and total length).
/// An update edits only the terms of the rows in the `kv` diff, and a
/// query reads only its own terms' postings, so nothing is loaded whole.
/// The same rows give the same root, however the view got there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bm25View {
    pub root: Id,
}

/// A view kept in two parts (E329, an option): `base`, a `Bm25View` at
/// the database `base_db`, and `delta`, a small graph of the rows changed
/// since: their postings (`t US term`), a mark per changed row (`k US
/// key`, removed rows included) and the stats of the whole (`s`). An
/// update edits only the delta, whose lists are short; once the delta
/// marks more than `merge_rows` rows it is folded into the base. A query
/// reads each term's list from both, drops the base postings of marked
/// rows, and scores as a rebuild would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bm25Live {
    pub base: Bm25View,
    pub base_db: Db,
    pub delta: Id,
    pub merge_rows: usize,
}

const US: char = '\u{1f}';
const STATS: &str = "s";

type Postings = BTreeMap<String, (usize, usize)>;

fn encode_postings(p: &Postings) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, (tf, dl)) in p {
        put_varint(&mut out, k.len());
        out.extend_from_slice(k.as_bytes());
        put_varint(&mut out, *tf);
        put_varint(&mut out, *dl);
    }
    out
}

fn decode_postings(mut b: &[u8]) -> Option<Postings> {
    let b = &mut b;
    let mut out = Postings::new();
    while !b.is_empty() {
        let n = varint(b)?;
        let k = String::from_utf8(b.get(..n)?.to_vec()).ok()?;
        *b = b.get(n..)?;
        let tf = varint(b)?;
        out.insert(k, (tf, varint(b)?));
    }
    Some(out)
}

/// Term frequencies and length of a row's value.
fn row_terms(value: &[u8]) -> (BTreeMap<String, usize>, usize) {
    let mut tf = BTreeMap::new();
    let mut len = 0;
    for t in terms(&String::from_utf8_lossy(value)) {
        *tf.entry(t).or_default() += 1;
        len += 1;
    }
    (tf, len)
}

fn term_label(t: &str) -> String {
    format!("t{US}{t}")
}

/// What a `kv` diff does to postings: per term, keys dropped and keys set
/// to `(tf, dl)`; the rows changed; and the change in rows and length.
#[derive(Default)]
struct Change {
    drops: BTreeMap<String, Vec<String>>,
    sets: BTreeMap<String, Vec<(String, usize, usize)>>,
    rows: Vec<String>,
    added: (usize, usize),
    removed: (usize, usize),
}

/// An encoded postings list read in place: `(key, tf, dl)` in key order.
fn postings_in(mut b: &[u8]) -> Option<Vec<(&str, usize, usize)>> {
    let mut out = Vec::new();
    while !b.is_empty() {
        let n = varint(&mut b)?;
        let k = std::str::from_utf8(b.get(..n)?).ok()?;
        b = b.get(n..)?;
        let tf = varint(&mut b)?;
        out.push((k, tf, varint(&mut b)?));
    }
    Some(out)
}

fn bm25_score<'a>(
    p: &[(&'a str, usize, usize)],
    n: f32,
    avg: f32,
    score: &mut HashMap<&'a str, f32>,
) {
    let df = p.len() as f32;
    let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
    for (key, tf, dl) in p {
        let (tf, dl) = (*tf as f32, *dl as f32);
        let norm = K1 * (1.0 - B + B * dl / avg.max(1.0));
        *score.entry(key).or_default() += idf * tf * (K1 + 1.0) / (tf + norm);
    }
}

fn ranked(score: HashMap<&str, f32>, k: usize) -> Vec<(String, f32)> {
    let mut out: Vec<(&str, f32)> = score.into_iter().collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    out.truncate(k);
    out.into_iter().map(|(k, s)| (k.to_string(), s)).collect()
}

impl<S: Store> Repo<S> {
    fn bm25_node(&mut self, label: String, bytes: &[u8]) -> Result<crate::Node, Error> {
        Ok(crate::Node {
            label,
            target: crate::Target::Content(crate::Arg::Whole(self.put_derived(bytes)?)),
            edges: Vec::new(),
        })
    }

    /// `(rows, total length)` kept in a graph's stats node.
    fn bm25_stats(&self, root: &Id) -> Result<(usize, usize), Error> {
        match self.graph_node(root, STATS)? {
            Some(n) => {
                let b = self.node_value(&n)?;
                let mut b = b.as_slice();
                let rows = varint(&mut b).ok_or(Error::Decode("bm25 stats"))?;
                Ok((rows, varint(&mut b).ok_or(Error::Decode("bm25 stats"))?))
            }
            None => Ok((0, 0)),
        }
    }

    fn bm25_stats_node(&mut self, rows: usize, total: usize) -> Result<crate::Node, Error> {
        let mut stats = Vec::new();
        put_varint(&mut stats, rows);
        put_varint(&mut stats, total);
        self.bm25_node(STATS.to_string(), &stats)
    }

    /// The postings lists of `terms` in a graph, by term; absent terms
    /// are left out.
    fn bm25_lists<'a>(
        &self,
        root: &Id,
        terms: impl Iterator<Item = &'a String>,
    ) -> Result<BTreeMap<String, Postings>, Error> {
        let mut out = BTreeMap::new();
        for (t, v) in self.bm25_encoded(root, terms)? {
            out.insert(
                t,
                decode_postings(&v).ok_or(Error::Decode("bm25 postings"))?,
            );
        }
        Ok(out)
    }

    /// The encoded postings lists of `terms` in a graph, by term.
    fn bm25_encoded<'a>(
        &self,
        root: &Id,
        terms: impl Iterator<Item = &'a String>,
    ) -> Result<BTreeMap<String, Vec<u8>>, Error> {
        let labels = terms.map(|t| term_label(t)).collect();
        let mut out = BTreeMap::new();
        for n in self.graph_nodes_in(root, &labels)? {
            out.insert(n.label[2..].to_string(), self.node_value(&n)?);
        }
        Ok(out)
    }

    fn bm25_change(&self, old: &Db, new: &Db, prefix: &str) -> Result<Change, Error> {
        let mut c = Change::default();
        for (row, before, after) in self.graph_diff(&old.kv, &new.kv)? {
            if !row.starts_with(prefix) {
                continue;
            }
            if let Some(n) = before {
                let (tf, len) = row_terms(&self.node_value(&n)?);
                c.removed = (c.removed.0 + 1, c.removed.1 + len);
                tf.into_keys()
                    .for_each(|t| c.drops.entry(t).or_default().push(row.clone()));
            }
            if let Some(n) = after {
                let (tf, len) = row_terms(&self.node_value(&n)?);
                c.added = (c.added.0 + 1, c.added.1 + len);
                for (t, f) in tf {
                    c.sets.entry(t).or_default().push((row.clone(), f, len));
                }
            }
            c.rows.push(row);
        }
        Ok(c)
    }

    /// Apply a change's postings edits to the term lists of `root`, with
    /// `extra` nodes edited too.
    fn bm25_apply(
        &mut self,
        root: &Id,
        c: &Change,
        mut extra: BTreeMap<String, Option<crate::Node>>,
    ) -> Result<Id, Error> {
        let touched: std::collections::BTreeSet<&String> =
            c.drops.keys().chain(c.sets.keys()).collect();
        let mut lists = self.bm25_lists(root, touched.iter().copied())?;
        for t in touched {
            let p = lists.entry(t.clone()).or_default();
            c.drops.get(t).into_iter().flatten().for_each(|k| {
                p.remove(k);
            });
            for (k, f, len) in c.sets.get(t).into_iter().flatten() {
                p.insert(k.clone(), (*f, *len));
            }
            let node = if p.is_empty() {
                None
            } else {
                Some(self.bm25_node(term_label(t), &encode_postings(p))?)
            };
            extra.insert(term_label(t), node);
        }
        self.graph_edit_with(root, extra, crate::Keys::Any)
    }

    pub fn bm25_view_empty(&mut self) -> Result<Bm25View, Error> {
        Ok(Bm25View {
            root: self.put_graph_with(Vec::new(), crate::Keys::Any)?,
        })
    }

    /// Bring `v`, kept at `old`, up to `new` for the rows under `prefix`.
    /// With an empty view and `old` empty, this is a build.
    pub fn bm25_view_update(
        &mut self,
        v: &Bm25View,
        old: &Db,
        new: &Db,
        prefix: &str,
    ) -> Result<Bm25View, Error> {
        let c = self.bm25_change(old, new, prefix)?;
        let (rows, total) = self.bm25_stats(&v.root)?;
        let stats = self.bm25_stats_node(
            rows + c.added.0 - c.removed.0,
            total + c.added.1 - c.removed.1,
        )?;
        Ok(Bm25View {
            root: self.bm25_apply(&v.root, &c, [(STATS.to_string(), Some(stats))].into())?,
        })
    }

    /// The `k` best rows for `query`, best first, ties by key; the same
    /// scores as `Bm25::search` over the same rows.
    pub fn bm25_view_search(
        &self,
        v: &Bm25View,
        query: &str,
        k: usize,
    ) -> Result<Vec<(String, f32)>, Error> {
        let (rows, total) = self.bm25_stats(&v.root)?;
        let (n, avg) = (rows as f32, total as f32 / rows.max(1) as f32);
        let q: std::collections::BTreeSet<String> = terms(query).collect();
        let encoded = self.bm25_encoded(&v.root, q.iter())?;
        let mut score = HashMap::new();
        for b in encoded.values() {
            let p = postings_in(b).ok_or(Error::Decode("bm25 postings"))?;
            bm25_score(&p, n, avg, &mut score);
        }
        Ok(ranked(score, k))
    }

    /// A live view of the rows under `prefix` at `db`: the base built
    /// there, the delta empty.
    pub fn bm25_live_build(
        &mut self,
        db: &Db,
        prefix: &str,
        merge_rows: usize,
    ) -> Result<Bm25Live, Error> {
        let empty_db = self.db_empty()?;
        let blank = self.bm25_view_empty()?;
        let base = self.bm25_view_update(&blank, &empty_db, db, prefix)?;
        let stats = self.bm25_stats(&base.root)?;
        let node = self.bm25_stats_node(stats.0, stats.1)?;
        Ok(Bm25Live {
            base,
            base_db: *db,
            delta: self.put_graph_with(vec![node], crate::Keys::Any)?,
            merge_rows,
        })
    }

    /// Bring `v`, kept at `old`, up to `new`. Edits the delta only, unless
    /// it then marks more than `merge_rows` rows: then the base is updated
    /// from `base_db` to `new` and the delta emptied.
    pub fn bm25_live_update(
        &mut self,
        v: &Bm25Live,
        old: &Db,
        new: &Db,
        prefix: &str,
    ) -> Result<Bm25Live, Error> {
        let c = self.bm25_change(old, new, prefix)?;
        let (rows, total) = self.bm25_stats(&v.delta)?;
        let (rows, total) = (
            rows + c.added.0 - c.removed.0,
            total + c.added.1 - c.removed.1,
        );
        let mut extra = BTreeMap::new();
        for row in &c.rows {
            let label = format!("k{US}{row}");
            let node = crate::Node {
                label: label.clone(),
                target: crate::Target::None,
                edges: Vec::new(),
            };
            extra.insert(label, Some(node));
        }
        extra.insert(STATS.to_string(), Some(self.bm25_stats_node(rows, total)?));
        let delta = self.bm25_apply(&v.delta, &c, extra)?;
        let marked = self.graph_prefix(&delta, &format!("k{US}"))?.len();
        if marked <= v.merge_rows {
            return Ok(Bm25Live { delta, ..*v });
        }
        let base = self.bm25_view_update(&v.base, &v.base_db, new, prefix)?;
        let node = self.bm25_stats_node(rows, total)?;
        Ok(Bm25Live {
            base,
            base_db: *new,
            delta: self.put_graph_with(vec![node], crate::Keys::Any)?,
            merge_rows: v.merge_rows,
        })
    }

    /// `bm25_view_search` over a live view: the same scores as a view
    /// built at the rows it is at.
    pub fn bm25_live_search(
        &self,
        v: &Bm25Live,
        query: &str,
        k: usize,
    ) -> Result<Vec<(String, f32)>, Error> {
        let (rows, total) = self.bm25_stats(&v.delta)?;
        let (n, avg) = (rows as f32, total as f32 / rows.max(1) as f32);
        let q: std::collections::BTreeSet<String> = terms(query).collect();
        let base = self.bm25_encoded(&v.base.root, q.iter())?;
        let delta = self.bm25_encoded(&v.delta, q.iter())?;
        let marked: std::collections::HashSet<String> = if base.is_empty() {
            Default::default()
        } else {
            self.graph_prefix(&v.delta, &format!("k{US}"))?
                .into_iter()
                .map(|n| n.label[2..].to_string())
                .collect()
        };
        let bad = || Error::Decode("bm25 postings");
        let mut score = HashMap::new();
        for t in &q {
            let mut p = match base.get(t) {
                Some(b) => postings_in(b).ok_or_else(bad)?,
                None => Vec::new(),
            };
            p.retain(|x| !marked.contains(x.0));
            if let Some(d) = delta.get(t) {
                p.extend(postings_in(d).ok_or_else(bad)?);
            }
            if !p.is_empty() {
                bm25_score(&p, n, avg, &mut score);
            }
        }
        Ok(ranked(score, k))
    }
}

/// Where `search` adds up a row's score: a dense array or a map.
trait Sums {
    fn slot(&mut self, row: u32) -> &mut f32;
}

impl Sums for Vec<f32> {
    fn slot(&mut self, row: u32) -> &mut f32 {
        &mut self[row as usize]
    }
}

impl Sums for HashMap<u32, f32> {
    fn slot(&mut self, row: u32) -> &mut f32 {
        self.entry(row).or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E338: both ways of adding up give the same rows and the same score
    /// bits, for small and large `k`.
    #[test]
    fn dense_and_map_sums_agree() {
        let mut seed = 7u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            seed >> 33
        };
        let (mut rows, mut lens) = (Vec::new(), Vec::new());
        let mut postings: HashMap<String, Vec<(u32, u32)>> = HashMap::new();
        for r in 0..500u32 {
            rows.push(format!("r{:03}", next() % 1000));
            let mut len = 0;
            for t in 0..20u64 {
                if next() % 3 == 0 {
                    let tf = 1 + (next() % 4) as u32;
                    len += tf;
                    postings.entry(format!("t{t}")).or_default().push((r, tf));
                }
            }
            lens.push(len);
        }
        let avg = lens.iter().map(|&l| l as f32).sum::<f32>() / rows.len() as f32;
        let bm = Bm25 {
            rows,
            lens,
            avg,
            postings,
            analyzer: Analyzer::Plain,
            weights: (K1, B),
        };
        let n = bm.rows.len() as f32;
        for query in ["t1", "t2 t3 t2", "t0 t5 t9 t13 t19"] {
            let mut seen = std::collections::HashSet::new();
            let lists: Vec<&Vec<(u32, u32)>> = terms(query)
                .filter_map(|t| bm.postings.get(&t).filter(|_| seen.insert(t.clone())))
                .collect();
            for k in [0, 1, 10, 100, 1000] {
                let mut dense = vec![0f32; bm.rows.len()];
                let hit = bm.sum(&lists, n, &mut dense);
                let a = bm.best(hit, k, |r| dense[r as usize]);
                let mut map: HashMap<u32, f32> = HashMap::new();
                let hit = bm.sum(&lists, n, &mut map);
                let b = bm.best(hit, k, |r| map[&r]);
                let bits = |v: &[(String, f32)]| -> Vec<(String, u32)> {
                    v.iter().map(|(s, f)| (s.clone(), f.to_bits())).collect()
                };
                assert_eq!(bits(&a), bits(&b), "{query} k={k}");
                assert_eq!(a, bm.search(query, k), "{query} k={k}");
                assert!(
                    a.windows(2)
                        .all(|w| w[0].1 > w[1].1 || (w[0].1 == w[1].1 && w[0].0 <= w[1].0))
                );
            }
        }
    }
}
