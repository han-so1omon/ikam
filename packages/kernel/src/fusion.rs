//! Hybrid ranking (E312): one ranked list from a keyword list and a vector
//! list, each a row key with its score, best first. The default is a
//! convex combination (Bruch et al., 2022): each list's scores min-max
//! normalized, then `alpha * keyword + (1 - alpha) * vector`, with alpha
//! the weight on the keyword list. Reciprocal rank fusion stays available.
//!
//! The best alpha follows how strong the vectors are against the keyword
//! list (E259 and E312): 0.5 for MiniLM-class embedders, 0.4 for
//! bge-small-class ones whose vectors alone beat BM25, 0.6 to 0.7 for a
//! static model like WordLlama. Retune it on a few hundred judged queries
//! whenever the embedding model changes.
//!
//! Opt-in: nothing in the kernel ranks with it unless a caller asks.

use std::collections::{BTreeMap, BTreeSet};

/// The default weight on the keyword list (MiniLM-class embedders).
pub const DEFAULT_ALPHA: f32 = 0.5;

/// The weight on the keyword list for embedders whose vectors alone beat
/// the keyword list (bge-small-class).
pub const STRONG_VECTORS_ALPHA: f32 = 0.4;

/// The default hybrid: [`convex`] with [`DEFAULT_ALPHA`].
pub fn hybrid(keyword: &[(String, f32)], vector: &[(String, f32)]) -> Vec<(String, f32)> {
    convex(keyword, vector, DEFAULT_ALPHA)
}

/// A convex combination of two scored lists: each list's scores min-max
/// normalized, then `alpha * a + (1 - alpha) * b`; a row missing from a
/// list scores 0 there. Ties go to the smaller key.
pub fn convex(a: &[(String, f32)], b: &[(String, f32)], alpha: f32) -> Vec<(String, f32)> {
    let (na, nb) = (normalized(a), normalized(b));
    let keys: BTreeSet<&String> = na.keys().chain(nb.keys()).copied().collect();
    let mut out: Vec<(String, f32)> = keys
        .into_iter()
        .map(|k| {
            let s = alpha * na.get(k).unwrap_or(&0.0) + (1.0 - alpha) * nb.get(k).unwrap_or(&0.0);
            (k.clone(), s)
        })
        .collect();
    sort(&mut out);
    out
}

/// Reciprocal rank fusion (Cormack, Clarke and Buettcher, 2009): a row's
/// score is the sum over lists of `1 / (k + rank)`, ranks from 1.
pub fn rrf(lists: &[Vec<String>], k: f32) -> Vec<(String, f32)> {
    let mut score: BTreeMap<String, f32> = BTreeMap::new();
    for list in lists {
        for (r, key) in list.iter().enumerate() {
            *score.entry(key.clone()).or_default() += 1.0 / (k + r as f32 + 1.0);
        }
    }
    let mut out: Vec<(String, f32)> = score.into_iter().collect();
    sort(&mut out);
    out
}

fn normalized(l: &[(String, f32)]) -> BTreeMap<&String, f32> {
    let (lo, hi) = l.iter().fold((f32::MAX, f32::MIN), |(lo, hi), x| {
        (lo.min(x.1), hi.max(x.1))
    });
    let span = if hi > lo { hi - lo } else { 1.0 };
    l.iter().map(|(k, s)| (k, (s - lo) / span)).collect()
}

fn sort(out: &mut [(String, f32)]) {
    out.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| x.0.cmp(&y.0)));
}
