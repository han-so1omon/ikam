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

#[test]
fn fusion_orders_as_its_formula() {
    use ikam_kernel::vectors::{convex, rrf};
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let r = rrf(&[s(&["a", "b", "c"]), s(&["c", "a", "d"])], 60.0);
    let keys: Vec<&str> = r.iter().map(|x| x.0.as_str()).collect();
    assert_eq!(keys, ["a", "c", "b", "d"]);
    assert!((r[0].1 - (1.0 / 61.0 + 1.0 / 62.0)).abs() < 1e-6);
    let a = vec![("x".to_string(), 10.0), ("y".to_string(), 0.0)];
    let b = vec![("y".to_string(), 3.0), ("z".to_string(), 1.0)];
    let c = convex(&a, &b, 0.5);
    assert_eq!(c[0].0, "x");
    assert!((c[0].1 - 0.5).abs() < 1e-6 && (c[1].1 - 0.5).abs() < 1e-6 && c[2].1 == 0.0);
}
