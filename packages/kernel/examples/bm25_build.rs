//! E382: what building a table's BM25 view costs, by phase.
//!
//! `cargo run --release --example bm25_build -- ROWS [build|update|stats|packed] [PACK]`
//!
//! `stats` (E403) is `build` that also gathers the rows' column statistics
//! in the same pass, keeps them as a derived content, and compares them with
//! an exact second pass over the same values (one JSON line on stdout).
//!
//! `zones` (E405) is `stats` with zone maps of ZONE rows (a fourth
//! argument, 4096 by default): it compares them with an exact pass over each
//! zone's values, and times four filtered reads (a range of row numbers, a
//! range of the float, one word, one date) over the whole table and over
//! only the zones the maps say can match, which must find the same rows.
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
    let objects = repo.store().ids().unwrap();
    let t = Instant::now();
    let zone_rows: u64 = std::env::args().nth(3).map_or(4096, |a| a.parse().unwrap());
    let mut stats = if mode == "zones" {
        ikam_kernel::colstats::TableStats::with_zones(zone_rows)
    } else {
        ikam_kernel::colstats::TableStats::default()
    };
    let root = match mode.as_str() {
        "plain" => repo.bm25_view_build(&db, "t/big/").unwrap().root,
        "stats" | "zones" => {
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
    // The build's peak, read before the exact pass below allocates its own.
    let peak = peak_mb();
    let exact = if mode == "stats" {
        compare(&mut stats, &values)
    } else if mode == "zones" {
        let zones = compare_zones(&stats, &values);
        format!(
            "{{\"zones\":{zones},\"reads\":{},\"kept_bytes\":{}}}",
            filtered_reads(&repo, &db, &stats),
            stats.to_json().len()
        )
    } else {
        "null".into()
    };
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
        "{{\"mode\":\"{mode}\",\"pack\":{},\"rows\":{rows},\"build_s\":{build:.2},\"peak_mb_before\":{before},\"peak_mb\":{peak},\"objects_added\":{added},\"bytes_added\":{added_bytes},\"first_ms\":[{}],\"q200_s\":{total:.3},\"q_p50_ms\":{:.3},\"q_p99_ms\":{:.3},\"hits_hash\":\"{:016x}\",\"root\":\"{root}\",\"stats\":{exact}}}",
        if mode == "packed" { pack } else { 0 },
        first.join(","),
        times[100],
        times[198],
        std::hash::Hasher::finish(&h),
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

/// A row's field `col` as text, quotes taken off a string.
fn field(v: &[u8], col: usize) -> &str {
    let s = std::str::from_utf8(v).unwrap();
    s.trim_matches(['[', ']'])
        .split(", ")
        .nth(col)
        .unwrap()
        .trim_matches('"')
}

/// E405: each zone's maps against an exact pass over its own rows (the
/// rows' keys sort as their numbers, so zone `n` holds rows `n*Z..`).
fn compare_zones(stats: &ikam_kernel::colstats::TableStats, values: &[String]) -> String {
    let z = stats.zones.as_ref().unwrap();
    let zr = z.zone_rows as usize;
    let mut equal = z.zones.len() == values.len().div_ceil(zr);
    for (n, zone) in z.zones.iter().enumerate() {
        let part = &values[n * zr..((n + 1) * zr).min(values.len())];
        for col in 0..5 {
            let c = &zone.columns[stats.columns[&col.to_string()].idx()];
            equal &= c.count == part.len() as u64 && c.nulls == 0;
            if col == 0 || col == 3 {
                let xs: Vec<f64> = part
                    .iter()
                    .map(|v| field(v.as_bytes(), col).parse().unwrap())
                    .collect();
                let lo = xs.iter().copied().fold(f64::INFINITY, f64::min);
                let hi = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                equal &= c.num == Some((lo, hi)) && c.str.is_none();
            } else {
                let lo = part.iter().map(|v| field(v.as_bytes(), col)).min().unwrap();
                let hi = part.iter().map(|v| field(v.as_bytes(), col)).max().unwrap();
                equal &= c.str == Some((lo.as_bytes().to_vec(), hi.as_bytes().to_vec()));
            }
        }
    }
    format!(
        "{{\"zones\":{},\"zone_rows\":{zr},\"equal_exact\":{equal}}}",
        z.zones.len()
    )
}

type Keep = Box<dyn Fn(&str) -> bool>;

/// E405: four filters read over the whole table and over only the zones
/// that can match; the rows found, the zones read and each read's p50 of 5.
fn filtered_reads(
    repo: &Repo<MemStore>,
    db: &ikam_kernel::Db,
    stats: &ikam_kernel::colstats::TableStats,
) -> String {
    use ikam_kernel::colstats::Filter;
    let z = stats.zones.as_ref().unwrap();
    let filters: [(&str, usize, Filter, Keep); 4] = [
        (
            "row_range",
            0,
            Filter::Range(100_000.0, 100_999.0),
            Box::new(|t| (100_000.0..=100_999.0).contains(&t.parse::<f64>().unwrap())),
        ),
        (
            "float_range",
            3,
            Filter::Range(500.0, 500.999),
            Box::new(|t| (500.0..=500.999).contains(&t.parse::<f64>().unwrap())),
        ),
        (
            "word",
            2,
            Filter::Eq(b"word42"),
            Box::new(|t| t == "word42"),
        ),
        (
            "date",
            1,
            Filter::Eq(b"2026-01-05"),
            Box::new(|t| t == "2026-01-05"),
        ),
    ];
    let p50 = |f: &mut dyn FnMut() -> usize| {
        let mut ts = Vec::new();
        let mut found = 0;
        for _ in 0..5 {
            let t = Instant::now();
            found = f();
            ts.push(t.elapsed().as_secs_f64() * 1e3);
        }
        ts.sort_by(f64::total_cmp);
        (ts[2], found)
    };
    let mut out = Vec::new();
    for (name, col, filter, keep) in &filters {
        let (full_ms, full) = p50(&mut || {
            repo.db_prefix(db, "t/big/", usize::MAX, false)
                .unwrap()
                .iter()
                .filter(|(_, v)| keep(field(v, *col)))
                .count()
        });
        let read = stats.zones_to_read(&col.to_string(), filter);
        let (zoned_ms, zoned) = p50(&mut || {
            read.iter()
                .map(|&i| {
                    let start = z.zones[i].first_key.as_deref().unwrap();
                    let end = z.zones.get(i + 1).and_then(|n| n.first_key.as_deref());
                    repo.db_range(db, start, end.or(Some("t/big0")), usize::MAX, false)
                        .unwrap()
                        .iter()
                        .filter(|(_, v)| keep(field(v, *col)))
                        .count()
                })
                .sum()
        });
        out.push(format!(
            "\"{name}\":{{\"rows\":{full},\"same_rows\":{},\"zones_read\":{},\"zones\":{},\"full_ms\":{full_ms:.1},\"zoned_ms\":{zoned_ms:.1}}}",
            full == zoned,
            read.len(),
            z.zones.len()
        ));
    }
    format!("{{{}}}", out.join(","))
}
