//! E344: stemming for BM25 on real text. One index a stemmer (none,
//! Harman's plural S-stemmer, Porter's), each scored by nDCG@10 on the
//! known-item and topic queries of E326's benchmark, with the paired
//! difference against no stemming and its standard error.
//!
//! `cargo run --release --example bm25_stem -- DIR`
//!
//! DIR holds `docs.tsv` and `queries.tsv` as `real_text/prepare.py` (on
//! exp/259-fusion-tune) writes them. Nothing from it is committed.

use std::collections::BTreeSet;
use std::time::Instant;

use ikam_kernel::bm25::Analyzer;
use ikam_kernel::{Event, MemStore, Repo};

fn ndcg10(ranked: &[(String, f32)], rel: &BTreeSet<String>) -> f64 {
    let dcg: f64 = ranked
        .iter()
        .take(10)
        .enumerate()
        .filter(|(_, r)| rel.contains(&r.0))
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

fn main() {
    let dir = std::env::args().nth(1).expect("DIR");
    let docs: Vec<(String, String)> = std::fs::read_to_string(format!("{dir}/docs.tsv"))
        .unwrap()
        .lines()
        .map(|l| {
            let (k, t) = l.split_once('\t').unwrap();
            (k.to_string(), t.replace('\u{1e}', "\n"))
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
        .map(|(k, t)| Event::upsert(&format!("t/{k}"), t.as_bytes(), &format!("t{k}")))
        .collect();
    let db = repo.db_append(&empty, &events).unwrap();
    println!("{{\"docs\":{},\"queries\":{}}}", docs.len(), queries.len());
    let mut base: Vec<f64> = Vec::new();
    for (name, a) in [
        ("none", Analyzer::Plain),
        ("plural", Analyzer::Plural),
        ("porter", Analyzer::Porter),
    ] {
        let t = Instant::now();
        let id = repo.bm25_build_with(&db, "t/", a).unwrap();
        let build_ms = t.elapsed().as_secs_f64() * 1e3;
        let bytes = repo.read_content(&id).unwrap().len();
        let ix = repo.bm25_load(&id).unwrap();
        let t = Instant::now();
        let scores: Vec<f64> = queries
            .iter()
            .map(|(_, _, rel, text)| ndcg10(&ix.search(text, 100), rel))
            .collect();
        let query_ms = t.elapsed().as_secs_f64() * 1e3 / queries.len() as f64;
        if base.is_empty() {
            base = scores.clone();
        }
        let mut cells = Vec::new();
        for task in ["known", "topic"] {
            for split in ["dev", "test", "all"] {
                let pick: Vec<usize> = (0..queries.len())
                    .filter(|&i| queries[i].0 == task && (split == "all" || queries[i].1 == split))
                    .collect();
                let (m, _) = mean_se(&pick.iter().map(|&i| scores[i]).collect::<Vec<_>>());
                let (d, se) = mean_se(
                    &pick
                        .iter()
                        .map(|&i| scores[i] - base[i])
                        .collect::<Vec<_>>(),
                );
                cells.push(format!(
                    "\"{task}_{split}\":{{\"n\":{},\"ndcg10\":{m:.4},\"diff\":{d:.4},\"se\":{se:.4}}}",
                    pick.len()
                ));
            }
        }
        println!(
            "{{\"stemmer\":\"{name}\",\"build_ms\":{build_ms:.0},\"index_mb\":{:.2},\"query_ms\":{query_ms:.3},{}}}",
            bytes as f64 / 1e6,
            cells.join(",")
        );
    }
}
