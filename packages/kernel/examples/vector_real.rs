//! E257: E253's vector views on real vectors: WordLlama embeddings of
//! PyPI READMEs' paragraphs (or whole READMEs), queried by real project
//! summaries, with a real drift: the centroids are trained before a
//! tenth of the data, two whole regions of it, arrives.
//! `cargo run --release --features vectors-hnsw --example vector_real -- DIR SET N`
//!
//! SET is `frags` (distinct paragraphs) or `docs` (whole READMEs); N caps
//! the rows. The rows are split by 20-means over a sample: the rows of two
//! of the 20 clusters are held out. Each way is built on the rest, queried
//! (recall@10 against exact search over the same rows), fed the held-out
//! rows in batches of 10, 100, 1,000 and then the remainder, and queried
//! again against exact search over every row. Ways as E253: `exact` (every
//! row read from ikam), `flat_int8`, `ivf_int8` (sqrt(N) lists, 8 or 32
//! probed), `hnsw` (hnsw_rs in memory, M 16, ef 100 to build, 64 to search).

mod db_common;
mod real_common;

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use db_common::Counting;
use hnsw_rs::prelude::{DistCosine, Hnsw};
use ikam_kernel::vectors::{dot, from_f32s, kmeans, to_f32s};
use ikam_kernel::{Db, Event, Repo};

const K: usize = 10;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn nearest(cents: &[f32], dim: usize, v: &[f32]) -> usize {
    (0..cents.len() / dim)
        .max_by(|&a, &b| {
            dot(&cents[a * dim..][..dim], v).total_cmp(&dot(&cents[b * dim..][..dim], v))
        })
        .unwrap()
}

fn recall(got: &[String], want: &[String]) -> f64 {
    let w: HashSet<&String> = want.iter().collect();
    got.iter().filter(|g| w.contains(g)).count() as f64 / want.len().max(1) as f64
}

fn exact(rows: &BTreeMap<String, Vec<f32>>, q: &[f32]) -> Vec<String> {
    let mut s: Vec<(f32, &String)> = rows.iter().map(|(k, v)| (dot(q, v), k)).collect();
    s.sort_by(|a, b| b.0.total_cmp(&a.0));
    s.into_iter().take(K).map(|x| x.1.clone()).collect()
}

struct Graph<'a> {
    h: Hnsw<'a, f32, DistCosine>,
    keys: Vec<String>,
}

impl Graph<'_> {
    fn insert(&mut self, k: &str, v: &[f32]) {
        self.h.insert((v, self.keys.len()));
        self.keys.push(k.to_string());
    }
    fn search(&self, q: &[f32], ef: usize) -> Vec<String> {
        self.h
            .search(q, K, ef)
            .into_iter()
            .map(|n| self.keys[n.d_id].clone())
            .collect()
    }
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (dir, set, cap) = (&a[0], &a[1], a[2].parse::<usize>().unwrap());
    let dim = real_common::dim(dir);
    let vecs = real_common::vectors(
        &format!(
            "{dir}/{}",
            if set == "frags" {
                "frags.f32"
            } else {
                "doc_whole.f32"
            }
        ),
        dim,
    );
    let step = (vecs.len() / cap).max(1);
    let all: Vec<(String, Vec<f32>)> = vecs
        .into_iter()
        .enumerate()
        .step_by(step)
        .take(cap)
        .map(|(i, v)| (format!("v/{i:08}"), v))
        .collect();
    let n = all.len();
    let queries: Vec<Vec<f32>> = real_common::queries(dir)
        .iter()
        .zip(real_common::vectors(&format!("{dir}/queries.f32"), dim))
        .filter(|(q, _)| q.task == "known")
        .map(|(_, v)| v)
        .take(300)
        .collect();
    // The drift: two regions held out of the build.
    let sample: Vec<Vec<f32>> = all
        .iter()
        .step_by((n / 5000).max(1))
        .map(|r| r.1.clone())
        .collect();
    let regions = kmeans(&sample, 20, 8);
    let (mut base, mut late): (Vec<_>, Vec<_>) = (vec![], vec![]);
    for r in &all {
        if nearest(&regions, dim, &r.1) < 2 {
            late.push(r.clone());
        } else {
            base.push(r.clone());
        }
    }
    println!(
        "{{\"set\":\"{set}\",\"rows\":{n},\"dim\":{dim},\"queries\":{},\"held_out\":{}}}",
        queries.len(),
        late.len()
    );
    let mut batches: Vec<Vec<(String, Vec<f32>)>> = vec![];
    let mut rest = late.as_slice();
    for size in [10, 100, 1000, usize::MAX] {
        let take = size.min(rest.len());
        batches.push(rest[..take].to_vec());
        rest = &rest[take..];
    }
    let mut repo = Repo::new(Counting::default());
    let mut dbs = vec![repo.db_empty().unwrap()];
    for b in std::iter::once(&base).chain(batches.iter()) {
        let ev: Vec<Event> = b
            .iter()
            .map(|(k, v)| Event::upsert(k, &from_f32s(v), k))
            .collect();
        let mut db = *dbs.last().unwrap();
        for chunk in ev.chunks(10_000) {
            db = repo.db_append(&db, chunk).unwrap();
        }
        dbs.push(db);
    }
    repo.object_templates = false;
    repo.prune_diff = true; // E249
    let base_map: BTreeMap<String, Vec<f32>> = base.iter().cloned().collect();
    let all_map: BTreeMap<String, Vec<f32>> = all.iter().cloned().collect();
    let want0: Vec<Vec<String>> = queries.iter().map(|q| exact(&base_map, q)).collect();
    let want1: Vec<Vec<String>> = queries.iter().map(|q| exact(&all_map, q)).collect();
    let report = |way: &str,
                  phase: &str,
                  times: &[f64],
                  got: &[Vec<String>],
                  want: &[Vec<String>]| {
        let r = got.iter().zip(want).map(|(g, w)| recall(g, w)).sum::<f64>() / got.len() as f64;
        let t = times.iter().sum::<f64>() / times.len() as f64;
        println!(
            "{{\"set\":\"{set}\",\"rows\":{n},\"way\":\"{way}\",\"phase\":\"{phase}\",\"query_ms\":{t:.3},\"recall_at_10\":{r:.4}}}"
        );
    };
    let built = &dbs[1];
    let last = dbs.last().unwrap();
    let (mut times, mut got) = (vec![], vec![]);
    for q in queries.iter().take(20) {
        let t = Instant::now();
        let rows: BTreeMap<String, Vec<f32>> = repo
            .db_scan(built)
            .unwrap()
            .into_iter()
            .map(|(k, v)| (k, to_f32s(&v)))
            .collect();
        got.push(exact(&rows, q));
        times.push(ms(t));
    }
    report("exact", "before", &times, &got, &want0[..20]);

    let base_sample: Vec<Vec<f32>> = base
        .iter()
        .step_by((base.len() / 5000).max(1))
        .map(|r| r.1.clone())
        .collect();
    let nlist = (base.len() as f64).sqrt() as usize;
    let t = Instant::now();
    let cents = kmeans(&base_sample, nlist, 8);
    println!(
        "{{\"set\":\"{set}\",\"rows\":{n},\"kmeans_ms\":{:.0},\"nlist\":{nlist}}}",
        ms(t)
    );
    for (way, lists, probes) in [
        ("flat_int8", 1, 1),
        ("ivf_int8_probe8", nlist, 8),
        ("ivf_int8_probe32", nlist, 32),
    ] {
        let mut r = Repo::new(repo.store().copy());
        r.object_templates = false;
        r.prune_diff = true;
        let c = if lists == 1 {
            kmeans(&base_sample, 1, 1)
        } else {
            cents.clone()
        };
        let before = r.store().stored();
        let t = Instant::now();
        let mut ivf = r.ivf_build(built, "v/", &c, dim).unwrap();
        println!(
            "{{\"set\":\"{set}\",\"rows\":{n},\"way\":\"{way}\",\"build_ms\":{:.0},\"stored_bytes\":{}}}",
            ms(t),
            r.store().stored() - before
        );
        let run = |r: &Repo<Counting>,
                   ivf: &ikam_kernel::vectors::Ivf,
                   db: &Db,
                   want: &[Vec<String>],
                   phase: &str| {
            let (mut times, mut got) = (vec![], vec![]);
            for q in &queries {
                let t = Instant::now();
                got.push(
                    r.ivf_search(ivf, db, q, K, probes, 40)
                        .unwrap()
                        .into_iter()
                        .map(|x| x.0)
                        .collect(),
                );
                times.push(ms(t));
            }
            report(way, phase, &times, &got, want);
        };
        run(&r, &ivf, built, &want0, "before");
        for (k, b) in batches.iter().enumerate() {
            let t = Instant::now();
            ivf = r.ivf_update(&ivf, &dbs[k + 1], &dbs[k + 2], "v/").unwrap();
            println!(
                "{{\"set\":\"{set}\",\"rows\":{n},\"way\":\"{way}\",\"batch\":{},\"update_ms\":{:.1}}}",
                b.len(),
                ms(t)
            );
        }
        run(&r, &ivf, last, &want1, "after_drift");
    }

    let t = Instant::now();
    let mut g = Graph {
        h: Hnsw::new(16, n + 10, 16, 100, DistCosine {}),
        keys: vec![],
    };
    for (k, v) in &base {
        g.insert(k, v);
    }
    println!(
        "{{\"set\":\"{set}\",\"rows\":{n},\"way\":\"hnsw\",\"build_ms\":{:.0}}}",
        ms(t)
    );
    for ef in [64, 128] {
        let (mut times, mut got) = (vec![], vec![]);
        for q in &queries {
            let t = Instant::now();
            got.push(g.search(q, ef));
            times.push(ms(t));
        }
        report(&format!("hnsw_ef{ef}"), "before", &times, &got, &want0);
    }
    for b in &batches {
        let t = Instant::now();
        for (k, v) in b {
            g.insert(k, v);
        }
        println!(
            "{{\"set\":\"{set}\",\"rows\":{n},\"way\":\"hnsw\",\"batch\":{},\"update_ms\":{:.1}}}",
            b.len(),
            ms(t)
        );
    }
    for ef in [64, 128] {
        let (mut times, mut got) = (vec![], vec![]);
        for q in &queries {
            let t = Instant::now();
            got.push(g.search(q, ef));
            times.push(ms(t));
        }
        report(&format!("hnsw_ef{ef}"), "after_drift", &times, &got, &want1);
    }
}
