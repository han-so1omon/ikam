//! E357: how an append's cost grows with the table, as `ikam db serve`
//! writes it: a pack store on disk, 2,000-row batches, each committed.
//! Rows have the shape `experiments/goal-scorecard/db_first_write.py`
//! sends: a row under `t/big/pNNN` (keys in insertion order) and, with
//! `index`, one index entry under `i/big/by_word/WORD/pNNN` (a word drawn
//! from 5,000, so its keys land all over the table).
//!
//! `cargo run --release --example db_append_scale -- DIR ROWS [BATCH] [live] [index] [plain-hot]`
//!
//! `plain-hot` is `db serve --plain-writes` (`Repo::plain_hot_writes`).
//!
//! DIR is created and must not exist. Prints one JSON line per window of
//! ten batches: rows before it, the p50 of append and commit, and bytes
//! the store grew by a batch.
use std::time::Instant;

use ikam_kernel::backends::{PackOptions, PackStore};
use ikam_kernel::{Event, Repo};

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

/// xorshift64, seeded per row so every run writes the same bytes.
fn rnd(mut s: u64) -> u64 {
    s ^= s << 13;
    s ^= s >> 7;
    s ^= s << 17;
    s
}

fn batch(start: usize, n: usize, index: bool) -> Vec<Event> {
    let mut out = Vec::with_capacity(n * 2);
    for i in start..start + n {
        let r = rnd(0x9e37_79b9_7f4a_7c15 ^ i as u64);
        let word = r % 5000;
        let pk = format!("p{i:019}");
        let pad = "x".repeat(40 + (r >> 20) as usize % 120);
        let v = format!(
            "[{i},\"2026-01-{:02}\",\"word{word}\",{:.4},\"{pad}\"]",
            1 + i % 28,
            (r >> 32) as f64 / 4294967.296
        );
        out.push(Event::upsert(
            &format!("t/big/{pk}"),
            v.as_bytes(),
            &format!("load-{i}"),
        ));
        if index {
            out.push(Event::upsert(
                &format!("i/big/by_word/word{word}/{pk}"),
                pk.as_bytes(),
                &format!("load-i{i}"),
            ));
        }
    }
    out
}

fn median(xs: &mut [f64]) -> f64 {
    xs.sort_by(f64::total_cmp);
    xs[xs.len() / 2]
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dir = a.first().expect("DIR").clone();
    let rows: usize = a.get(1).map_or(200_000, |s| s.parse().unwrap());
    let per: usize = a.get(2).map_or(2000, |s| s.parse().unwrap());
    let live = a.iter().any(|f| f == "live");
    let index = a.iter().any(|f| f == "index");
    let plain_hot = a.iter().any(|f| f == "plain-hot");
    let opts = PackOptions {
        lazy_checkpoint: true,
        ref_log: false,
        filters: true,
    };
    // `db serve --packs --plain-events --no-object-templates`, and with
    // `live` its `--edit-index` (E356; concurrent reads change no write).
    let mut repo = Repo::new(PackStore::open_with(&dir, opts).unwrap());
    repo.plain_db_events = true;
    repo.object_templates = false;
    repo.plain_hot_writes = plain_hot;
    if live {
        repo.set_seed_scope(Some(Vec::new()));
        repo.replace_hints = true;
    }
    let path = std::path::Path::new(&dir);
    let mut db = repo.db_empty().unwrap();
    let (mut app, mut com, mut grew) = (Vec::new(), Vec::new(), Vec::new());
    let mut window_start = 0;
    let t0 = Instant::now();
    for start in (0..rows).step_by(per) {
        let events = batch(start, per.min(rows - start), index);
        let before = dir_bytes(path);
        let t = Instant::now();
        let new = repo.db_append(&db, &events).unwrap();
        app.push(t.elapsed().as_secs_f64() * 1000.0);
        let t = Instant::now();
        repo.db_commit("db/t", &new, "batch").unwrap();
        com.push(t.elapsed().as_secs_f64() * 1000.0);
        grew.push((dir_bytes(path) - before) as f64);
        db = new;
        if app.len() == 10 || start + per >= rows {
            println!(
                "{{\"rows\":{window_start},\"append_ms\":{:.1},\"commit_ms\":{:.1},\
                 \"grew_mb\":{:.2},\"store_mb\":{:.1},\"elapsed_s\":{:.1}}}",
                median(&mut app),
                median(&mut com),
                median(&mut grew) / 1e6,
                dir_bytes(path) as f64 / 1e6,
                t0.elapsed().as_secs_f64()
            );
            app.clear();
            com.clear();
            grew.clear();
            window_start = start + per;
        }
    }
    // Reads after the load: the whole index in key order, and 1,000 rows.
    let t = Instant::now();
    let n = repo
        .db_prefix(&db, "i/big/", usize::MAX, false)
        .unwrap()
        .len();
    let scan_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    for i in (0..rows).step_by((rows / 1000).max(1)) {
        assert!(
            repo.db_get(&db, &format!("t/big/p{i:019}"))
                .unwrap()
                .is_some()
        );
    }
    let get_ms = t.elapsed().as_secs_f64() * 1000.0;
    print!(
        "{{\"done\":{rows},\"store_mb\":{:.1},\"index_rows\":{n},\"scan_index_ms\":{scan_ms:.0},\"get_1000_ms\":{get_ms:.0}",
        dir_bytes(path) as f64 / 1e6
    );
    println!("}}");
}
