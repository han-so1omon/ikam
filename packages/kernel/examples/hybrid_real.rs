//! E258: E254's hybrid ranking on real text and real judgments: BM25
//! (Tantivy, English stemming) and WordLlama vectors over READMEs of
//! OSI-licensed PyPI projects, fused by RRF (k 60) and by a convex
//! combination (alpha 0.5), as E254 ran them on synthetic data.
//! `cargo run --release --features search-tantivy --example hybrid_real -- DIR`
//!
//! Two tasks from the data's own structure:
//! - `known`: a project's summary as the query, its README (with any
//!   paragraph holding the summary removed) the one relevant document;
//! - `topic`: a Trove classifier (`Topic :: ...`) as the query, the
//!   projects tagged with it relevant (5 to 300 of them).
//!
//! Vectors: `whole` embeds each README whole; `mean` is the mean of its
//! paragraphs' vectors (what a fragment cache, E255/E256, holds). Prints
//! nDCG@10, recall@10, recall@100 and MRR@10 per task and way, and the
//! mean query times, and each way's mean per-query nDCG@10 difference
//! from BM25 with its standard error. Exact vector search, so these are the ceilings an
//! index (E257) approaches.

mod real_common;

use std::time::Instant;

use ikam_kernel::vectors::{convex, rrf};
use real_common::{Quality, keys, mean, quality};

fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let dim = real_common::dim(&dir);
    let docs = real_common::docs(&dir);
    let queries = real_common::queries(&dir);
    let qv = real_common::vectors(&format!("{dir}/queries.f32"), dim);
    let t = Instant::now();
    let whole = real_common::lists(
        &docs,
        &real_common::vectors(&format!("{dir}/doc_whole.f32"), dim),
        &queries,
        &qv,
    );
    let lists_ms = t.elapsed().as_secs_f64() * 1e3 / queries.len() as f64;
    let meanv = real_common::lists(
        &docs,
        &real_common::vectors(&format!("{dir}/doc_mean.f32"), dim),
        &queries,
        &qv,
    );
    let ways: [&str; 7] = [
        "bm25",
        "vec_whole",
        "vec_mean",
        "rrf_k60_whole",
        "convex_0.5_whole",
        "rrf_k60_mean",
        "convex_0.5_mean",
    ];
    for task in ["known", "topic"] {
        let mut acc: Vec<Vec<Quality>> = vec![vec![]; ways.len()];
        let mut fuse_ms = 0.0;
        let mut m = 0;
        for (i, q) in queries.iter().enumerate().filter(|(_, q)| q.task == task) {
            let (b, vw) = &whole[i];
            let vm = &meanv[i].1;
            let t = Instant::now();
            let ranked = [
                keys(b),
                keys(vw),
                keys(vm),
                keys(&rrf(&[keys(b), keys(vw)], 60.0)),
                keys(&convex(b, vw, 0.5)),
                keys(&rrf(&[keys(b), keys(vm)], 60.0)),
                keys(&convex(b, vm, 0.5)),
            ];
            fuse_ms += t.elapsed().as_secs_f64() * 1e3 / 4.0;
            for (w, r) in ranked.iter().enumerate() {
                acc[w].push(quality(r, &q.relevant));
            }
            m += 1;
        }
        for (w, way) in ways.iter().enumerate() {
            let q = mean(&acc[w]);
            // Standard error of the per-query nDCG@10 difference from BM25.
            let d: Vec<f64> = acc[w]
                .iter()
                .zip(&acc[0])
                .map(|(a, b)| a.ndcg10 - b.ndcg10)
                .collect();
            let md = d.iter().sum::<f64>() / m as f64;
            let se = (d.iter().map(|x| (x - md).powi(2)).sum::<f64>()
                / (m as f64 - 1.0).max(1.0)
                / m as f64)
                .sqrt();
            println!(
                "{{\"task\":\"{task}\",\"queries\":{m},\"docs\":{},\"way\":\"{way}\",\"ndcg_at_10\":{:.4},\"recall_at_10\":{:.4},\"recall_at_100\":{:.4},\"mrr_at_10\":{:.4},\"vs_bm25\":{md:.4},\"vs_bm25_se\":{se:.4}}}",
                docs.len(),
                q.ndcg10,
                q.recall10,
                q.recall100,
                q.mrr10
            );
        }
        println!(
            "{{\"task\":\"{task}\",\"fusion_ms\":{:.3}}}",
            fuse_ms / m.max(1) as f64
        );
    }
    println!("{{\"bm25_plus_exact_vectors_ms_per_query\":{lists_ms:.2}}}");
}
