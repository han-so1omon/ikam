//! E332: the graph index finds the exact nearest rows on clustered data,
//! survives being stored and loaded, and stays right through updates.

use std::collections::BTreeMap;

use ikam_kernel::hnsw::HnswIndex;
use ikam_kernel::vectors::{dot, from_f32s};
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

const DIM: usize = 32;

fn exact(rows: &BTreeMap<String, Vec<f32>>, q: &[f32], k: usize) -> Vec<String> {
    let mut s: Vec<(f32, &String)> = rows.iter().map(|(r, v)| (dot(q, v), r)).collect();
    s.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    s.into_iter().take(k).map(|x| x.1.clone()).collect()
}

#[test]
fn graph_finds_neighbours_and_survives_storage_and_updates() {
    let mut rng = Lcg(11);
    let topics: Vec<Vec<f32>> = (0..12)
        .map(|_| unit((0..DIM).map(|_| rng.gauss()).collect()))
        .collect();
    let point = |rng: &mut Lcg| {
        let t = &topics[(rng.next() % 12) as usize];
        unit(t.iter().map(|x| x + 0.3 * rng.gauss()).collect())
    };
    let mut rows: BTreeMap<String, Vec<f32>> = (0..2000)
        .map(|i| (format!("v/{i:05}"), point(&mut rng)))
        .collect();
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let mut events: Vec<Event> = rows
        .iter()
        .map(|(k, v)| Event::upsert(k, &from_f32s(v), &format!("s{k}")))
        .collect();
    events.push(Event::upsert("other/x", b"not a vector", "o"));
    let db = repo.db_append(&empty, &events).unwrap();
    let (h, mut index) = repo
        .hnsw_build(&db, "v/", HnswIndex::new(DIM, 8, 64))
        .unwrap();
    assert_eq!(index.len(), 2000);
    let queries: Vec<Vec<f32>> = (0..40).map(|_| point(&mut rng)).collect();
    let recall = |index: &mut HnswIndex, rows: &BTreeMap<_, _>| {
        let mut hit = 0;
        for q in &queries {
            let want = exact(rows, q, 10);
            let got = index.search(q, 10, 64);
            assert!(got.windows(2).all(|w| w[0].1 >= w[1].1));
            hit += got.iter().filter(|g| want.contains(&g.0)).count();
        }
        hit as f64 / (10 * queries.len()) as f64
    };
    let r0 = recall(&mut index, &rows);
    assert!(r0 > 0.95, "recall {r0}");
    // Stored and loaded: the same answers.
    let mut loaded = repo.hnsw_load(&h, &db).unwrap();
    for q in &queries {
        assert_eq!(loaded.search(q, 10, 64), index.search(q, 10, 64));
    }
    // Updates: changed, new and removed rows.
    let mut at = db;
    for round in 0..3 {
        let mut edits = Vec::new();
        for j in 0..150 {
            let i = (rng.next() % 2400) as usize;
            let key = format!("v/{i:05}");
            if j % 5 == 0 && rows.contains_key(&key) {
                rows.remove(&key);
                edits.push(Event::remove(&key, false, &format!("d{round}-{j}")));
            } else {
                let v = point(&mut rng);
                edits.push(Event::upsert(
                    &key,
                    &from_f32s(&v),
                    &format!("u{round}-{j}"),
                ));
                rows.insert(key, v);
            }
        }
        let next = repo.db_append(&at, &edits).unwrap();
        repo.hnsw_update(&mut index, &at, &next, "v/").unwrap();
        at = next;
        let r = recall(&mut index, &rows);
        assert!(r > 0.93, "round {round} recall {r}");
        for q in &queries {
            for (k, s) in index.search(q, 10, 64) {
                let v = rows.get(&k).expect("a removed row came back");
                assert!((dot(q, v) - s).abs() < 1e-5);
            }
        }
    }
    let h2 = repo.hnsw_save(&index).unwrap();
    let mut loaded = repo.hnsw_load(&h2, &at).unwrap();
    for q in &queries {
        assert_eq!(loaded.search(q, 10, 64), index.search(q, 10, 64));
    }
    // A database missing a live row is refused.
    assert!(repo.hnsw_load(&h2, &empty).is_err());
}

#[test]
fn threaded_build_is_deterministic_and_finds_neighbours() {
    let mut rng = Lcg(5);
    let topics: Vec<Vec<f32>> = (0..12)
        .map(|_| unit((0..DIM).map(|_| rng.gauss()).collect()))
        .collect();
    let point = |rng: &mut Lcg| {
        let t = &topics[(rng.next() % 12) as usize];
        unit(t.iter().map(|x| x + 0.3 * rng.gauss()).collect())
    };
    let rows: BTreeMap<String, Vec<f32>> = (0..5000)
        .map(|i| (format!("v/{i:05}"), point(&mut rng)))
        .collect();
    let queries: Vec<Vec<f32>> = (0..40).map(|_| point(&mut rng)).collect();
    let build = |threads| {
        let mut g = HnswIndex::new(DIM, 8, 64).with_threads(threads);
        g.extend(rows.iter().map(|(k, v)| (k.clone(), v.clone())));
        g
    };
    let (mut a, mut b) = (build(3), build(3));
    let mut hit = 0;
    for q in &queries {
        let got = a.search(q, 10, 64);
        assert_eq!(got, b.search(q, 10, 64));
        let want = exact(&rows, q, 10);
        hit += got.iter().filter(|g| want.contains(&g.0)).count();
    }
    let r = hit as f64 / (10 * queries.len()) as f64;
    assert!(r > 0.95, "recall {r}");
}
