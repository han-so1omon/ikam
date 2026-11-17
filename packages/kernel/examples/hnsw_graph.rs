//! E332: the graph vector index (`hnsw`) against the loaded IVF (E325) on
//! real vectors, both through the store. Per setting: build time, stored
//! bytes, load time, recall@10 against an exact scan, query time and the
//! vectors a query scores; then an update of 1% of the rows.
//!
//! `cargo run --release --example hnsw_graph -- DIR [paragraphs]`
//!
//! DIR is what `real_text/prepare.py` writes (see `hybrid_kernel.rs`);
//! rows are `doc_whole.f32` (one vector a README), or an even sample of
//! `paragraphs` rows of `frags.f32`. Queries are `queries.f32`, or with
//! `QUERIES=paragraphs` paragraph vectors. Nothing from DIR is committed.

use std::collections::BTreeSet;
use std::time::Instant;

use ikam_kernel::hnsw::HnswIndex;
use ikam_kernel::vectors::{dot_lanes, from_f32s, kmeans, to_f32s};
use ikam_kernel::{Event, MemStore, Repo};

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
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = &args[0];
    let raw = std::fs::read(format!("{dir}/queries.f32")).unwrap().len() / 4;
    let lines = std::fs::read_to_string(format!("{dir}/queries.tsv"))
        .unwrap()
        .lines()
        .count();
    let dim = raw / lines;
    let mut queries = read_f32s(&format!("{dir}/queries.f32"), dim);
    // QUERIES=paragraphs: as many paragraph vectors as there are queries,
    // evenly spaced, to set short text queries beside text like the rows.
    if std::env::var("QUERIES").as_deref() == Ok("paragraphs") {
        let all = read_f32s(&format!("{dir}/frags.f32"), dim);
        let step = all.len() / queries.len();
        queries = all
            .into_iter()
            .skip(step / 2)
            .step_by(step)
            .take(lines)
            .collect();
    }
    let vecs: Vec<Vec<f32>> = match args.get(1) {
        Some(n) => {
            let n: usize = n.parse().unwrap();
            let all = read_f32s(&format!("{dir}/frags.f32"), dim);
            let step = (all.len() / n).max(1);
            all.into_iter().step_by(step).take(n).collect()
        }
        None => read_f32s(&format!("{dir}/doc_whole.f32"), dim),
    };
    let n = vecs.len();
    let keys: Vec<String> = (0..n).map(|i| format!("v/{i:07}")).collect();
    let truth: Vec<BTreeSet<&String>> = queries
        .iter()
        .map(|q| {
            let mut s: Vec<(f32, &String)> = vecs
                .iter()
                .zip(&keys)
                .map(|(v, k)| (dot_lanes(q, v), k))
                .collect();
            s.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
            s.into_iter().take(10).map(|x| x.1).collect()
        })
        .collect();
    let nq = queries.len() as f64;
    let recall = |got: &[Vec<(String, f32)>]| {
        got.iter()
            .zip(&truth)
            .map(|(g, t)| g.iter().filter(|x| t.contains(&x.0)).count() as f64 / 10.0)
            .sum::<f64>()
            / nq
    };
    let mut repo = Repo::new(MemStore::default());
    // READ_CACHE=bytes: keep checked contents across reads (E294), for the
    // rescore's reads from the store.
    repo.read_cache = std::env::var("READ_CACHE")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(0);
    let empty = repo.db_empty().unwrap();
    let events: Vec<Event> = keys
        .iter()
        .zip(&vecs)
        .map(|(k, v)| Event::upsert(k, &from_f32s(v), &format!("s{k}")))
        .collect();
    let db = repo.db_append(&empty, &events).unwrap();
    println!(
        "{{\"rows\":{n},\"dim\":{dim},\"queries\":{}}}",
        queries.len()
    );
    // Baseline: the loaded IVF with vectors kept and lanes (E325, E327);
    // IVF=0 leaves it out.
    let ivf = (std::env::var("IVF").as_deref() != Ok("0")).then(|| {
        let nlist = (n as f64).sqrt() as usize;
        let t = Instant::now();
        let sample: Vec<Vec<f32>> = vecs
            .iter()
            .step_by((n / (nlist * 20)).max(1))
            .cloned()
            .collect();
        let ivf = repo
            .ivf_build(&db, "v/", &kmeans(&sample, nlist, 8), dim)
            .unwrap();
        let ivf_build_ms = ms(t);
        let index = repo
            .ivf_load_with(&ivf, &db, true)
            .unwrap()
            .with_lanes(true);
        for p in [8, 16, 32, 64, 128] {
            if p > nlist {
                break;
            }
            let t = Instant::now();
            let got: Vec<_> = queries
                .iter()
                .map(|q| index.search(&repo, &db, q, 10, p, 100).unwrap())
                .collect();
            println!(
                "{{\"index\":\"ivf\",\"nlist\":{nlist},\"build_ms\":{ivf_build_ms:.0},\"nprobe\":{p},\"recall10\":{:.3},\"query_ms\":{:.3}}}",
                recall(&got),
                ms(t) / nq
            );
        }
        ivf
    });
    // The exact scan, for scale.
    let t = Instant::now();
    for q in &queries {
        let mut s: Vec<f32> = vecs.iter().map(|v| dot_lanes(q, v)).collect();
        s.select_nth_unstable_by(10, |a, b| b.total_cmp(a));
    }
    println!("{{\"index\":\"exact\",\"query_ms\":{:.3}}}", ms(t) / nq);
    // THREADS=n: build on n threads (default 1).
    let threads: usize = std::env::var("THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(1);
    // MS=8,16: the link counts to try (default 8, 16 and 24).
    let ms_list: Vec<usize> = std::env::var("MS")
        .ok()
        .map(|m| m.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or(vec![8, 16, 24]);
    // CODES=1: keep int8 codes, not vectors (E334); queries rescore the
    // best RESCORE (default 50; 0: none, the codes' scores) from the store.
    let codes = std::env::var("CODES").as_deref() == Ok("1");
    let rescore: usize = std::env::var("RESCORE")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(50);
    for m in ms_list {
        let t = Instant::now();
        let (h, built) = repo
            .hnsw_build(
                &db,
                "v/",
                HnswIndex::new(dim, m, 100)
                    .with_threads(threads)
                    .with_lanes(true)
                    .with_codes(codes),
            )
            .unwrap();
        let build_ms = ms(t);
        let bytes = repo.read_content(&h.links).unwrap().len();
        drop(built);
        let t = Instant::now();
        let mut g = repo.hnsw_load(&h, &db).unwrap().with_lanes(true);
        let load_ms = ms(t);
        for ef in [10, 20, 40, 80, 160, 320] {
            let (mut scored, t) = (0, Instant::now());
            let got: Vec<_> = queries
                .iter()
                .map(|q| {
                    let r = if codes && rescore > 0 {
                        repo.hnsw_search_rescored(&mut g, &db, q, 10, ef, rescore)
                            .unwrap()
                    } else {
                        g.search(q, 10, ef)
                    };
                    scored += g.scored;
                    r
                })
                .collect();
            println!(
                "{{\"index\":\"hnsw\",\"codes\":{codes},\"threads\":{threads},\"m\":{m},\"build_ms\":{build_ms:.0},\"graph_mb\":{:.2},\"load_ms\":{load_ms:.0},\"ef\":{ef},\"recall10\":{:.3},\"query_ms\":{:.3},\"scored\":{:.0}}}",
                bytes as f64 / 1e6,
                recall(&got),
                ms(t) / nq,
                scored as f64 / nq
            );
        }
        if m != 16 {
            continue;
        }
        // 1% of rows get new values (another row's vector, so the data
        // keeps its shape), then the queries again against the new truth.
        let edits: Vec<Event> = (0..n / 100)
            .map(|j| {
                let (i, src) = (j * 97 % n, (j * 31 + 7) % n);
                Event::upsert(&keys[i], &from_f32s(&vecs[src]), &format!("u{j}"))
            })
            .collect();
        let next = repo.db_append(&db, &edits).unwrap();
        let t = Instant::now();
        repo.hnsw_update(&mut g, &db, &next, "v/").unwrap();
        let update_ms = ms(t);
        let t = Instant::now();
        let h2 = repo.hnsw_save(&g).unwrap();
        let save_ms = ms(t);
        let t = Instant::now();
        if let Some(ivf) = &ivf {
            repo.ivf_update(ivf, &db, &next, "v/").unwrap();
        }
        let ivf_update_ms = ms(t);
        let mut now = vecs.clone();
        for j in 0..n / 100 {
            now[j * 97 % n] = vecs[(j * 31 + 7) % n].clone();
        }
        let truth2: Vec<BTreeSet<&String>> = queries
            .iter()
            .map(|q| {
                let mut s: Vec<(f32, &String)> = now
                    .iter()
                    .zip(&keys)
                    .map(|(v, k)| (dot_lanes(q, v), k))
                    .collect();
                s.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
                s.into_iter().take(10).map(|x| x.1).collect()
            })
            .collect();
        let got: Vec<_> = queries
            .iter()
            .map(|q| {
                if codes && rescore > 0 {
                    repo.hnsw_search_rescored(&mut g, &next, q, 10, 80, rescore)
                        .unwrap()
                } else {
                    g.search(q, 10, 80)
                }
            })
            .collect();
        let r2 = got
            .iter()
            .zip(&truth2)
            .map(|(g, t)| g.iter().filter(|x| t.contains(&x.0)).count() as f64 / 10.0)
            .sum::<f64>()
            / nq;
        let mut reloaded = repo.hnsw_load(&h2, &next).unwrap().with_lanes(true);
        let same = queries
            .iter()
            .all(|q| reloaded.search(q, 10, 80) == g.search(q, 10, 80));
        println!(
            "{{\"update_rows\":{},\"hnsw_update_ms\":{update_ms:.0},\"hnsw_save_ms\":{save_ms:.0},\"ivf_update_ms\":{ivf_update_ms:.0},\"recall10_after_ef80\":{r2:.3},\"reload_same\":{same}}}",
            n / 100
        );
    }
}
