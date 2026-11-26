//! E382: what building a table's BM25 view costs, by phase.
//!
//! `cargo run --release --example bm25_build -- ROWS [build|update|stats]`
//!
//! `stats` (E403) is `build` that also gathers the rows' column statistics
//! in the same pass, keeps them as a derived content, and compares them with
//! an exact second pass over the same values (one JSON line on stdout).
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
    let mut values: Vec<String> = Vec::new();
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
                let e = Event::upsert(&format!("t/big/p{i:019}"), v.as_bytes(), &format!("r{i}"));
                values.push(v);
                e
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
    }
    eprintln!("load {:.1} s", t.elapsed().as_secs_f64());
    let before = peak_mb();
    let t = Instant::now();
    let mut stats = ikam_kernel::colstats::TableStats::default();
    let root = match mode.as_str() {
        "plain" => repo.bm25_view_build(&db, "t/big/").unwrap().root,
        "stats" => {
            let v = repo
                .bm25_view_build_with_stats(&db, "t/big/", &mut stats)
                .unwrap();
            repo.put_table_stats(&mut stats).unwrap();
            v.root
        }
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
    let exact = if mode == "stats" {
        compare(&mut stats, &values)
    } else {
        "null".into()
    };
    let v = ikam_kernel::bm25::Bm25View { root };
    let hits = repo.bm25_view_search(&v, "word42", 10).unwrap();
    println!(
        "{{\"mode\":\"{mode}\",\"rows\":{rows},\"build_s\":{build:.2},\"peak_mb_before\":{before},\"peak_mb\":{},\"hits\":{},\"root\":\"{root}\",\"stats\":{exact}}}",
        peak_mb(),
        hits.len(),
    );
}

/// The stats against an exact second pass over the same values: per column,
/// mean and variance (two-pass) relative error, min and max equal, each
/// quantile's rank error, and the distinct estimate's relative error.
fn compare(stats: &mut ikam_kernel::colstats::TableStats, values: &[String]) -> String {
    let rows: Vec<Vec<serde_json::Value>> = values
        .iter()
        .map(|v| serde_json::from_str(v).unwrap())
        .collect();
    let mut out = Vec::new();
    for col in 0..rows[0].len() {
        let c = stats.columns.get_mut(&col.to_string()).unwrap();
        let texts: std::collections::HashSet<String> = rows
            .iter()
            .map(|r| match &r[col] {
                serde_json::Value::String(s) => s.clone(),
                v => v.to_string(),
            })
            .collect();
        let distinct_err = (c.distinct.estimate() - texts.len() as f64) / texts.len() as f64;
        let mut line = format!(
            "\"{col}\":{{\"distinct\":{},\"distinct_rel_err\":{distinct_err:.4}",
            texts.len()
        );
        if c.numbers > 0 {
            // From each number's own text, as Rust parses it (serde_json's
            // default parse can differ in the last bit).
            let mut xs: Vec<f64> = values
                .iter()
                .map(|v| {
                    v.trim_matches(['[', ']'])
                        .split(", ")
                        .nth(col)
                        .unwrap()
                        .parse()
                        .unwrap()
                })
                .collect();
            let n = xs.len() as f64;
            let mean = xs.iter().sum::<f64>() / n;
            let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
            xs.sort_by(f64::total_cmp);
            let worst = [0.01, 0.1, 0.25, 0.5, 0.75, 0.9, 0.99]
                .iter()
                .map(|q| {
                    let est = c.quantiles.quantile(*q).unwrap();
                    (xs.partition_point(|x| *x < est) as f64 / n - q).abs()
                })
                .fold(0.0, f64::max);
            line += &format!(
                ",\"mean_rel_err\":{:.2e},\"variance_rel_err\":{:.2e},\"min_max_equal\":{},\"quantile_rank_err_max\":{worst:.4},\"centroids\":{}",
                ((c.mean - mean) / mean).abs(),
                ((c.variance() - var) / var).abs(),
                c.min == xs[0] && c.max == xs[xs.len() - 1],
                c.quantiles.centroids()
            );
        }
        out.push(line + "}");
    }
    format!(
        "{{\"rows_equal\":{},{}}}",
        stats.rows == values.len() as u64,
        out.join(",")
    )
}
