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
}

/// The terms of `text`: alphanumeric runs, lowercased.
pub fn terms(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
}

fn encode(rows: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut post: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    let mut out = Vec::new();
    put_varint(&mut out, rows.len());
    for (i, (key, value)) in rows.iter().enumerate() {
        let mut tf: BTreeMap<String, usize> = BTreeMap::new();
        let mut len = 0;
        for t in terms(&String::from_utf8_lossy(value)) {
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
    })
}

impl Bm25 {
    /// The `k` best rows for `query`, best first, ties by key.
    pub fn search(&self, query: &str, k: usize) -> Vec<(String, f32)> {
        let n = self.rows.len() as f32;
        let mut score: HashMap<u32, f32> = HashMap::new();
        let mut seen = std::collections::HashSet::new();
        for t in terms(query) {
            let Some(list) = self.postings.get(&t).filter(|_| seen.insert(t.clone())) else {
                continue;
            };
            let df = list.len() as f32;
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            for &(row, tf) in list {
                let (tf, dl) = (tf as f32, self.lens[row as usize] as f32);
                let norm = K1 * (1.0 - B + B * dl / self.avg.max(1.0));
                *score.entry(row).or_default() += idf * tf * (K1 + 1.0) / (tf + norm);
            }
        }
        let mut out: Vec<(String, f32)> = score
            .into_iter()
            .map(|(r, s)| (self.rows[r as usize].clone(), s))
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out.truncate(k);
        out
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
        let rows = self.db_prefix(db, prefix, usize::MAX, false)?;
        self.put_derived(&encode(&rows))
    }

    pub fn bm25_load(&self, id: &Id) -> Result<Bm25, Error> {
        decode(&self.read_content(id)?).ok_or(Error::Decode("bm25 index"))
    }
}
