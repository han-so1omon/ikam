//! E253: vector indexes kept as views of a database, on synthetic
//! clustered unit vectors (100 clusters, a center plus noise, 0.4 a component).
//! `cargo run --release --features vectors-hnsw --example vector_views -- N DIM BATCH...`
//!
//! Ways: `exact` (every row read from ikam and scored), `flat_int8` (an
//! IVF view of one list: every int8 code scored, the best 40 rescored),
//! `ivf_int8` (sqrt(N) lists, 8 or 32 probed, the best 40 rescored), and
//! `hnsw` (hnsw_rs in memory, M 16, ef 100 to build and 64 to search;
//! deletes as tombstones filtered during the search). Prints build time,
//! stored bytes, query time and recall@10 against exact search, then
//! update times for batches of changes (70% new vectors for existing rows,
//! 15% new rows, 15% removes) and recall after them. Synthetic data only.

mod db_common;

use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

use db_common::{Counting, Lcg};
use hnsw_rs::prelude::{DistCosine, Hnsw};
use ikam_kernel::vectors::{dot, from_f32s, kmeans};
use ikam_kernel::{Db, Event, Repo};

const K: usize = 10;

/// A row's new vector, or its removal.
type Edit = (String, Option<Vec<f32>>);

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn unit(v: Vec<f32>) -> Vec<f32> {
    let n = dot(&v, &v).sqrt();
    v.into_iter().map(|x| x / n).collect()
}

fn noise(rng: &mut Lcg, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|_| (rng.next() % 2001) as f32 / 1000.0 - 1.0)
        .collect()
}

fn point(rng: &mut Lcg, centers: &[Vec<f32>]) -> Vec<f32> {
    let c = &centers[rng.next() as usize % centers.len()];
    let n = noise(rng, c.len());
    unit(c.iter().zip(n).map(|(a, b)| a + 0.4 * b).collect())
}

fn key(i: usize) -> String {
    format!("v/{i:08}")
}

/// Exact top `K` keys over `rows`.
fn exact(rows: &BTreeMap<String, Vec<f32>>, q: &[f32]) -> Vec<String> {
    let mut s: Vec<(f32, &String)> = rows.iter().map(|(k, v)| (dot(q, v), k)).collect();
    s.sort_by(|a, b| b.0.total_cmp(&a.0));
    s.into_iter().take(K).map(|x| x.1.clone()).collect()
}

fn recall(got: &[String], want: &[String]) -> f64 {
    let w: HashSet<&String> = want.iter().collect();
    got.iter().filter(|g| w.contains(g)).count() as f64 / want.len() as f64
}

/// HNSW over the rows, with ids handed out in insertion order.
struct Graph<'a> {
    h: Hnsw<'a, f32, DistCosine>,
    keys: Vec<String>,
    id_of: BTreeMap<String, usize>,
    dead: HashSet<usize>,
}

impl Graph<'_> {
    fn insert(&mut self, k: &str, v: &[f32]) {
        if let Some(old) = self.id_of.remove(k) {
            self.dead.insert(old);
        }
        let id = self.keys.len();
        self.h.insert((v, id));
        self.keys.push(k.to_string());
        self.id_of.insert(k.to_string(), id);
    }
    fn remove(&mut self, k: &str) {
        if let Some(old) = self.id_of.remove(k) {
            self.dead.insert(old);
        }
    }
    fn search(&self, q: &[f32]) -> Vec<String> {
        let alive = |id: &usize| !self.dead.contains(id);
        let f: &dyn hnsw_rs::filter::FilterT = &alive;
        self.h
            .search_filter(q, K, 64, Some(f))
            .into_iter()
            .map(|n| self.keys[n.d_id].clone())
            .collect()
    }
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let (n, dim, batches) = (args[0], args[1], args[2..].to_vec());
    let mut rng = Lcg(5);
    let centers: Vec<Vec<f32>> = (0..100).map(|_| unit(noise(&mut rng, dim))).collect();
    let mut rows: BTreeMap<String, Vec<f32>> = (0..n)
        .map(|i| (key(i), point(&mut rng, &centers)))
        .collect();
    let queries: Vec<Vec<f32>> = (0..200).map(|_| point(&mut rng, &centers)).collect();
    let mut repo = Repo::new(Counting::default());
    let mut db = repo.db_empty().unwrap();
    let events: Vec<Event> = rows
        .iter()
        .map(|(k, v)| Event::upsert(k, &from_f32s(v), k))
        .collect();
    for chunk in events.chunks(10_000) {
        db = repo.db_append(&db, chunk).unwrap();
    }
    repo.object_templates = false;
    repo.prune_diff = true; // E249
    let sample: Vec<Vec<f32>> = rows.values().step_by((n / 5000).max(1)).cloned().collect();
    let nlist = (n as f64).sqrt() as usize;
    let t = Instant::now();
    let cents = kmeans(&sample, nlist, 8);
    println!(
        "{{\"rows\":{n},\"dim\":{dim},\"kmeans_ms\":{:.0},\"nlist\":{nlist},\"sample\":{}}}",
        ms(t),
        sample.len()
    );

    // The changes, made up front.
    let mut next_key = n;
    let mut idem = 0;
    let rounds: Vec<(usize, Vec<Edit>)> = batches
        .iter()
        .flat_map(|&size| std::iter::repeat_n(size, 5))
        .map(|size| {
            let edits = (0..size)
                .map(|_| match rng.next() % 20 {
                    0..=2 => {
                        next_key += 1;
                        (key(next_key), Some(point(&mut rng, &centers)))
                    }
                    3..=5 => (key(rng.next() as usize % n), None),
                    _ => (
                        key(rng.next() as usize % n),
                        Some(point(&mut rng, &centers)),
                    ),
                })
                .collect();
            (size, edits)
        })
        .collect();
    let mut dbs = vec![db];
    for (_, edits) in &rounds {
        let ev: Vec<Event> = edits
            .iter()
            .map(|(k, v)| {
                idem += 1;
                let id = format!("u{idem}");
                match v {
                    Some(v) => Event::upsert(k, &from_f32s(v), &id),
                    None => Event::remove(k, false, &id),
                }
            })
            .collect();
        dbs.push(repo.db_append(dbs.last().unwrap(), &ev).unwrap());
    }
    let want0: Vec<Vec<String>> = queries.iter().map(|q| exact(&rows, q)).collect();
    let mut after = rows.clone();
    for (_, edits) in &rounds {
        for (k, v) in edits {
            match v {
                Some(v) => after.insert(k.clone(), v.clone()),
                None => after.remove(k),
            };
        }
    }
    let want1: Vec<Vec<String>> = queries.iter().map(|q| exact(&after, q)).collect();
    let report = |way: &str,
                  phase: &str,
                  times: &[f64],
                  got: &[Vec<String>],
                  want: &[Vec<String>]| {
        let r = got.iter().zip(want).map(|(g, w)| recall(g, w)).sum::<f64>() / got.len() as f64;
        let t = times.iter().sum::<f64>() / times.len() as f64;
        println!(
            "{{\"rows\":{n},\"way\":\"{way}\",\"phase\":\"{phase}\",\"query_ms\":{t:.3},\"recall_at_10\":{r:.4}}}"
        );
    };

    // Exact search reading every row from ikam.
    let scan = |db: &Db| -> BTreeMap<String, Vec<f32>> {
        repo.db_scan(db)
            .unwrap()
            .into_iter()
            .map(|(k, v)| (k, ikam_kernel::vectors::to_f32s(&v)))
            .collect()
    };
    let (mut times, mut got) = (vec![], vec![]);
    for q in queries.iter().take(20) {
        let t = Instant::now();
        got.push(exact(&scan(&dbs[0]), q));
        times.push(ms(t));
    }
    report("exact", "before", &times, &got, &want0[..20]);

    // IVF views (one list: a flat int8 scan).
    for (way, lists, probes) in [
        ("flat_int8", 1, 1),
        ("ivf_int8_probe8", nlist, 8),
        ("ivf_int8_probe32", nlist, 32),
    ] {
        let mut r = Repo::new(repo.store().copy());
        r.object_templates = false;
        r.prune_diff = true;
        let c = if lists == 1 {
            kmeans(&sample, 1, 1)
        } else {
            cents.clone()
        };
        let before = r.store().stored();
        let t = Instant::now();
        let mut ivf = r.ivf_build(&dbs[0], "v/", &c, dim).unwrap();
        println!(
            "{{\"rows\":{n},\"way\":\"{way}\",\"build_ms\":{:.0},\"stored_bytes\":{}}}",
            ms(t),
            r.store().stored() - before
        );
        let search = |r: &Repo<Counting>,
                      ivf: &ikam_kernel::vectors::Ivf,
                      db: &Db,
                      q: &[f32]|
         -> Vec<String> {
            r.ivf_search(ivf, db, q, K, probes, 40)
                .unwrap()
                .into_iter()
                .map(|x| x.0)
                .collect()
        };
        let (mut times, mut got) = (vec![], vec![]);
        for q in &queries {
            let t = Instant::now();
            got.push(search(&r, &ivf, &dbs[0], q));
            times.push(ms(t));
        }
        report(way, "before", &times, &got, &want0);
        let mut per: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
        for (k, (size, _)) in rounds.iter().enumerate() {
            let t = Instant::now();
            ivf = r.ivf_update(&ivf, &dbs[k], &dbs[k + 1], "v/").unwrap();
            per.entry(*size).or_default().push(ms(t));
        }
        for (size, t) in per {
            let m = t.iter().sum::<f64>() / t.len() as f64;
            println!("{{\"rows\":{n},\"way\":\"{way}\",\"batch\":{size},\"update_ms\":{m:.1}}}");
        }
        let last = dbs.last().unwrap();
        let (mut times, mut got) = (vec![], vec![]);
        for q in &queries {
            let t = Instant::now();
            got.push(search(&r, &ivf, last, q));
            times.push(ms(t));
        }
        report(way, "after", &times, &got, &want1);
    }

    // HNSW in memory, fed the same rows and changes.
    let t = Instant::now();
    let mut g = Graph {
        h: Hnsw::new(16, n * 2, 16, 100, DistCosine {}),
        keys: vec![],
        id_of: BTreeMap::new(),
        dead: HashSet::new(),
    };
    for (k, v) in &rows {
        g.insert(k, v);
    }
    println!(
        "{{\"rows\":{n},\"way\":\"hnsw\",\"build_ms\":{:.0}}}",
        ms(t)
    );
    let (mut times, mut got) = (vec![], vec![]);
    for q in &queries {
        let t = Instant::now();
        got.push(g.search(q));
        times.push(ms(t));
    }
    report("hnsw", "before", &times, &got, &want0);
    let mut per: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
    for (size, edits) in &rounds {
        let t = Instant::now();
        for (k, v) in edits {
            match v {
                Some(v) => g.insert(k, v),
                None => g.remove(k),
            }
        }
        per.entry(*size).or_default().push(ms(t));
    }
    for (size, t) in per {
        let m = t.iter().sum::<f64>() / t.len() as f64;
        println!("{{\"rows\":{n},\"way\":\"hnsw\",\"batch\":{size},\"update_ms\":{m:.1}}}");
    }
    let (mut times, mut got) = (vec![], vec![]);
    for q in &queries {
        let t = Instant::now();
        got.push(g.search(q));
        times.push(ms(t));
    }
    report("hnsw", "after", &times, &got, &want1);
    println!(
        "{{\"rows\":{n},\"hnsw_tombstones\":{},\"hnsw_points\":{}}}",
        g.dead.len(),
        g.keys.len()
    );
    rows.clear();
}
