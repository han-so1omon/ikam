//! E293: where one firing's time goes in a commit-per-firing runner, and
//! what `read_weight` and the sum hash (E279) do to it.
//!
//! One writer on a MemStore takes N candidates through start and finish of
//! the loop net (E292's), committing after every firing, and times each
//! step: read the head (`resolve`, `db_at`), read the marking
//! (`net_marking`, as a done check does), find the next firing (a start
//! from `net_enabled_limit`, or a running token), fire, and commit. Also a
//! thousand `db_get`s of one key, and the bytes written per firing.
//!
//!   cargo run --release --example net_firing_cost [-- N...]
//!   CACHE=1 cargo run --release --example net_firing_cost (E294's rows)

use std::time::{Duration, Instant};

use ikam_kernel::net::Net;
use ikam_kernel::{MemStore, Repo};

const NET: &str = "\
net loop 1
place cpu 2
place queued colored
place running colored
place measured colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu
";

struct Row {
    get_us: f64,
    per_ms: f64,
    steps: [f64; 5],
    bytes: f64,
}

fn run(n: usize, read_weight: f64, sum: bool, cache: usize) -> Row {
    let net = Net::parse(NET).unwrap();
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = read_weight;
    repo.read_cache = cache;
    let db = repo.db_empty().unwrap();
    let mut db = if sum {
        repo.net_init_sum(&db, &net).unwrap()
    } else {
        repo.net_init(&db, &net).unwrap()
    };
    for i in 0..n {
        db = repo
            .net_fire(
                &db,
                &net,
                "propose",
                Some(&format!("c{i}")),
                &format!("p{i}"),
            )
            .unwrap()
            .0;
    }
    repo.db_commit("loop", &db, "proposed").unwrap();
    let s0 = Instant::now();
    for _ in 0..1000 {
        repo.db_get(&db, "net/loop/m/cpu/_").unwrap();
    }
    let get_us = s0.elapsed().as_secs_f64() * 1e3;
    let mut t = [Duration::ZERO; 5];
    let lap = |t: &mut [Duration; 5], i: usize, s: &mut Instant| {
        t[i] += s.elapsed();
        *s = Instant::now();
    };
    let written = repo.bytes_written;
    let steps = 2 * n;
    for step in 0..steps {
        let mut s = Instant::now();
        let head = repo.resolve("loop").unwrap();
        let db = repo.db_at(&head).unwrap();
        lap(&mut t, 0, &mut s);
        let m = repo.net_marking(&db, "loop").unwrap();
        lap(&mut t, 1, &mut s);
        let (tr, tok) = if step % 2 == 0 {
            let e = repo.net_enabled_limit(&db, &net, true, 64).unwrap();
            e.into_iter().find(|(t, _)| t == "start").unwrap()
        } else {
            ("finish".to_string(), m["running"].keys().next().cloned())
        };
        lap(&mut t, 2, &mut s);
        let (db, _) = repo
            .net_fire(&db, &net, &tr, tok.as_deref(), &format!("k{step}"))
            .unwrap();
        lap(&mut t, 3, &mut s);
        repo.db_commit("loop", &db, "step").unwrap();
        lap(&mut t, 4, &mut s);
    }
    let ms = |d: Duration| d.as_secs_f64() * 1e3 / steps as f64;
    Row {
        get_us,
        per_ms: t.iter().map(|&d| ms(d)).sum(),
        steps: t.map(ms),
        bytes: (repo.bytes_written - written) as f64 / steps as f64,
    }
}

/// (read_weight, sum hash, read_cache bytes). E293's rows by default;
/// CACHE=1 runs E294's: the read cache (64 MB) against the same options.
fn configs() -> Vec<(f64, bool, usize)> {
    if std::env::var("CACHE").as_deref() == Ok("1") {
        let c = 64 << 20;
        return vec![
            (0.001, false, 0),
            (0.001, false, c),
            (0.001, true, 0),
            (0.001, true, c),
            (1.0, true, 0),
            (1.0, true, c),
        ];
    }
    vec![
        (0.001, false, 0),
        (0.01, false, 0),
        (0.1, false, 0),
        (1.0, false, 0),
        (10.0, false, 0),
        (0.001, true, 0),
        (1.0, true, 0),
    ]
}

fn main() {
    let ns: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    let ns = if ns.is_empty() { vec![200, 1000] } else { ns };
    println!(
        "{:>5} {:>6} {:<7} {:>8} {:>9} {:>7} {:>8} {:>7} {:>7} {:>7} {:>9}",
        "N",
        "weight",
        "hash",
        "get us",
        "firing ms",
        "head",
        "marking",
        "next",
        "fire",
        "commit",
        "bytes"
    );
    for &n in &ns {
        for (w, sum, cache) in configs() {
            let r = run(n, w, sum, cache);
            println!(
                "{:>5} {:>6} {:<7} {:>8.1} {:>9.1} {:>7.2} {:>8.2} {:>7.2} {:>7.2} {:>7.2} {:>9.0}",
                n,
                w,
                match (sum, cache > 0) {
                    (false, false) => "plain",
                    (true, false) => "sum",
                    (false, true) => "plain+c",
                    (true, true) => "sum+c",
                },
                r.get_us,
                r.per_ms,
                r.steps[0],
                r.steps[1],
                r.steps[2],
                r.steps[3],
                r.steps[4],
                r.bytes
            );
        }
    }
}
