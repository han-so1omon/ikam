//! E232: where a small append's time goes in `ikam db serve`'s write path,
//! on a pack store on disk holding a database that already has many rows:
//! storing the event, editing the log, the ids view and the kv view, and the
//! commit (trees, commit object, ref move and its sync).
//!
//! `cargo run --release --example db_write_path -- DIR [ROWS] [APPENDS] [PER] [edit-index]`
//! (DIR is created and must not exist; ROWS preloaded in key order.)
use std::time::Instant;

use ikam_kernel::backends::{PackOptions, PackStore};
use ikam_kernel::{Event, Repo};

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

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let dir = a.first().expect("DIR").clone();
    let rows: usize = a.get(1).map_or(200_000, |s| s.parse().unwrap());
    let appends: usize = a.get(2).map_or(200, |s| s.parse().unwrap());
    let per: usize = a.get(3).map_or(1, |s| s.parse().unwrap());
    // `edit-index`: index only what each edit reads (E133's scope and
    // hints), not every stored object, before the first plan (E232).
    let edit_index = a.iter().any(|f| f == "edit-index");
    let opts = PackOptions {
        lazy_checkpoint: true,
        ref_log: false,
        filters: true,
    };
    let mut repo = Repo::new(PackStore::open_with(&dir, opts).unwrap());
    repo.plain_db_events = true;
    repo.object_templates = false;
    if edit_index {
        repo.set_seed_scope(Some(Vec::new()));
        repo.replace_hints = true;
    }
    let mut db = repo.db_empty().unwrap();
    let groups = 200;
    let per_group = rows.div_ceil(groups);
    let t = Instant::now();
    let mut batch = Vec::new();
    for s in 0..groups {
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
    eprintln!(
        "preloaded {} rows in {:.1} s",
        groups * per_group,
        t.elapsed().as_secs_f64()
    );
    drop(repo);
    // Reopen, as a restarted server does: the first append's cost.
    let t = Instant::now();
    let mut repo = Repo::new(PackStore::open_with(&dir, opts).unwrap());
    repo.plain_db_events = true;
    repo.object_templates = false;
    if edit_index {
        repo.set_seed_scope(Some(Vec::new()));
        repo.replace_hints = true;
    }
    let head = ikam_kernel::Store::get_ref(repo.store(), "db/bench")
        .unwrap()
        .unwrap();
    db = repo.db_at(&head).unwrap();
    println!("reopen and read the head: {:.1} ms", ms(t));

    // Small appends at the end of one group's range (new rows), as a
    // steady writer sends them.
    let mut steps = [Vec::new(), Vec::new(), Vec::new()];
    for i in 0..appends {
        let events: Vec<Event> = (0..per)
            .map(|j| {
                let (k, v) = row(7, per_group + i * per + j);
                Event::upsert(&k, &v, &format!("live:{i}:{j}"))
            })
            .collect();
        let t = Instant::now();
        let new = repo.db_append(&db, &events).unwrap();
        steps[0].push(ms(t));
        let t = Instant::now();
        repo.db_commit("db/bench", &new, "live").unwrap();
        steps[1].push(ms(t));
        db = new;
        steps[2].push(steps[0][i] + steps[1][i]);
        if i == 0 {
            println!("first append after reopen: {:.1} ms", steps[2][0]);
        }
    }
    println!(
        "store {:.1} MB",
        dir_bytes(std::path::Path::new(&dir)) as f64 / 1e6
    );
    for (name, xs) in [
        "append (event, log, ids, kv)",
        "commit (trees, ref, sync)",
        "total",
    ]
    .iter()
    .zip(steps.iter_mut())
    {
        if xs.is_empty() {
            continue;
        }
        xs.sort_by(f64::total_cmp);
        println!(
            "{name}: p50 {:.2} ms, p95 {:.2} ms",
            xs[xs.len() / 2],
            xs[xs.len() * 95 / 100]
        );
    }
}
