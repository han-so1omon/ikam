//! E253: an IVF view probing every list finds the exact nearest rows, and
//! an updated view holds the rows a build holds, in the same lists.

use ikam_kernel::vectors::{dot, from_f32s, kmeans, to_f32s};
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
    fn unit(&mut self, dim: usize) -> Vec<f32> {
        let v: Vec<f32> = (0..dim)
            .map(|_| (self.next() % 2001) as f32 / 1000.0 - 1.0)
            .collect();
        let n = dot(&v, &v).sqrt();
        v.into_iter().map(|x| x / n).collect()
    }
}

#[test]
fn ivf_view_is_exact_when_probing_all_and_updates_like_a_build() {
    let (dim, mut rng) = (16, Lcg(4));
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let start: Vec<Event> = (0..600)
        .map(|i| {
            Event::upsert(
                &format!("v/{i:05}"),
                &from_f32s(&rng.unit(dim)),
                &format!("s{i}"),
            )
        })
        .collect();
    let mut db = repo.db_append(&empty, &start).unwrap();
    let sample: Vec<Vec<f32>> = (0..200).map(|_| rng.unit(dim)).collect();
    let cents = kmeans(&sample, 8, 5);
    let mut ivf = repo.ivf_build(&db, "v/", &cents, dim).unwrap();
    for round in 0..15 {
        let batch: Vec<Event> = (0..1 + rng.next() % 40)
            .map(|j| {
                let k = format!("v/{:05}", rng.next() % 700);
                let idem = format!("r{round}-{j}");
                match rng.next() % 4 {
                    0 => Event::remove(&k, false, &idem),
                    _ => Event::upsert(&k, &from_f32s(&rng.unit(dim)), &idem),
                }
            })
            .collect();
        let next = repo.db_append(&db, &batch).unwrap();
        ivf = repo.ivf_update(&ivf, &db, &next, "v/").unwrap();
        db = next;
        let built = repo.ivf_build(&db, "v/", &cents, dim).unwrap();
        let labels = |id| {
            repo.graph_nodes(&id)
                .unwrap()
                .into_iter()
                .map(|n| n.label)
                .collect::<Vec<_>>()
        };
        assert_eq!(labels(ivf.lists), labels(built.lists), "round {round}");
        let q = rng.unit(dim);
        let mut want: Vec<(String, f32)> = repo
            .db_scan(&db)
            .unwrap()
            .into_iter()
            .filter(|(k, _)| k.starts_with("v/"))
            .map(|(k, v)| (k, dot(&q, &to_f32s(&v))))
            .collect();
        want.sort_by(|a, b| b.1.total_cmp(&a.1));
        want.truncate(5);
        let got = repo.ivf_search(&ivf, &db, &q, 5, 8, 1000).unwrap();
        // E325: the loaded index returns the same rows, with and without
        // vectors kept, also when it probes and rescores fewer.
        for (keep, lanes) in [(false, false), (true, false), (false, true), (true, true)] {
            let loaded = repo
                .ivf_load_with(&ivf, &db, keep)
                .unwrap()
                .with_lanes(lanes);
            let l = loaded.search(&repo, &db, &q, 5, 8, 1000).unwrap();
            assert_eq!(
                l.iter().map(|g| &g.0).collect::<Vec<_>>(),
                got.iter().map(|g| &g.0).collect::<Vec<_>>()
            );
            let few = repo.ivf_search(&ivf, &db, &q, 5, 2, 20).unwrap();
            let lf = loaded.search(&repo, &db, &q, 5, 2, 20).unwrap();
            assert_eq!(
                lf.iter().map(|g| &g.0).collect::<Vec<_>>(),
                few.iter().map(|g| &g.0).collect::<Vec<_>>()
            );
        }
        assert_eq!(
            got.iter().map(|g| &g.0).collect::<Vec<_>>(),
            want.iter().map(|w| &w.0).collect::<Vec<_>>(),
            "round {round}"
        );
    }
}

/// E343: k-means scores each centroid once, and threads never change a
/// result: the same centroid bits as the old step, which scored each twice.
#[test]
fn kmeans_assign_once_and_threads_match_old_bits() {
    use ikam_kernel::vectors::{Assign, assign, dot, kmeans, kmeans_with};
    fn nearest_old(cents: &[f32], dim: usize, v: &[f32]) -> usize {
        (0..cents.len() / dim)
            .max_by(|&a, &b| {
                dot(&cents[a * dim..][..dim], v).total_cmp(&dot(&cents[b * dim..][..dim], v))
            })
            .unwrap_or(0)
    }
    let mut x = 9u64;
    let mut next = || {
        x = x
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((x >> 33) % 2001) as f32 / 1000.0 - 1.0
    };
    let dim = 37;
    let mut sample: Vec<Vec<f32>> = (0..600)
        .map(|_| {
            let v: Vec<f32> = (0..dim).map(|_| next()).collect();
            let n = dot(&v, &v).sqrt();
            v.into_iter().map(|a| a / n).collect()
        })
        .collect();
    // Exact duplicates make ties: the later centroid must still win.
    sample[5] = sample[0].clone();
    let cents = kmeans(&sample, 24, 4);
    let threaded = kmeans_with(
        &sample,
        24,
        4,
        Assign {
            lanes: false,
            threads: 3,
        },
    );
    let b = |v: &[f32]| v.iter().map(|a| a.to_bits()).collect::<Vec<_>>();
    assert_eq!(b(&cents), b(&threaded));
    let old: Vec<usize> = sample.iter().map(|v| nearest_old(&cents, dim, v)).collect();
    assert_eq!(assign(&cents, dim, &sample, Assign::default()), old);
    assert_eq!(
        assign(
            &cents,
            dim,
            &sample,
            Assign {
                lanes: false,
                threads: 5
            }
        ),
        old
    );
    let tie: Vec<f32> = [sample[0].clone(), sample[0].clone()].concat();
    assert_eq!(assign(&tie, dim, &sample[..1], Assign::default()), vec![1]);
}
