//! E240: deleting many rows. `cargo run --release --example row_ids -- [N...]`
//!
//! Builds a database with three generated tables of N rows each (stable row
//! ids from `db_insert`), then removes one table four ways and a range of
//! half its row ids two ways. Prints one JSON line per case: milliseconds,
//! objects and bytes read from the store, log bytes added and new bytes
//! written. Every case's view root is checked against `db_rebuild`.

mod db_common;

use std::time::Instant;

use db_common::{Counting, Lcg, row};
use ikam_kernel::{Db, Event, Repo};

fn build(n: u64) -> (Repo<Counting>, Db, Vec<String>) {
    let mut repo = Repo::new(Counting::default());
    let mut db = repo.db_empty().unwrap();
    let mut rng = Lcg(42);
    let mut keys_b = Vec::new();
    for t in ["a", "b", "c"] {
        let mut start = 0;
        while start < n {
            let end = (start + 10_000).min(n);
            let rows: Vec<(Vec<u8>, String)> = (start..end)
                .map(|i| (row(&mut rng, i).into_bytes(), format!("{t}-{i}")))
                .collect();
            let refs: Vec<(&[u8], &str)> = rows
                .iter()
                .map(|(v, k)| (v.as_slice(), k.as_str()))
                .collect();
            let (next, keys) = repo.db_insert(&db, t, &refs).unwrap();
            if t == "b" {
                keys_b.extend(keys);
            }
            db = next;
            start = end;
        }
    }
    repo.store().take();
    (repo, db, keys_b)
}

type Base = (Repo<Counting>, Db, Vec<String>);

fn case(base: &Base, n: u64, name: &str, fast: bool, events: impl Fn(&[String]) -> Vec<Event>) {
    let (db, keys) = (base.1, &base.2);
    let mut repo = Repo::new(base.0.store().copy());
    repo.db_range_drop = fast;
    // A fresh handle loads its ledger and content index on its first
    // append; make one (and drop its result) before counting.
    repo.db_append(&db, &[Event::upsert("warm", b"up", "warm-up")])
        .unwrap();
    repo.store().take();
    let events = events(keys);
    let log_bytes: usize = events.iter().map(|e| e.encode().len()).sum();
    let written = repo.bytes_written;
    let t = Instant::now();
    let after = repo.db_append(&db, &events).unwrap();
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let io = repo.store().take();
    let new_bytes = repo.bytes_written - written;
    let left = repo.db_scan(&after).unwrap().len();
    assert_eq!(repo.db_rebuild(&after.log).unwrap(), after, "{name}");
    println!(
        "{{\"rows\":{n},\"case\":\"{name}\",\"range_drop\":{fast},\"events\":{},\"log_bytes\":{log_bytes},\"ms\":{ms:.1},\"reads\":{},\"read_bytes\":{},\"new_bytes\":{new_bytes},\"keys_left\":{left}}}",
        events.len(),
        io.reads,
        io.bytes
    );
}

fn main() {
    let sizes: Vec<u64> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let sizes = if sizes.is_empty() {
        vec![10_000, 100_000]
    } else {
        sizes
    };
    for n in sizes {
        let base = build(n);
        let per_row = |keys: &[String]| -> Vec<Event> {
            keys.iter()
                .map(|k| Event::remove(k, false, &format!("rm-{k}")))
                .collect()
        };
        let subtree = |_: &[String]| vec![Event::remove("b", true, "rm-b")];
        let half = |keys: &[String]| {
            let mid = &keys[keys.len() / 2];
            vec![Event::remove_range(&keys[0], mid, "rm-half")]
        };
        let per_row_half = |keys: &[String]| -> Vec<Event> {
            keys[..keys.len() / 2]
                .iter()
                .map(|k| Event::remove(k, false, &format!("rm-{k}")))
                .collect()
        };
        // Row-by-row deletes take minutes past 100,000 rows; skipped there.
        if n <= 100_000 {
            case(&base, n, "per_row_table", false, per_row);
            case(&base, n, "per_row_half", false, per_row_half);
        }
        case(&base, n, "subtree_table", false, subtree);
        case(&base, n, "subtree_table", true, subtree);
        case(&base, n, "range_half", false, half);
        case(&base, n, "range_half", true, half);
    }
}
