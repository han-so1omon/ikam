//! E382: what building a table's BM25 view costs, by phase.
//!
//! `cargo run --release --example bm25_build -- ROWS [build|update|packed] [PACK]`
//!
//! `build` (the default) is `bm25_live_build`; `update` builds the same
//! view the way it was built before, as an update from an empty view and
//! an empty database. Each prints the view's root, which must be equal,
//! the build's time and the process's peak memory (VmHWM, Linux).
//! `packed` (E408) is `build` with `Repo::bm25_pack` set to PACK bytes
//! (65536 if not given). Every mode also prints the objects and stored
//! bytes the build added, the time of a first search per query term kind
//! and the fastest of three runs of 200 queries (p50, p99) and a hash of their ranked keys and score bits, which must be
//! equal across modes.
//!
//! Rows like `experiments/goal-scorecard/db_first_write.py`'s: a JSON array
//! of a row number, a date, one of 5,000 words, a float and a run of x's.

use std::time::Instant;

use ikam_kernel::{Event, MemStore, Repo, Store};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn peak_mb() -> usize {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse::<usize>().ok())
        })
        .map_or(0, |kb| kb / 1024)
}

fn main() {
    let rows: usize = std::env::args()
        .nth(1)
        .map_or(200_000, |a| a.parse().unwrap());
    let mode = std::env::args().nth(2).unwrap_or_else(|| "build".into());
    let pack: usize = std::env::args()
        .nth(3)
        .map_or(65536, |a| a.parse().unwrap());
    let mut rng = Lcg(42);
    let mut repo = Repo::new(MemStore::default());
    if mode == "packed" {
        repo.bm25_pack = pack;
    }
    let mut db = repo.db_empty().unwrap();
    let t = Instant::now();
    for start in (0..rows).step_by(10_000) {
        let events: Vec<Event> = (start..(start + 10_000).min(rows))
            .map(|i| {
                let v = format!(
                    "[{i}, \"2026-01-{:02}\", \"word{}\", {}.{:016}, \"{}\"]",
                    1 + i % 28,
                    rng.next() % 5000,
                    rng.next() % 1000,
                    rng.next(),
                    "x".repeat(40 + (rng.next() % 120) as usize)
                );
                Event::upsert(&format!("t/big/p{i:019}"), v.as_bytes(), &format!("r{i}"))
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
    }
    eprintln!("load {:.1} s", t.elapsed().as_secs_f64());
    let before = peak_mb();
    let objects = repo.store().ids().unwrap();
    let t = Instant::now();
    let root = match mode.as_str() {
        "update" => {
            let (empty, blank) = (repo.db_empty().unwrap(), repo.bm25_view_empty().unwrap());
            repo.bm25_view_update(&blank, &empty, &db, "t/big/")
                .unwrap()
                .root
        }
        _ => {
            repo.bm25_live_build(&db, "t/big/", rows / 20)
                .unwrap()
                .base
                .root
        }
    };
    let build = t.elapsed().as_secs_f64();
    let had: std::collections::HashSet<_> = objects.into_iter().collect();
    let (mut added, mut added_bytes) = (0usize, 0usize);
    for id in repo.store().ids().unwrap() {
        if !had.contains(&id) {
            added += 1;
            added_bytes += repo.store().stored_len(&id).unwrap();
        }
    }
    let v = ikam_kernel::bm25::Bm25View { root };
    // First searches: a common word, a rare number, three terms at once.
    let mut first = Vec::new();
    // The rare number is one the rows hold at any row count.
    let rare = format!("{}", 12345 % rows);
    for q in ["word42", rare.as_str(), "word7 77 2026"] {
        let t = Instant::now();
        let hits = repo.bm25_view_search(&v, q, 10).unwrap();
        first.push(format!("{:.2}", t.elapsed().as_secs_f64() * 1e3));
        assert!(!hits.is_empty(), "{q}");
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let t = Instant::now();
    // Each query three times, its fastest run kept: the CPU here is shared.
    let mut times = vec![f64::MAX; 200];
    for pass in 0..3 {
        for i in 0..200u64 {
            let q = match i % 4 {
                0 => format!("word{}", i * 37 % 5000),
                1 => format!("{}", i * 997 % rows as u64),
                2 => format!("word{} {}", i % 5000, i * 13 % 1000),
                _ => format!("2026 01 {:02} word{}", 1 + i % 28, i),
            };
            let s = Instant::now();
            let hits = repo.bm25_view_search(&v, &q, 10).unwrap();
            let ms = s.elapsed().as_secs_f64() * 1e3;
            times[i as usize] = times[i as usize].min(ms);
            if pass == 0 {
                for (k, s) in hits {
                    std::hash::Hash::hash(&k, &mut h);
                    std::hash::Hash::hash(&s.to_bits(), &mut h);
                }
            }
        }
    }
    let total = t.elapsed().as_secs_f64();
    times.sort_by(f64::total_cmp);
    println!(
        "{{\"mode\":\"{mode}\",\"pack\":{},\"rows\":{rows},\"build_s\":{build:.2},\"peak_mb_before\":{before},\"peak_mb\":{},\"objects_added\":{added},\"bytes_added\":{added_bytes},\"first_ms\":[{}],\"q200_s\":{total:.3},\"q_p50_ms\":{:.3},\"q_p99_ms\":{:.3},\"hits_hash\":\"{:016x}\",\"root\":\"{root}\"}}",
        if mode == "packed" { pack } else { 0 },
        peak_mb(),
        first.join(","),
        times[100],
        times[198],
        std::hash::Hasher::finish(&h),
    );
}
