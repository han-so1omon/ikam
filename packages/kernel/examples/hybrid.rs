//! E254: hybrid ranking (BM25 with vectors) and metadata filters, on a
//! synthetic corpus built so that neither signal alone finds the answer.
//! `cargo run --release --features search-tantivy,vectors-hnsw --example hybrid -- N`
//!
//! Each document has a topic (100) and one of its topic's 10 facets. A
//! facet has 3 synonyms, and a document's text names its facet by one of
//! them, among topic and common words (facet words also appear by chance in
//! 3% of other documents). Its vector is its topic's and facet's meaning
//! plus noise, whichever synonym the text uses. A query names the facet by
//! one synonym, with a topic word, and its vector is the same meaning plus
//! noise; its relevant documents are that topic's documents with that facet.
//! So words match a third of them, and vectors find meaning but blur
//! between neighbouring facets. Prints
//! nDCG@10 and recall@50 for BM25 (Tantivy), vectors, RRF and convex
//! fusion, then recall@10 and query time for filters on a metadata field
//! at several selectivities: pre-filter (exact over matching rows),
//! post-filter (HNSW top 100, then the filter) and in-graph (hnsw_rs
//! `search_filter`). Synthetic data only.

mod db_common;

use std::collections::HashSet;
use std::time::Instant;

use db_common::Lcg;
use hnsw_rs::prelude::{DistCosine, Hnsw};
use ikam_kernel::vectors::{convex, dot, rrf};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{STORED, STRING, Schema, TEXT, Value};
use tantivy::{Index, TantivyDocument, doc};

const DIM: usize = 64;

fn unit(v: Vec<f32>) -> Vec<f32> {
    let n = dot(&v, &v).sqrt();
    v.into_iter().map(|x| x / n).collect()
}

fn noise(rng: &mut Lcg) -> Vec<f32> {
    (0..DIM)
        .map(|_| (rng.next() % 2001) as f32 / 1000.0 - 1.0)
        .collect()
}

/// How much a vector knows: the facet's weight and the noise's length,
/// against the topic's (1). Run from most to least informative.
const LEVELS: [(f32, f32); 3] = [(0.4, 0.8), (0.3, 1.0), (0.2, 1.5)];

/// The meaning of topic `t` and facet `f` at `level`, plus noise.
fn near(
    rng: &mut Lcg,
    topics: &[Vec<f32>],
    facets: &[Vec<f32>],
    t: usize,
    f: usize,
    level: (f32, f32),
) -> Vec<f32> {
    let n = unit(noise(rng));
    let (c, d) = (&topics[t], &facets[t * 10 + f]);
    unit(
        (0..DIM)
            .map(|i| c[i] + level.0 * d[i] + level.1 * n[i])
            .collect(),
    )
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// nDCG@10 with binary gains, and recall@50.
fn quality(ranked: &[String], relevant: &HashSet<String>) -> (f64, f64) {
    let dcg: f64 = ranked
        .iter()
        .take(10)
        .enumerate()
        .filter(|(_, k)| relevant.contains(*k))
        .map(|(i, _)| 1.0 / (i as f64 + 2.0).log2())
        .sum();
    let ideal: f64 = (0..relevant.len().min(10))
        .map(|i| 1.0 / (i as f64 + 2.0).log2())
        .sum();
    let hit50 = ranked
        .iter()
        .take(50)
        .filter(|k| relevant.contains(*k))
        .count();
    (dcg / ideal, hit50 as f64 / relevant.len().min(50) as f64)
}

fn main() {
    let n: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    for (li, level) in LEVELS.into_iter().enumerate() {
        run(n, level, li == 0);
    }
}

/// One corpus at `level`; the filter measurements on the first only.
fn run(n: usize, level: (f32, f32), filters: bool) {
    let mut rng = Lcg(11);
    let centers: Vec<Vec<f32>> = (0..100).map(|_| unit(noise(&mut rng))).collect();
    let facets: Vec<Vec<f32>> = (0..1000).map(|_| unit(noise(&mut rng))).collect();
    let word = |kind: &str, i: usize| format!("{kind}{i}");
    let mut docs = Vec::with_capacity(n);
    for i in 0..n {
        let (t, f) = (rng.next() as usize % 100, rng.next() as usize % 10);
        let mut w: Vec<String> = (0..30)
            .map(|_| match rng.next() % 10 {
                0..=3 => word("topic", t * 50 + rng.next() as usize % 50),
                _ => word("common", rng.next() as usize % 2000),
            })
            .collect();
        w.push(word("facet", (t * 10 + f) * 3 + rng.next() as usize % 3));
        if rng.next() % 100 < 3 {
            w.push(word("facet", rng.next() as usize % 3000));
        }
        let status = rng.next() % 1000;
        docs.push((
            format!("d{i:07}"),
            t,
            f,
            w.join(" "),
            near(&mut rng, &centers, &facets, t, f, level),
            status,
        ));
    }
    // Tantivy for BM25.
    let mut sb = Schema::builder();
    let (kf, bf) = (
        sb.add_text_field("key", STRING | STORED),
        sb.add_text_field("body", TEXT),
    );
    let index = Index::create_in_ram(sb.build());
    let mut writer = index.writer_with_num_threads(1, 200_000_000).unwrap();
    for d in &docs {
        writer
            .add_document(doc!(kf => d.0.clone(), bf => d.3.clone()))
            .unwrap();
    }
    writer.commit().unwrap();
    let searcher = index.reader().unwrap().searcher();
    let qp = QueryParser::for_index(&index, vec![bf]);
    let bm25 = |q: &str| -> Vec<(String, f32)> {
        searcher
            .search(&qp.parse_query(q).unwrap(), &TopDocs::with_limit(100))
            .unwrap()
            .into_iter()
            .map(|(s, a)| {
                let d: TantivyDocument = searcher.doc(a).unwrap();
                (
                    d.get_first(kf)
                        .and_then(|v| v.as_str())
                        .unwrap()
                        .to_string(),
                    s,
                )
            })
            .collect()
    };
    let vec_top = |q: &[f32], keep: &dyn Fn(usize) -> bool, k: usize| -> Vec<(String, f32)> {
        let mut s: Vec<(String, f32)> = docs
            .iter()
            .enumerate()
            .filter(|(i, _)| keep(*i))
            .map(|(_, d)| (d.0.clone(), dot(q, &d.4)))
            .collect();
        s.sort_by(|a, b| b.1.total_cmp(&a.1));
        s.truncate(k);
        s
    };
    // Queries: a facet word and a topic word, a vector near the topic.
    let mut sums = [(0.0, 0.0, 0.0); 4];
    let queries: Vec<(usize, usize, Vec<f32>, String)> = (0..200)
        .map(|_| {
            let (t, f) = (rng.next() as usize % 100, rng.next() as usize % 10);
            let q = format!(
                "{} {}",
                word("facet", (t * 10 + f) * 3 + rng.next() as usize % 3),
                word("topic", t * 50 + rng.next() as usize % 50)
            );
            (t, f, near(&mut rng, &centers, &facets, t, f, level), q)
        })
        .collect();
    for (t, f, qv, qt) in &queries {
        let relevant: HashSet<String> = docs
            .iter()
            .filter(|d| d.1 == *t && d.2 == *f)
            .map(|d| d.0.clone())
            .collect();
        let tm = Instant::now();
        let b = bm25(qt);
        let tb = ms(tm);
        let tm = Instant::now();
        let v = vec_top(qv, &|_| true, 100);
        let tv = ms(tm);
        let keys = |l: &[(String, f32)]| l.iter().map(|x| x.0.clone()).collect::<Vec<_>>();
        let tm = Instant::now();
        let r = rrf(&[keys(&b), keys(&v)], 60.0);
        let tr = ms(tm) + tb + tv;
        let tm = Instant::now();
        let c = convex(&b, &v, 0.5);
        let tc = ms(tm) + tb + tv;
        for (i, (ranked, t)) in [
            (keys(&b), tb),
            (keys(&v), tv),
            (keys(&r), tr),
            (keys(&c), tc),
        ]
        .into_iter()
        .enumerate()
        {
            let (nd, rc) = quality(&ranked, &relevant);
            sums[i].0 += nd;
            sums[i].1 += rc;
            sums[i].2 += t;
        }
    }
    let m = queries.len() as f64;
    for (i, way) in ["bm25", "vectors", "rrf_k60", "convex_alpha_0.5"]
        .iter()
        .enumerate()
    {
        println!(
            "{{\"docs\":{n},\"facet_weight\":{},\"noise\":{},\"way\":\"{way}\",\"ndcg_at_10\":{:.4},\"recall_at_50\":{:.4},\"query_ms\":{:.3}}}",
            level.0,
            level.1,
            sums[i].0 / m,
            sums[i].1 / m,
            sums[i].2 / m
        );
    }
    if !filters {
        return;
    }
    // Filters on `status` (0..1000): keep rows with status below a bound.
    let t = Instant::now();
    let h: Hnsw<f32, DistCosine> = Hnsw::new(16, n, 16, 100, DistCosine {});
    for (i, d) in docs.iter().enumerate() {
        h.insert((&d.4, i));
    }
    println!("{{\"docs\":{n},\"hnsw_build_ms\":{:.0}}}", ms(t));
    for bound in [500, 100, 20, 5] {
        let keep = |i: usize| docs[i].5 < bound;
        let mut acc = [(0.0, 0.0); 3];
        for (_, _, qv, _) in &queries {
            let want: HashSet<String> = vec_top(qv, &keep, 10).into_iter().map(|x| x.0).collect();
            let tm = Instant::now();
            let pre = vec_top(qv, &keep, 10);
            let tp = ms(tm);
            let tm = Instant::now();
            let post: Vec<String> = h
                .search(qv, 100, 128)
                .into_iter()
                .filter(|x| keep(x.d_id))
                .take(10)
                .map(|x| docs[x.d_id].0.clone())
                .collect();
            let to = ms(tm);
            let tm = Instant::now();
            let f = |id: &usize| keep(*id);
            let ing: Vec<String> = h
                .search_filter(qv, 10, 128, Some(&f))
                .into_iter()
                .map(|x| docs[x.d_id].0.clone())
                .collect();
            let ti = ms(tm);
            for (i, (got, t)) in [
                (pre.into_iter().map(|x| x.0).collect::<Vec<_>>(), tp),
                (post, to),
                (ing, ti),
            ]
            .into_iter()
            .enumerate()
            {
                acc[i].0 += got.iter().filter(|k| want.contains(*k)).count() as f64
                    / want.len().max(1) as f64;
                acc[i].1 += t;
            }
        }
        for (i, way) in ["pre_filter_exact", "post_filter_hnsw_100", "in_graph_hnsw"]
            .iter()
            .enumerate()
        {
            println!(
                "{{\"docs\":{n},\"filter_keeps\":{:.3},\"way\":\"{way}\",\"recall_at_10\":{:.4},\"query_ms\":{:.3}}}",
                bound as f64 / 1000.0,
                acc[i].0 / m,
                acc[i].1 / m
            );
        }
    }
}
