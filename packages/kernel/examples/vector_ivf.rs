//! E323: the IVF vector view (E253) and embedding cache (E255) on main's
//! line. Clustered unit vectors as database rows; build and update times,
//! and recall@10 against an exact scan at each `nprobe`.
//!
//! `cargo run --release --example vector_ivf -- [rows] [dim]`

use std::time::Instant;

use ikam_kernel::vectors::{dot, from_f32s, kmeans};
use ikam_kernel::{Event, MemStore, Repo};

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
        // Sum of 4 uniforms: close enough to normal for clustering.
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

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let (rows, dim) = (
        args.first().copied().unwrap_or(20_000),
        args.get(1).copied().unwrap_or(256),
    );
    let mut rng = Lcg(7);
    let topics: Vec<Vec<f32>> = (0..64)
        .map(|_| unit((0..dim).map(|_| rng.gauss()).collect()))
        .collect();
    let point = |rng: &mut Lcg| {
        let t = &topics[(rng.next() % 64) as usize];
        unit(t.iter().map(|x| x + 0.08 * rng.gauss()).collect())
    };
    let vecs: Vec<Vec<f32>> = (0..rows).map(|_| point(&mut rng)).collect();
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let events: Vec<Event> = vecs
        .iter()
        .enumerate()
        .map(|(i, v)| Event::upsert(&format!("v/{i:07}"), &from_f32s(v), &format!("s{i}")))
        .collect();
    let db = repo.db_append(&empty, &events).unwrap();
    let nlist = ((rows as f64).sqrt() as usize).clamp(1, 1024);
    let t = Instant::now();
    let sample: Vec<Vec<f32>> = vecs
        .iter()
        .step_by((rows / (nlist * 20)).max(1))
        .cloned()
        .collect();
    let cents = kmeans(&sample, nlist, 8);
    let train_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let ivf = repo.ivf_build(&db, "v/", &cents, dim).unwrap();
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    // An update of 1% of the rows.
    let edits: Vec<Event> = (0..rows / 100)
        .map(|j| {
            let i = (rng.next() as usize) % rows;
            Event::upsert(
                &format!("v/{i:07}"),
                &from_f32s(&point(&mut rng)),
                &format!("u{j}"),
            )
        })
        .collect();
    let next = repo.db_append(&db, &edits).unwrap();
    let t = Instant::now();
    let ivf2 = repo.ivf_update(&ivf, &db, &next, "v/").unwrap();
    let update_ms = t.elapsed().as_secs_f64() * 1e3;
    let all: Vec<(String, Vec<f32>)> = repo
        .db_scan(&next)
        .unwrap()
        .into_iter()
        .map(|(k, v)| (k, ikam_kernel::vectors::to_f32s(&v)))
        .collect();
    let queries: Vec<Vec<f32>> = (0..50).map(|_| point(&mut rng)).collect();
    let truth: Vec<Vec<String>> = queries
        .iter()
        .map(|q| {
            let mut s: Vec<(f32, &String)> = all.iter().map(|(k, v)| (dot(q, v), k)).collect();
            s.sort_by(|a, b| b.0.total_cmp(&a.0));
            s.iter().take(10).map(|x| x.1.clone()).collect()
        })
        .collect();
    println!(
        "{{\"rows\":{rows},\"dim\":{dim},\"nlist\":{nlist},\"train_ms\":{train_ms:.1},\"build_ms\":{build_ms:.1},\"update_rows\":{},\"update_ms\":{update_ms:.1}}}",
        rows / 100
    );
    for nprobe in [1, 2, 4, 8, 16, 32, 64] {
        if nprobe > nlist {
            break;
        }
        let (mut hit, t) = (0, Instant::now());
        for (q, want) in queries.iter().zip(&truth) {
            let got = repo.ivf_search(&ivf2, &next, q, 10, nprobe, 100).unwrap();
            hit += got.iter().filter(|g| want.contains(&g.0)).count();
        }
        let ms = t.elapsed().as_secs_f64() * 1e3 / queries.len() as f64;
        println!(
            "{{\"nprobe\":{nprobe},\"recall10\":{:.3},\"query_ms\":{ms:.2}}}",
            hit as f64 / (10 * queries.len()) as f64
        );
    }
    // E325: the same queries on the loaded index, rows read from the
    // database or kept in memory.
    for (keep, lanes) in [(false, false), (true, false), (true, true)] {
        let t = Instant::now();
        let loaded = repo
            .ivf_load_with(&ivf2, &next, keep)
            .unwrap()
            .with_lanes(lanes);
        let load_ms = t.elapsed().as_secs_f64() * 1e3;
        for (nprobe, rescore) in [(1, 100), (4, 10), (4, 30), (4, 100), (16, 100), (64, 100)] {
            if nprobe > nlist {
                break;
            }
            let (mut hit, t) = (0, Instant::now());
            for (q, want) in queries.iter().zip(&truth) {
                let got = loaded.search(&repo, &next, q, 10, nprobe, rescore).unwrap();
                hit += got.iter().filter(|g| want.contains(&g.0)).count();
            }
            let ms = t.elapsed().as_secs_f64() * 1e3 / queries.len() as f64;
            println!(
                "{{\"loaded\":true,\"vectors_kept\":{keep},\"lanes\":{lanes},\"load_ms\":{load_ms:.0},\"nprobe\":{nprobe},\"rescore\":{rescore},\"recall10\":{:.3},\"query_ms\":{ms:.3}}}",
                hit as f64 / (10 * queries.len()) as f64
            );
        }
    }
    // Where a query's time goes: the rescore reads rows from the database.
    for rescore in [10, 30, 100, 300] {
        let (mut hit, t) = (0, Instant::now());
        for (q, want) in queries.iter().zip(&truth) {
            let got = repo.ivf_search(&ivf2, &next, q, 10, 4, rescore).unwrap();
            hit += got.iter().filter(|g| want.contains(&g.0)).count();
        }
        let ms = t.elapsed().as_secs_f64() * 1e3 / queries.len() as f64;
        println!(
            "{{\"nprobe\":4,\"rescore\":{rescore},\"recall10\":{:.3},\"query_ms\":{ms:.2}}}",
            hit as f64 / (10 * queries.len()) as f64
        );
    }
}
