//! E279: a firing's cost against the tokens the net holds, with the
//! marking hashed as a whole (E269, the default) and as a sum kept up to
//! date (`net_init_sum`).
//!
//! Runs the loop net for N candidates with the owner never merging, so
//! finished candidates pile up in `waiting`, and reports the fire p50 for
//! each quarter of the run (by tokens held). Then checks, for the sum mode:
//! replay equal, the stored hash equal to the hash of the stored marking,
//! a fold from scratch equal, and fsck on an FsStore run.
//!
//!   cargo run --release --example net_sum_hash [-- N]
//!
//! N defaults to 2000 (8000 firings per mode). Nothing outside a temp
//! directory is written.

use std::time::Instant;

use ikam_kernel::net::{Net, sum_hash};
use ikam_kernel::{FsStore, MemStore, Repo, Store};

const LOOP: &str = "\
net loop 1
place cpu 1
place queued colored
place running colored
place measured colored
place waiting colored
transition propose agent propose: -> queued
transition start cmd bench: queued, cpu -> running
transition finish pure: running -> measured, cpu
transition ask pure: measured -> waiting
transition merge human: waiting ->
";

fn p50(v: &mut [u128]) -> f64 {
    v.sort_unstable();
    v[v.len() / 2] as f64 / 1000.0
}

/// Fire p50 (ms) per quarter of the run, and the whole run's seconds.
fn run<S: Store>(repo: &mut Repo<S>, net: &Net, n: usize, sum: bool) -> (Vec<f64>, f64) {
    let db = repo.db_empty().unwrap();
    let mut db = if sum {
        repo.net_init_sum(&db, net).unwrap()
    } else {
        repo.net_init(&db, net).unwrap()
    };
    let mut quarters: Vec<Vec<u128>> = vec![vec![]; 4];
    let all = Instant::now();
    for i in 0..n {
        let tok = format!("E{i:05}");
        for t in ["propose", "start", "finish", "ask"] {
            let start = Instant::now();
            db = repo
                .net_fire(&db, net, t, Some(&tok), &format!("{t}-{tok}"))
                .unwrap()
                .0;
            quarters[(i * 4 / n).min(3)].push(start.elapsed().as_micros());
        }
    }
    let secs = all.elapsed().as_secs_f64();
    let m = repo.net_marking(&db, "loop").unwrap();
    assert_eq!(m.get("waiting").map(|w| w.len()), Some(n));
    let start = Instant::now();
    assert_eq!(repo.net_replay(&db, net).unwrap(), m, "replay");
    let replay = start.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(repo.db_rebuild(&db.log).unwrap().kv, db.kv, "fold");
    if sum {
        let stored = repo.db_get(&db, "net/loop/hash").unwrap().unwrap();
        assert_eq!(
            std::str::from_utf8(&stored).unwrap(),
            sum_hash(&m).to_string(),
            "stored hash"
        );
    }
    println!("  replay of {} firings: {replay:.0} ms", 4 * n);
    (quarters.iter_mut().map(|q| p50(q)).collect(), secs)
}

fn main() {
    let n: usize = (std::env::args().nth(1))
        .and_then(|a| a.parse().ok())
        .unwrap_or(2000);
    let net = Net::parse(LOOP).unwrap();
    for (label, sum) in [("whole-marking hash (default)", false), ("sum hash", true)] {
        let mut mem = Repo::new(MemStore::default());
        println!("mem, {label}:");
        let (q, secs) = run(&mut mem, &net, n, sum);
        println!(
            "  fire p50 by quarter (tokens held up to {}): {:.2} / {:.2} / {:.2} / {:.2} ms; {secs:.1} s in all",
            n, q[0], q[1], q[2], q[3]
        );
        let dir = tempfile::tempdir().unwrap();
        let mut fs = Repo::new(FsStore::open(dir.path()).unwrap());
        println!("fs, {label}:");
        let (q, secs) = run(&mut fs, &net, n, sum);
        assert!(fs.fsck().unwrap().is_empty(), "fsck");
        println!(
            "  fire p50 by quarter: {:.2} / {:.2} / {:.2} / {:.2} ms; {secs:.1} s in all; fsck clean",
            q[0], q[1], q[2], q[3]
        );
    }
}
