//! E231: where a database's bytes go, for grouped numeric rows (one JSON
//! array per group and sequence number, as `ikam db serve` stores them).
//! Writes the same rows three ways into fresh PackStores and prints the
//! bytes on disk next to the raw event bytes:
//!   batched: 2,000 events an append, a commit after each (a bulk load)
//!   one: every event in one append and one commit (no older versions)
//! `cargo run --release --example db_size_breakdown [GROUPS] [PER_GROUP] [plain] [notemplates] [shuffle] [edit-index]`
use std::path::Path;
use std::time::Instant;

use ikam_kernel::backends::{PackOptions, PackStore};
use ikam_kernel::{Event, Repo};

fn du(p: &Path) -> u64 {
    let mut n = 0;
    for e in std::fs::read_dir(p).unwrap() {
        let e = e.unwrap();
        let m = e.metadata().unwrap();
        n += if m.is_dir() { du(&e.path()) } else { m.len() };
    }
    n
}

fn rows(groups: usize, per_group: usize) -> Vec<Event> {
    let mut s = 0x9e3779b97f4a7c15u64;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        (s >> 11) as f64 / (1u64 << 53) as f64
    };
    let run = "0-1762912345678901234";
    let mut out = Vec::new();
    for g in 0..groups {
        let name = format!("G{g:04}");
        // A random walk per group, and three readings near it.
        let mut c = 20.0 + 400.0 * rnd();
        for i in 0..per_group {
            c *= 1.0 + (rnd() - 0.5) * 0.04;
            let (a, b, d) = (c * (1.0 + (rnd() - 0.5) * 0.01), c * 1.01, c * 0.99);
            let n = (1e5 + 1e7 * rnd()).round();
            let value = format!(
                "[\"{name}\",{i},{a:?},{b:?},{d:?},{c:?},{n:?},\"bench\",\"2026-01-05T21:00:00+00:00\"]"
            );
            let key = format!("t/rows/{name}/{i:06}");
            out.push(Event::upsert(
                &key,
                value.as_bytes(),
                &format!("load:{run}:{key}"),
            ));
        }
    }
    out
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let groups = a.get(1).map_or(300, |s| s.parse().unwrap());
    let per_group = a.get(2).map_or(1200, |s| s.parse().unwrap());
    let plain = a.iter().any(|f| f == "plain");
    let notemplates = a.iter().any(|f| f == "notemplates");
    let mut events = rows(groups, per_group);
    if a.iter().any(|f| f == "shuffle") {
        // Rows arriving out of key order.
        let mut s = 0x2545f4914f6cdd1du64;
        for i in (1..events.len()).rev() {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            events.swap(i, (s % (i as u64 + 1)) as usize);
        }
    }
    let raw: usize = events.iter().map(|e| e.encode().len()).sum();
    let values: usize = events
        .iter()
        .map(|e| match &e.op {
            ikam_kernel::Op::Upsert(v) => v.len(),
            _ => 0,
        })
        .sum();
    let keys: usize = events.iter().map(|e| e.key.len()).sum();
    let idem: usize = events.iter().map(|e| e.idempotency_key.len()).sum();
    println!(
        "{} rows: values {:.1} MB, keys {:.1} MB, idempotency keys {:.1} MB, encoded events {:.1} MB",
        events.len(),
        values as f64 / 1e6,
        keys as f64 / 1e6,
        idem as f64 / 1e6,
        raw as f64 / 1e6
    );
    let opts = PackOptions {
        lazy_checkpoint: true,
        filters: true,
        ref_log: false,
    };
    for (label, per) in [("batched", 2000usize), ("one", usize::MAX)] {
        let dir = std::env::temp_dir().join(format!("ikam-e231-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut repo = Repo::new(PackStore::open_with(&dir, opts).unwrap());
        repo.plain_db_events = plain;
        repo.object_templates = !notemplates;
        if a.iter().any(|f| f == "edit-index") {
            repo.set_seed_scope(Some(Vec::new()));
            repo.replace_hints = true;
        }
        let mut db = repo.db_empty().unwrap();
        let t = Instant::now();
        let mut commits = 0;
        for chunk in events.chunks(per.min(events.len())) {
            db = repo.db_append(&db, chunk).unwrap();
            repo.db_commit("db/rows", &db, "batch").unwrap();
            commits += 1;
        }
        let secs = t.elapsed().as_secs_f64();
        drop(repo);
        let bytes = du(&dir);
        println!(
            "{label:>8}: {commits} commits, {secs:.1} s, {:.1} MB on disk ({:.0} bytes a row, {:.2}x the encoded events)",
            bytes as f64 / 1e6,
            bytes as f64 / events.len() as f64,
            bytes as f64 / raw as f64
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
