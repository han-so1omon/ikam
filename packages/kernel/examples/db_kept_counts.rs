//! E233: what kept counts cost and save. Preloads grouped rows (200 groups,
//! key order) into a pack store, optionally keeping counts during the load
//! or building them after it, then times the count of every group's prefix
//! and of the table both ways, and small appends with the
//! view on or off.
//!
//! `cargo run --release --example db_kept_counts -- DIR [ROWS] [APPENDS] [off|after|load]`
//! (DIR is created and must not exist.)
use std::time::Instant;

use ikam_kernel::backends::{PackOptions, PackStore};
use ikam_kernel::{Event, Repo};

const GROUPS: usize = 200;

/// Row `i` of group `g`: a JSON array of numbers under `t/rows/GROUP/SEQ`.
fn row(g: usize, i: usize) -> (String, Vec<u8>) {
    let key = format!("t/rows/G{g:04}/{i:06}");
    let v = format!(
        "[\"G{g:04}\",{i},{:.4},{:.4},{:.4},{:.4},{},\"bench\",\"2026-01-05T21:00:00+00:00\"]",
        100.0 + (i % 97) as f64 * 0.37,
        101.0 + (g % 13) as f64,
        99.5,
        100.25,
        1000 + i * 7 % 9973
    );
    (key, v.into_bytes())
}

fn dir_bytes(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p)
        .map(|it| {
            it.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_bytes(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn mb(dir: &str) -> f64 {
    dir_bytes(std::path::Path::new(dir)) as f64 / 1e6
}

fn p50(xs: &mut [f64]) -> f64 {
    xs.sort_by(f64::total_cmp);
    xs[xs.len() / 2]
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dir = a.first().expect("DIR").clone();
    let rows: usize = a.get(1).map_or(200_000, |s| s.parse().unwrap());
    let appends: usize = a.get(2).map_or(200, |s| s.parse().unwrap());
    let mode = a.get(3).map_or("after", String::as_str);
    let opts = PackOptions {
        lazy_checkpoint: true,
        ref_log: false,
        filters: true,
    };
    let mut repo = Repo::new(PackStore::open_with(&dir, opts).unwrap());
    repo.plain_db_events = true;
    repo.object_templates = false;
    let mut db = repo.db_empty().unwrap();
    if mode == "load" {
        db = repo.db_keep_counts(&db, 3).unwrap();
    }
    let per_group = rows.div_ceil(GROUPS);
    let t = Instant::now();
    let mut batch = Vec::new();
    for s in 0..GROUPS {
        for d in 0..per_group {
            let (k, v) = row(s, d);
            batch.push(Event::upsert(&k, &v, &format!("load:0:{k}")));
            if batch.len() == 5000 {
                db = repo.db_append(&db, &batch).unwrap();
                batch.clear();
            }
        }
    }
    if !batch.is_empty() {
        db = repo.db_append(&db, &batch).unwrap();
    }
    repo.db_commit("db/bench", &db, "preload").unwrap();
    println!(
        "load {} rows ({mode}): {:.1} s, store {:.1} MB",
        GROUPS * per_group,
        t.elapsed().as_secs_f64(),
        mb(&dir)
    );
    if mode == "after" {
        let t = Instant::now();
        db = repo.db_keep_counts(&db, 3).unwrap();
        let built = ms(t);
        repo.db_commit("db/bench", &db, "keep counts").unwrap();
        println!(
            "build the view after the load: {built:.0} ms, store {:.1} MB",
            mb(&dir)
        );
    }
    // Every group's count, and the table's, as a list view asks.
    let prefixes: Vec<String> = (0..GROUPS)
        .map(|s| format!("t/rows/G{s:04}/"))
        .chain(["t/rows/".to_string()])
        .collect();
    let refs: Vec<&str> = prefixes.iter().map(String::as_str).collect();
    let kept = if db.counts.is_some() {
        let t = Instant::now();
        let kept = repo.db_count_prefixes(&db, &refs).unwrap();
        println!(
            "counts of {} prefixes kept: {:.2} ms",
            prefixes.len(),
            ms(t)
        );
        Some(kept)
    } else {
        None
    };
    let t = Instant::now();
    let by_range: Vec<usize> = prefixes
        .iter()
        .map(|p| {
            let end = ikam_kernel::prefix_end(p);
            repo.db_count(&db, p, end.as_deref()).unwrap()
        })
        .collect();
    println!(
        "counts of {} prefixes by range: {:.1} ms",
        prefixes.len(),
        ms(t)
    );
    if let Some(kept) = kept {
        assert_eq!(kept, by_range, "kept counts equal range counts");
    }
    // Small appends: a new row for one group each, as a steady writer sends.
    let mut took = Vec::new();
    for i in 0..appends {
        let (k, v) = row(i % GROUPS, per_group + i);
        let t = Instant::now();
        db = repo
            .db_append(&db, &[Event::upsert(&k, &v, &format!("live:{i}"))])
            .unwrap();
        repo.db_commit("db/bench", &db, "live").unwrap();
        took.push(ms(t));
    }
    if appends > 0 {
        println!(
            "{appends} one-row appends with commit: p50 {:.2} ms, store {:.1} MB",
            p50(&mut took),
            mb(&dir)
        );
    }
    if let Some(c) = db.counts {
        assert_eq!(repo.db_counts_rebuild(&db.kv, 3).unwrap(), c.root);
    }
}
