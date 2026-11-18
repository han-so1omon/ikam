//! E343: the k-means and IVF assignment step. Before E343 `nearest` scored
//! every centroid twice (inside `max_by`) with the sequential `dot`. This
//! times the old step (copied here), the new one (each centroid scored
//! once, same result), `dot_lanes`, and threads, on clustered unit vectors,
//! and checks which give the old result bit for bit.
//!
//! `cargo run --release --example kmeans_assign -- [rows] [dim] [threads]`

use std::time::Instant;

use ikam_kernel::vectors::{Assign, assign, dot, kmeans, kmeans_with};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    fn gauss(&mut self) -> f32 {
        (0..4)
            .map(|_| (self.next() % 10_001) as f32 / 10_000.0)
            .sum::<f32>()
            - 2.0
    }
}

fn unit(v: Vec<f32>) -> Vec<f32> {
    let n = dot(&v, &v).sqrt();
    v.into_iter().map(|x| x / n).collect()
}

/// The step as it was before E343.
fn nearest_old(cents: &[f32], dim: usize, v: &[f32]) -> usize {
    (0..cents.len() / dim)
        .max_by(|&a, &b| {
            dot(&cents[a * dim..][..dim], v).total_cmp(&dot(&cents[b * dim..][..dim], v))
        })
        .unwrap_or(0)
}

fn kmeans_old(sample: &[Vec<f32>], k: usize, iters: usize) -> Vec<f32> {
    let dim = sample.first().map_or(0, Vec::len);
    let mut cents: Vec<f32> = sample.iter().take(k).flatten().copied().collect();
    for _ in 0..iters {
        let mut sums = vec![0f32; cents.len()];
        for v in sample {
            let c = nearest_old(&cents, dim, v);
            for (s, x) in sums[c * dim..][..dim].iter_mut().zip(v) {
                *s += x;
            }
        }
        for c in 0..cents.len() / dim {
            let s = &mut sums[c * dim..][..dim];
            let n = dot(s, s).sqrt();
            if n > 0.0 {
                s.iter_mut().for_each(|x| *x /= n);
                cents[c * dim..][..dim].copy_from_slice(s);
            }
        }
    }
    cents
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|x| x.to_bits()).collect()
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let rows = args.first().copied().unwrap_or(100_000);
    let dim = args.get(1).copied().unwrap_or(256);
    let threads = args.get(2).copied().unwrap_or(4);
    let nlist = (rows as f64).sqrt().round() as usize;
    let mut rng = Lcg(343);
    // Uneven topics, like real text (E331): topic t is drawn with weight 1/(t+1).
    let ntopics = 512;
    let topics: Vec<Vec<f32>> = (0..ntopics)
        .map(|_| unit((0..dim).map(|_| rng.gauss()).collect()))
        .collect();
    let weights: Vec<f64> = (0..ntopics).map(|t| 1.0 / (t + 1) as f64).collect();
    let total: f64 = weights.iter().sum();
    let pick = |rng: &mut Lcg| {
        let mut r = (rng.next() % 1_000_000) as f64 / 1_000_000.0 * total;
        for (t, w) in weights.iter().enumerate() {
            if r < *w {
                return t;
            }
            r -= w;
        }
        ntopics - 1
    };
    let vecs: Vec<Vec<f32>> = (0..rows)
        .map(|_| {
            let t = &topics[pick(&mut rng)];
            unit(t.iter().map(|x| x + 0.12 * rng.gauss()).collect())
        })
        .collect();
    // Sample of 20 rows a list, as the examples train (E326).
    let sample: Vec<Vec<f32>> = vecs
        .iter()
        .step_by((rows / (20 * nlist)).max(1))
        .take(20 * nlist)
        .cloned()
        .collect();
    println!(
        "{{\"rows\":{rows},\"dim\":{dim},\"nlist\":{nlist},\"sample\":{},\"threads\":{threads}}}",
        sample.len()
    );

    let t = Instant::now();
    let old = kmeans_old(&sample, nlist, 8);
    let old_ms = t.elapsed().as_secs_f64() * 1e3;
    let modes = [
        ("once", Assign::default()),
        (
            "once+threads",
            Assign {
                lanes: false,
                threads,
            },
        ),
        (
            "lanes",
            Assign {
                lanes: true,
                threads: 1,
            },
        ),
        (
            "lanes+threads",
            Assign {
                lanes: true,
                threads,
            },
        ),
    ];
    println!("{{\"step\":\"kmeans 8 rounds\",\"mode\":\"old\",\"ms\":{old_ms:.0}}}");
    let mut cents_lanes = Vec::new();
    for (name, how) in modes {
        let t = Instant::now();
        let c = if how == Assign::default() {
            kmeans(&sample, nlist, 8)
        } else {
            kmeans_with(&sample, nlist, 8, how)
        };
        let ms = t.elapsed().as_secs_f64() * 1e3;
        if name == "lanes" {
            cents_lanes = c.clone();
        }
        println!(
            "{{\"step\":\"kmeans 8 rounds\",\"mode\":\"{name}\",\"ms\":{ms:.0},\"speedup\":{:.1},\"same_bits_as_old\":{}}}",
            old_ms / ms,
            bits(&c) == bits(&old)
        );
    }

    // Assigning every row to a list (what ivf_build does), old centroids.
    let t = Instant::now();
    let old_lists: Vec<usize> = vecs.iter().map(|v| nearest_old(&old, dim, v)).collect();
    let old_ms = t.elapsed().as_secs_f64() * 1e3;
    println!("{{\"step\":\"assign all rows\",\"mode\":\"old\",\"ms\":{old_ms:.0}}}");
    for (name, how) in modes {
        let t = Instant::now();
        let lists = assign(&old, dim, &vecs, how);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        let same = lists.iter().zip(&old_lists).filter(|(a, b)| a == b).count();
        println!(
            "{{\"step\":\"assign all rows\",\"mode\":\"{name}\",\"ms\":{ms:.0},\"speedup\":{:.1},\"rows_in_same_list\":{same},\"of\":{rows}}}",
            old_ms / ms
        );
    }
    // How far lanes' own centroids drift from the old ones: rows whose list
    // differs when each side uses its own centroids.
    let lanes_lists = assign(
        &cents_lanes,
        dim,
        &vecs,
        Assign {
            lanes: true,
            threads,
        },
    );
    let same = lanes_lists
        .iter()
        .zip(&old_lists)
        .filter(|(a, b)| a == b)
        .count();
    println!(
        "{{\"step\":\"lanes trained and assigned vs old\",\"rows_in_same_list\":{same},\"of\":{rows}}}"
    );
}
