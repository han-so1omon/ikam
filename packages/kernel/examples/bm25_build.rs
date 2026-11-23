//! E382: what building a table's BM25 view costs, by phase.
//!
//! `cargo run --release --example bm25_build -- ROWS [build|update]`
//!
//! `build` (the default) is `bm25_live_build`; `update` builds the same
//! view the way it was built before, as an update from an empty view and
//! an empty database. Each prints the view's root, which must be equal,
//! the build's time and the process's peak memory (VmHWM, Linux).
//!
//! Rows like `experiments/goal-scorecard/db_first_write.py`'s: a JSON array
//! of a row number, a date, one of 5,000 words, a float and a run of x's.

use std::time::Instant;

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
    let mut rng = Lcg(42);
    let mut repo = Repo::new(MemStore::default());
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
    let v = ikam_kernel::bm25::Bm25View { root };
    let hits = repo.bm25_view_search(&v, "word42", 10).unwrap();
    println!(
        "{{\"mode\":\"{mode}\",\"rows\":{rows},\"build_s\":{build:.2},\"peak_mb_before\":{before},\"peak_mb\":{},\"hits\":{},\"root\":\"{root}\"}}",
        peak_mb(),
        hits.len(),
    );
}
