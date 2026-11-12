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
        assert_eq!(
            got.iter().map(|g| &g.0).collect::<Vec<_>>(),
            want.iter().map(|w| &w.0).collect::<Vec<_>>(),
            "round {round}"
        );
    }
}
