//! E324 and E326: ranked keyword search (`bm25`), the loaded vector index
//! (`vectors`, E325) and hybrid ranking (`fusion`, E312), all in the
//! kernel, on real text with judgments. The IVF is searched at several
//! numbers of probed lists, each fused with BM25 beside exact vectors.
//!
//! `cargo run --release --example hybrid_kernel -- DIR`
//!
//! DIR holds what `real_text/prepare.py` (on exp/259-fusion-tune and
//! exp/306-transformer-vector-views) writes: `docs.tsv` (key, paragraphs
//! joined by 0x1e), `doc_whole.f32` (one unit vector a document),
//! `queries.tsv` (task, id, split, relevant keys, text) and `queries.f32`.
//! Nothing from it is committed.

use std::collections::BTreeSet;
use std::time::Instant;

use ikam_kernel::fusion::{convex, rrf};
use ikam_kernel::vectors::{dot, from_f32s, kmeans, to_f32s};
use ikam_kernel::{Event, MemStore, Repo};

/// A ranked list of keys.
type Ranked = Vec<(String, f32)>;

struct Query {
    task: String,
    split: String,
    rel: BTreeSet<String>,
    text: String,
    vec: Vec<f32>,
}

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

fn read_f32s(path: &str, dim: usize) -> Vec<Vec<f32>> {
    to_f32s(&std::fs::read(path).unwrap())
        .chunks_exact(dim)
        .map(<[f32]>::to_vec)
        .collect()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
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
    let raw = std::fs::read(format!("{dir}/doc_whole.f32")).unwrap();
    let dim = raw.len() / 4 / docs.len();
    let dvec = read_f32s(&format!("{dir}/doc_whole.f32"), dim);
    let qvec = read_f32s(&format!("{dir}/queries.f32"), dim);
    let queries: Vec<Query> = std::fs::read_to_string(format!("{dir}/queries.tsv"))
        .unwrap()
        .lines()
        .zip(qvec)
        .map(|(l, vec)| {
            let f: Vec<&str> = l.splitn(5, '\t').collect();
            Query {
                task: f[0].into(),
                split: f[2].into(),
                rel: f[3].split(',').map(String::from).collect(),
                text: f[4].into(),
                vec,
            }
        })
        .collect();
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let mut events: Vec<Event> = docs
        .iter()
        .map(|(k, t)| Event::upsert(&format!("t/{k}"), t.as_bytes(), &format!("t{k}")))
        .collect();
    events.extend(
        docs.iter()
            .zip(&dvec)
            .map(|((k, _), v)| Event::upsert(&format!("v/{k}"), &from_f32s(v), &format!("v{k}"))),
    );
    let t = Instant::now();
    let db = repo.db_append(&empty, &events).unwrap();
    let append_ms = ms(t);
    // E324: BM25.
    let t = Instant::now();
    let bid = repo.bm25_build(&db, "t/").unwrap();
    let bm25_build_ms = ms(t);
    let bm25_bytes = repo.read_content(&bid).unwrap().len();
    let t = Instant::now();
    let bm25 = repo.bm25_load(&bid).unwrap();
    let bm25_load_ms = ms(t);
    let text_bytes: usize = docs.iter().map(|d| d.1.len()).sum();
    // E325: IVF over the document vectors, loaded with vectors kept.
    let nlist = ((docs.len() as f64).sqrt() as usize).max(1);
    let t = Instant::now();
    let sample: Vec<Vec<f32>> = dvec
        .iter()
        .step_by((docs.len() / (nlist * 20)).max(1))
        .cloned()
        .collect();
    let cents = kmeans(&sample, nlist, 8);
    let ivf = repo.ivf_build(&db, "v/", &cents, dim).unwrap();
    let ivf_build_ms = ms(t);
    let index = repo
        .ivf_load_with(&ivf, &db, true)
        .unwrap()
        .with_lanes(true);
    println!(
        "{{\"docs\":{},\"dim\":{dim},\"queries\":{},\"text_mb\":{:.1},\"append_ms\":{append_ms:.0},\"bm25_build_ms\":{bm25_build_ms:.0},\"bm25_index_mb\":{:.2},\"bm25_load_ms\":{bm25_load_ms:.0},\"nlist\":{nlist},\"ivf_build_ms\":{ivf_build_ms:.0}}}",
        docs.len(),
        queries.len(),
        text_bytes as f64 / 1e6,
        bm25_bytes as f64 / 1e6
    );
    let strip = |l: Vec<(String, f32)>| -> Vec<(String, f32)> {
        l.into_iter()
            .map(|(k, s)| (k.trim_start_matches("v/").to_string(), s))
            .collect()
    };
    let unprefix = |l: Vec<(String, f32)>| -> Vec<(String, f32)> {
        l.into_iter()
            .map(|(k, s)| (k.trim_start_matches("t/").to_string(), s))
            .collect()
    };
    // Per query: BM25 top 100, and vector top 100 exactly and from the
    // loaded IVF at each number of probed lists.
    let probes: Vec<usize> = [4, 8, 16, 32, 64]
        .into_iter()
        .filter(|&p| p < nlist)
        .chain([nlist])
        .collect();
    let mut t_bm25 = 0.0;
    let mut t_ivf = vec![0.0; probes.len()];
    let (mut kws, mut exacts, mut ivfs) = (Vec::new(), Vec::new(), vec![Vec::new(); probes.len()]);
    for q in &queries {
        let t = Instant::now();
        kws.push(unprefix(bm25.search(&q.text, 100)));
        t_bm25 += ms(t);
        for (i, &p) in probes.iter().enumerate() {
            let t = Instant::now();
            ivfs[i].push(strip(
                index.search(&repo, &db, &q.vec, 100, p, 200).unwrap(),
            ));
            t_ivf[i] += ms(t);
        }
        let mut exact: Vec<(String, f32)> = docs
            .iter()
            .zip(&dvec)
            .map(|((k, _), v)| (k.clone(), dot(&q.vec, v)))
            .collect();
        exact.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        exact.truncate(100);
        exacts.push(exact);
    }
    let nq = queries.len() as f64;
    let t = Instant::now();
    for (kw, vs) in kws.iter().zip(&exacts) {
        let _ = convex(kw, vs, 0.6);
    }
    println!(
        "{{\"bm25_query_ms\":{:.3},\"fuse_ms\":{:.3}}}",
        t_bm25 / nq,
        ms(t) / nq
    );
    let recall = |got: &[Vec<(String, f32)>]| {
        got.iter()
            .zip(&exacts)
            .map(|(vs, ex)| {
                let e: BTreeSet<&String> = ex.iter().take(10).map(|x| &x.0).collect();
                vs.iter().take(10).filter(|x| e.contains(&x.0)).count() as f64 / 10.0
            })
            .sum::<f64>()
            / nq
    };
    let mut sources: Vec<(String, &[Ranked])> = vec![("exact".into(), &exacts[..])];
    for (i, &p) in probes.iter().enumerate() {
        println!(
            "{{\"ivf_nprobe\":{p},\"query_ms\":{:.3},\"recall10_vs_exact\":{:.3}}}",
            t_ivf[i] / nq,
            recall(&ivfs[i])
        );
        sources.push((format!("ivf{p}"), &ivfs[i][..]));
    }
    // nDCG@10 by task and split: BM25, then each vector source alone and
    // fused with BM25.
    let names = |l: &[(String, f32)]| l.iter().map(|x| x.0.clone()).collect::<Vec<_>>();
    for task in ["known", "topic"] {
        for split in ["dev", "test"] {
            let idx: Vec<usize> = (0..queries.len())
                .filter(|&i| queries[i].task == task && queries[i].split == split)
                .collect();
            let stat = |f: &dyn Fn(usize) -> Vec<(String, f32)>| {
                let scores: Vec<f64> = idx
                    .iter()
                    .map(|&i| ndcg10(&f(i), &queries[i].rel))
                    .collect();
                let mean = scores.iter().sum::<f64>() / scores.len() as f64;
                let sd = (scores.iter().map(|s| (s - mean).powi(2)).sum::<f64>()
                    / scores.len() as f64)
                    .sqrt();
                format!("[{mean:.4},{:.4}]", sd / (scores.len() as f64).sqrt())
            };
            println!(
                "{{\"task\":\"{task}\",\"split\":\"{split}\",\"queries\":{},\"bm25\":{}}}",
                idx.len(),
                stat(&|i| kws[i].clone())
            );
            for (name, lists) in &sources {
                let mut row = format!(
                    "{{\"task\":\"{task}\",\"split\":\"{split}\",\"vectors\":\"{name}\",\"alone\":{}",
                    stat(&|i| lists[i].clone())
                );
                row += &format!(
                    ",\"rrf-60\":{}",
                    stat(&|i| rrf(&[names(&kws[i]), names(&lists[i])], 60.0))
                );
                for a in [0.5f32, 0.6, 0.7, 0.8] {
                    row += &format!(
                        ",\"convex-{a}\":{}",
                        stat(&|i| convex(&kws[i], &lists[i], a))
                    );
                }
                println!("{row}}}");
            }
        }
    }
}
