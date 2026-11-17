//! E331: how an IVF routes real vectors to lists. For each way of
//! training the centroids, the share of a query's true top 10 that lies
//! in the lists it probes (the most recall any rescore can reach), and the
//! rows those lists hold. In memory, outside the store: the routing alone.
//!
//! `cargo run --release --example ivf_routing -- DIR [paragraphs]`
//!
//! DIR is what `prepare.py` writes; rows are `doc_whole.f32` (every
//! README), or an even sample of `paragraphs` rows of `frags.f32`.

use std::collections::BTreeSet;
use std::time::Instant;

use ikam_kernel::vectors::{dot_lanes, kmeans, to_f32s};

fn read_f32s(path: &str, dim: usize) -> Vec<Vec<f32>> {
    to_f32s(&std::fs::read(path).unwrap())
        .chunks_exact(dim)
        .map(<[f32]>::to_vec)
        .collect()
}

fn unit(mut v: Vec<f32>) -> Vec<f32> {
    let n = dot_lanes(&v, &v).sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

/// Lists by centroid score for `v`, best first.
fn ranked_lists(cents: &[f32], dim: usize, v: &[f32]) -> Vec<usize> {
    let mut s: Vec<(usize, f32)> = cents
        .chunks_exact(dim)
        .enumerate()
        .map(|(i, c)| (i, dot_lanes(c, v)))
        .collect();
    s.sort_by(|a, b| b.1.total_cmp(&a.1));
    s.into_iter().map(|x| x.0).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = &args[0];
    let raw = std::fs::read(format!("{dir}/queries.f32")).unwrap().len() / 4;
    let nq_lines = std::fs::read_to_string(format!("{dir}/queries.tsv"))
        .unwrap()
        .lines()
        .count();
    let dim = raw / nq_lines;
    let queries = read_f32s(&format!("{dir}/queries.f32"), dim);
    let rows: Vec<Vec<f32>> = match args.get(1) {
        Some(n) => {
            let n: usize = n.parse().unwrap();
            let all = read_f32s(&format!("{dir}/frags.f32"), dim);
            let step = (all.len() / n).max(1);
            all.into_iter().step_by(step).take(n).collect()
        }
        None => read_f32s(&format!("{dir}/doc_whole.f32"), dim),
    };
    let n = rows.len();
    let nlist = (n as f64).sqrt() as usize;
    // True top 10 of each query.
    let truth: Vec<BTreeSet<usize>> = queries
        .iter()
        .map(|q| {
            let mut s: Vec<(usize, f32)> = rows
                .iter()
                .enumerate()
                .map(|(i, v)| (i, dot_lanes(q, v)))
                .collect();
            s.sort_by(|a, b| b.1.total_cmp(&a.1));
            s.iter().take(10).map(|x| x.0).collect()
        })
        .collect();
    let mut mean = vec![0f32; dim];
    for v in &rows {
        mean.iter_mut().zip(v).for_each(|(m, x)| *m += x / n as f32);
    }
    let mean_norm = dot_lanes(&mean, &mean).sqrt();
    let avg_cos: f32 = rows
        .iter()
        .take(2000)
        .map(|v| dot_lanes(v, &mean) / mean_norm)
        .sum::<f32>()
        / rows.len().min(2000) as f32;
    println!(
        "{{\"rows\":{n},\"dim\":{dim},\"queries\":{},\"nlist\":{nlist},\"mean_norm\":{mean_norm:.3},\"avg_cos_to_mean\":{avg_cos:.3}}}",
        queries.len()
    );
    let center = |v: &[f32]| unit(v.iter().zip(&mean).map(|(x, m)| x - m).collect());
    for (centered, per_list, iters) in [
        (false, 20, 8),
        (false, 100, 20),
        (true, 20, 8),
        (true, 100, 20),
    ] {
        let space: Vec<Vec<f32>> = if centered {
            rows.iter().map(|v| center(v)).collect()
        } else {
            rows.clone()
        };
        let t = Instant::now();
        let sample: Vec<Vec<f32>> = space
            .iter()
            .step_by((n / (nlist * per_list)).max(1))
            .cloned()
            .collect();
        let cents = kmeans(&sample, nlist, iters);
        let train_ms = t.elapsed().as_secs_f64() * 1e3;
        let mut lists: Vec<Vec<usize>> = vec![Vec::new(); nlist];
        for (i, v) in space.iter().enumerate() {
            lists[ranked_lists(&cents, dim, v)[0]].push(i);
        }
        let sizes: Vec<usize> = lists.iter().map(Vec::len).collect();
        let max = *sizes.iter().max().unwrap();
        let empty = sizes.iter().filter(|&&s| s == 0).count();
        let mut row = format!(
            "{{\"centered\":{centered},\"sample_per_list\":{per_list},\"iters\":{iters},\"train_ms\":{train_ms:.0},\"largest_list_x_mean\":{:.1},\"empty_lists\":{empty}",
            max as f64 * nlist as f64 / n as f64
        );
        for probe in [4, 8, 16, 32, 64] {
            if probe > nlist {
                break;
            }
            let (mut hit, mut scanned) = (0usize, 0usize);
            for (q, want) in queries.iter().zip(&truth) {
                let qs = if centered { center(q) } else { q.clone() };
                let probed: BTreeSet<usize> = ranked_lists(&cents, dim, &qs)
                    .into_iter()
                    .take(probe)
                    .flat_map(|l| {
                        scanned += lists[l].len();
                        lists[l].iter().copied()
                    })
                    .collect();
                hit += want.iter().filter(|i| probed.contains(i)).count();
            }
            row += &format!(
                ",\"p{probe}\":[{:.3},{:.0}]",
                hit as f64 / (10 * queries.len()) as f64,
                scanned as f64 / queries.len() as f64
            );
        }
        println!("{row}}}");
    }
}
