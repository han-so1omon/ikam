//! E351: pseudo-relevance feedback for BM25 on real text. Each query is run,
//! the best `m` rows' terms are weighted (term frequency over row length,
//! times idf, summed over the rows), the best `n` new terms are run as a
//! second query, and the two rankings are fused as `alpha` times the first
//! (scaled by its best score) plus `1 - alpha` times the second. Scored by
//! nDCG@10 on E344's known-item and topic queries, with the paired
//! difference against plain BM25 and its standard error.
//!
//! `cargo run --release --example bm25_prf -- DIR`
//!
//! DIR holds `docs.tsv` and `queries.tsv` as for `bm25_stem`. Nothing from
//! it is committed.

use std::collections::{BTreeSet, HashMap, HashSet};

use ikam_kernel::bm25::terms;
use ikam_kernel::{Event, MemStore, Repo};

fn ndcg10(ranked: &[String], rel: &BTreeSet<String>) -> f64 {
    let dcg: f64 = ranked
        .iter()
        .take(10)
        .enumerate()
        .filter(|(_, r)| rel.contains(*r))
        .map(|(i, _)| 1.0 / ((i + 2) as f64).log2())
        .sum();
    let ideal: f64 = (0..rel.len().min(10))
        .map(|i| 1.0 / ((i + 2) as f64).log2())
        .sum();
    dcg / ideal
}

fn mean_se(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0).max(1.0);
    (m, (var / n).sqrt())
}

fn scaled(hits: &[(String, f32)]) -> HashMap<String, f64> {
    let top = hits.first().map(|h| h.1 as f64).unwrap_or(1.0).max(1e-9);
    hits.iter()
        .map(|(k, s)| (k.clone(), *s as f64 / top))
        .collect()
}

fn main() {
    let dir = std::env::args().nth(1).expect("DIR");
    let docs: Vec<(String, String)> = std::fs::read_to_string(format!("{dir}/docs.tsv"))
        .unwrap()
        .lines()
        .map(|l| {
            let (k, t) = l.split_once('\t').unwrap();
            (format!("t/{k}"), t.replace('\u{1e}', "\n"))
        })
        .collect();
    let queries: Vec<(String, String, BTreeSet<String>, String)> =
        std::fs::read_to_string(format!("{dir}/queries.tsv"))
            .unwrap()
            .lines()
            .map(|l| {
                let f: Vec<&str> = l.splitn(5, '\t').collect();
                let rel = f[3].split(',').map(|k| format!("t/{k}")).collect();
                (f[0].to_string(), f[2].to_string(), rel, f[4].to_string())
            })
            .collect();
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let events: Vec<Event> = docs
        .iter()
        .map(|(k, t)| Event::upsert(k, t.as_bytes(), k))
        .collect();
    let db = repo.db_append(&empty, &events).unwrap();
    let id = repo.bm25_build(&db, "t/").unwrap();
    let ix = repo.bm25_load(&id).unwrap();
    // Each row's term counts and length, and each term's document count.
    let mut tf: HashMap<&str, (HashMap<String, u32>, u32)> = HashMap::new();
    let mut df: HashMap<String, u32> = HashMap::new();
    for (k, t) in &docs {
        let ts: Vec<String> = terms(t).collect();
        let mut c: HashMap<String, u32> = HashMap::new();
        for w in &ts {
            *c.entry(w.clone()).or_default() += 1;
        }
        for w in c.keys() {
            *df.entry(w.clone()).or_default() += 1;
        }
        tf.insert(k, (c, ts.len() as u32));
    }
    let n_docs = docs.len() as f64;
    let idf = |w: &str| (1.0 + (n_docs - df[w] as f64 + 0.5) / (df[w] as f64 + 0.5)).ln();
    let plain: Vec<Vec<(String, f32)>> = queries.iter().map(|q| ix.search(&q.3, 1000)).collect();
    let base: Vec<f64> = queries
        .iter()
        .zip(&plain)
        .map(|(q, h)| ndcg10(&h.iter().map(|x| x.0.clone()).collect::<Vec<_>>(), &q.2))
        .collect();
    println!("{{\"docs\":{},\"queries\":{}}}", docs.len(), queries.len());
    for m in [3usize, 5, 10] {
        for n in [5usize, 10, 20] {
            let expanded: Vec<Vec<(String, f32)>> = queries
                .iter()
                .zip(&plain)
                .map(|(q, hits)| {
                    let own: HashSet<String> = terms(&q.3).collect();
                    let mut w: HashMap<&str, f64> = HashMap::new();
                    for (k, _) in hits.iter().take(m) {
                        let (c, len) = &tf[k.as_str()];
                        for (t, &f) in c {
                            if !own.contains(t) && df[t] >= 2 {
                                *w.entry(t).or_default() += f as f64 / *len as f64 * idf(t);
                            }
                        }
                    }
                    let mut w: Vec<(&str, f64)> = w.into_iter().collect();
                    w.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(b.0)));
                    let text: Vec<&str> = w.iter().take(n).map(|x| x.0).collect();
                    ix.search(&text.join(" "), 1000)
                })
                .collect();
            for alpha in [0.5f64, 0.7, 0.85] {
                let scores: Vec<f64> = (0..queries.len())
                    .map(|i| {
                        let (a, b) = (scaled(&plain[i]), scaled(&expanded[i]));
                        let mut f: HashMap<&String, f64> = HashMap::new();
                        for (k, s) in &a {
                            *f.entry(k).or_default() += alpha * s;
                        }
                        for (k, s) in &b {
                            *f.entry(k).or_default() += (1.0 - alpha) * s;
                        }
                        let mut r: Vec<(&String, f64)> = f.into_iter().collect();
                        r.sort_by(|x, y| y.1.total_cmp(&x.1).then_with(|| x.0.cmp(y.0)));
                        let ranked: Vec<String> = r.iter().map(|x| x.0.clone()).collect();
                        ndcg10(&ranked, &queries[i].2)
                    })
                    .collect();
                let mut cells = Vec::new();
                for task in ["known", "topic"] {
                    for split in ["dev", "test"] {
                        let pick: Vec<usize> = (0..queries.len())
                            .filter(|&i| queries[i].0 == task && queries[i].1 == split)
                            .collect();
                        let (mean, _) =
                            mean_se(&pick.iter().map(|&i| scores[i]).collect::<Vec<_>>());
                        let (d, se) = mean_se(
                            &pick
                                .iter()
                                .map(|&i| scores[i] - base[i])
                                .collect::<Vec<_>>(),
                        );
                        cells.push(format!("\"{task}_{split}\":[{mean:.4},{d:.4},{se:.4}]"));
                    }
                }
                println!(
                    "{{\"m\":{m},\"n\":{n},\"alpha\":{alpha},{}}}",
                    cells.join(",")
                );
            }
        }
    }
}
