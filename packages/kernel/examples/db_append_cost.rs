//! E229: where an append of a batch of row events spends its time, by step:
//! storing events, editing the log, the ids view and the kv view.
use std::time::Instant;

use ikam_kernel::{Event, MemStore, Repo};

fn main() {
    let batches: usize = std::env::args().nth(1).map_or(10, |s| s.parse().unwrap());
    let per: usize = std::env::args().nth(2).map_or(2000, |s| s.parse().unwrap());
    let mut repo = Repo::new(MemStore::default());
    let flags: Vec<String> = std::env::args().skip(3).collect();
    repo.plain_db_events = flags.iter().any(|f| f == "plain");
    repo.object_templates = !flags.iter().any(|f| f == "notemplates");
    let mut db = repo.db_empty().unwrap();
    let mut s = 0x9e3779b97f4a7c15u64;
    let value = b"[1.2345,2.5,\"src\",\"2026-01-05T00:00:00+00:00\",1.2345,2.5,\"src\"]";
    let t = Instant::now();
    for b in 0..batches {
        let events: Vec<Event> = (0..per)
            .map(|i| {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                Event::upsert(
                    &format!("t/rows/G{:04}/{b:03}-{i:05}", s % 100),
                    value,
                    &format!("load:1:t/rows/G{:04}/{b:03}-{i:05}", s % 100),
                )
            })
            .collect();
        let t = Instant::now();
        db = repo.db_append(&db, &events).unwrap();
        println!("batch {b}: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
    }
    println!(
        "{} events in {:.2} s ({:.1} us/event)",
        batches * per,
        t.elapsed().as_secs_f64(),
        t.elapsed().as_secs_f64() * 1e6 / (batches * per) as f64
    );
    println!("stored {:.1} MB", repo.bytes_written as f64 / 1e6);
}
