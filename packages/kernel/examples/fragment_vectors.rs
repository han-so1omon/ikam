//! E330: a document found by its best paragraph. Each README's paragraphs
//! are rows of their own (E255's fragments), searched exactly or through
//! the loaded IVF (E325), and a document scores as its best paragraph;
//! against one vector of the whole README and the mean of its paragraphs
//! (E258), alone and fused with BM25 (E324, E312).
//!
//! `cargo run --release --example fragment_vectors -- DIR`
//!
//! DIR is what `prepare.py` writes (see `hybrid_kernel`), with
//! `frags.tsv` (every distinct paragraph) and `frags.f32` (their vectors)
//! and `doc_mean.f32`. Nothing from it is committed.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use ikam_kernel::fusion::convex;
use ikam_kernel::vectors::{dot_lanes, from_f32s, kmeans, to_f32s};
use ikam_kernel::{Event, MemStore, Repo};

/// A ranked list of keys.
type Ranked = Vec<(String, f32)>;
/// Task, split, relevant keys, text, vector.
type Query = (String, String, BTreeSet<String>, String, Vec<f32>);

fn read_f32s(path: &str, dim: usize) -> Vec<Vec<f32>> {
    to_f32s(&std::fs::read(path).unwrap())
        .chunks_exact(dim)
        .map(<[f32]>::to_vec)
        .collect()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
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

fn top(mut v: Ranked, k: usize) -> Ranked {
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(k);
    v
}

/// Documents ranked by their best paragraph among `frags` (index, score).
fn best_paragraph(
    frags: &[(usize, f32)],
    owner: &[Vec<usize>],
    keys: &[String],
    k: usize,
) -> Ranked {
    let mut best: HashMap<usize, f32> = HashMap::new();
    for &(f, s) in frags {
        for &d in &owner[f] {
            let e = best.entry(d).or_insert(f32::MIN);
            *e = e.max(s);
        }
    }
    top(
        best.into_iter()
            .map(|(d, s)| (keys[d].clone(), s))
            .collect(),
        k,
    )
}

fn main() {
    let dir = std::env::args().nth(1).expect("DIR");
    let docs: Vec<(String, Vec<String>)> = std::fs::read_to_string(format!("{dir}/docs.tsv"))
        .unwrap()
        .lines()
        .map(|l| {
            let (k, t) = l.split_once('\t').unwrap();
            (k.to_string(), t.split('\u{1e}').map(String::from).collect())
        })
        .collect();
    let keys: Vec<String> = docs.iter().map(|d| d.0.clone()).collect();
    let raw = std::fs::read(format!("{dir}/doc_whole.f32")).unwrap();
    let dim = raw.len() / 4 / docs.len();
    let whole = read_f32s(&format!("{dir}/doc_whole.f32"), dim);
    let mean = read_f32s(&format!("{dir}/doc_mean.f32"), dim);
    let frag_text: Vec<String> = std::fs::read_to_string(format!("{dir}/frags.tsv"))
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    let fvec = read_f32s(&format!("{dir}/frags.f32"), dim);
    assert_eq!(frag_text.len(), fvec.len());
    let index: HashMap<&str, usize> = frag_text
        .iter()
        .enumerate()
        .map(|(i, t)| (t.as_str(), i))
        .collect();
    // Which documents hold each paragraph (a paragraph may repeat).
    let mut owner: Vec<Vec<usize>> = vec![Vec::new(); fvec.len()];
    let mut missing = 0;
    for (d, (_, paras)) in docs.iter().enumerate() {
        for p in paras {
            match index.get(p.as_str()) {
                Some(&f) if owner[f].last() != Some(&d) => owner[f].push(d),
                Some(_) => {}
                None => missing += 1,
            }
        }
    }
    let mut frags_of: Vec<Vec<usize>> = vec![Vec::new(); docs.len()];
    for (f, ds) in owner.iter().enumerate() {
        for &d in ds {
            frags_of[d].push(f);
        }
    }
    let qvec = read_f32s(&format!("{dir}/queries.f32"), dim);
    let queries: Vec<Query> = std::fs::read_to_string(format!("{dir}/queries.tsv"))
        .unwrap()
        .lines()
        .zip(qvec)
        .map(|(l, v)| {
            let f: Vec<&str> = l.splitn(5, '\t').collect();
            (
                f[0].into(),
                f[2].into(),
                f[3].split(',').map(String::from).collect(),
                f[4].into(),
                v,
            )
        })
        .collect();
    // Rows: text for BM25, one vector a paragraph.
    let mut repo = Repo::new(MemStore::default());
    repo.needed_index = true;
    let empty = repo.db_empty().unwrap();
    let mut events: Vec<Event> = docs
        .iter()
        .map(|(k, p)| Event::upsert(&format!("t/{k}"), p.join("\n").as_bytes(), &format!("t{k}")))
        .collect();
    events.extend(
        fvec.iter()
            .enumerate()
            .map(|(i, v)| Event::upsert(&format!("f/{i:07}"), &from_f32s(v), &format!("f{i}"))),
    );
    let t = Instant::now();
    let db = repo.db_append(&empty, &events).unwrap();
    let append_ms = ms(t);
    drop(events);
    let bid = repo.bm25_build(&db, "t/").unwrap();
    let bm25 = repo.bm25_load(&bid).unwrap();
    let nlist = (fvec.len() as f64).sqrt() as usize;
    let t = Instant::now();
    let sample: Vec<Vec<f32>> = fvec
        .iter()
        .step_by((fvec.len() / (nlist * 20)).max(1))
        .cloned()
        .collect();
    let cents = kmeans(&sample, nlist, 8);
    let ivf = repo.ivf_build(&db, "f/", &cents, dim).unwrap();
    let ivf_build_ms = ms(t);
    let t = Instant::now();
    let loaded = repo
        .ivf_load_with(&ivf, &db, true)
        .unwrap()
        .with_lanes(true);
    let load_ms = ms(t);
    println!(
        "{{\"docs\":{},\"paragraphs\":{},\"paragraphs_not_found\":{missing},\"dim\":{dim},\"queries\":{},\"append_ms\":{append_ms:.0},\"nlist\":{nlist},\"ivf_build_ms\":{ivf_build_ms:.0},\"ivf_load_ms\":{load_ms:.0}}}",
        docs.len(),
        fvec.len(),
        queries.len()
    );
    // Per query: BM25, whole and mean vectors exactly, best paragraph
    // exactly (every paragraph scored) and through the IVF.
    let probes = [16, 32, 64, 128];
    let mut lists: Vec<HashMap<String, Ranked>> = Vec::new();
    let (mut t_exact, mut t_ivf) = (0.0, vec![0.0; probes.len()]);
    let mut recall = vec![0.0; probes.len()];
    for (_, _, _, text, q) in &queries {
        let mut l = HashMap::new();
        let kw: Ranked = bm25
            .search(text, 100)
            .into_iter()
            .map(|(k, s)| (k.trim_start_matches("t/").to_string(), s))
            .collect();
        l.insert("bm25".to_string(), kw);
        let scan = |vs: &[Vec<f32>]| {
            top(
                vs.iter()
                    .zip(&keys)
                    .map(|(v, k)| (k.clone(), dot_lanes(q, v)))
                    .collect(),
                100,
            )
        };
        l.insert("whole".into(), scan(&whole));
        l.insert("mean".into(), scan(&mean));
        let t = Instant::now();
        let mut all: Vec<(usize, f32)> = fvec
            .iter()
            .enumerate()
            .map(|(i, v)| (i, dot_lanes(q, v)))
            .collect();
        all.sort_by(|a, b| b.1.total_cmp(&a.1));
        all.truncate(2000);
        let exact = best_paragraph(&all, &owner, &keys, 100);
        t_exact += ms(t);
        for (j, &p) in probes.iter().enumerate() {
            let t = Instant::now();
            let got: Vec<(usize, f32)> = loaded
                .search(&repo, &db, q, 2000, p, 2000)
                .unwrap()
                .into_iter()
                .map(|(k, s)| (k[2..].parse().unwrap(), s))
                .collect();
            let docs_ivf = best_paragraph(&got, &owner, &keys, 100);
            t_ivf[j] += ms(t);
            let e: BTreeSet<&String> = exact.iter().take(10).map(|x| &x.0).collect();
            recall[j] += docs_ivf
                .iter()
                .take(10)
                .filter(|x| e.contains(&x.0))
                .count() as f64
                / 10.0;
            l.insert(format!("best-ivf{p}"), docs_ivf);
        }
        // Both signals: the mean of a document's whole-README score and
        // its best paragraph's, over the union of the two top 100s.
        let pos: HashMap<&String, usize> = keys.iter().enumerate().map(|(i, k)| (k, i)).collect();
        let cands: BTreeSet<usize> = exact
            .iter()
            .chain(&l["whole"])
            .map(|(k, _)| pos[k])
            .collect();
        let both: Ranked = cands
            .into_iter()
            .map(|d| {
                let b = frags_of[d]
                    .iter()
                    .map(|&f| dot_lanes(q, &fvec[f]))
                    .fold(f32::MIN, f32::max);
                (keys[d].clone(), 0.5 * b + 0.5 * dot_lanes(q, &whole[d]))
            })
            .collect();
        l.insert("best+whole".into(), top(both, 100));
        l.insert("best".into(), exact);
        lists.push(l);
    }
    let nq = queries.len() as f64;
    println!("{{\"best_exact_ms\":{:.2}}}", t_exact / nq);
    for (j, p) in probes.iter().enumerate() {
        println!(
            "{{\"best_ivf_nprobe\":{p},\"query_ms\":{:.2},\"doc_recall10_vs_exact\":{:.3}}}",
            t_ivf[j] / nq,
            recall[j] / nq
        );
    }
    let mut sources = vec![
        "whole".to_string(),
        "mean".into(),
        "best".into(),
        "best+whole".into(),
    ];
    sources.extend(probes.iter().map(|p| format!("best-ivf{p}")));
    for task in ["known", "topic"] {
        for split in ["dev", "test"] {
            let idx: Vec<usize> = (0..queries.len())
                .filter(|&i| queries[i].0 == task && queries[i].1 == split)
                .collect();
            let stat = |f: &dyn Fn(usize) -> Ranked| {
                let s: Vec<f64> = idx.iter().map(|&i| ndcg10(&f(i), &queries[i].2)).collect();
                let m = s.iter().sum::<f64>() / s.len() as f64;
                let sd = (s.iter().map(|x| (x - m).powi(2)).sum::<f64>() / s.len() as f64).sqrt();
                format!("[{m:.4},{:.4}]", sd / (s.len() as f64).sqrt())
            };
            println!(
                "{{\"task\":\"{task}\",\"split\":\"{split}\",\"queries\":{},\"bm25\":{}}}",
                idx.len(),
                stat(&|i| lists[i]["bm25"].clone())
            );
            for s in &sources {
                let mut row = format!(
                    "{{\"task\":\"{task}\",\"split\":\"{split}\",\"vectors\":\"{s}\",\"alone\":{}",
                    stat(&|i| lists[i][s].clone())
                );
                for a in [0.5f32, 0.6, 0.7] {
                    row += &format!(
                        ",\"convex-{a}\":{}",
                        stat(&|i| convex(&lists[i]["bm25"], &lists[i][s], a))
                    );
                }
                println!("{row}}}");
            }
        }
    }
}
