//! E280: the scheduler's question ("what can run next?") against the
//! tokens the net holds: `net_enabled` (reads the whole marking) and
//! `net_enabled_limit(.., 1)` (reads only what the first answer needs).
//!
//! Builds up finished candidates waiting on the owner (sum-hash net, E279,
//! MemStore), and at each level of tokens held times both calls with one
//! candidate queued, then a runner step (ask for one enabled firing, fire
//! it) with each. Both calls' first answers are checked equal.
//!
//!   cargo run --release --example net_pick [-- MAX]
//!
//! MAX (tokens held at the last level) defaults to 4000.

use std::time::Instant;

use ikam_kernel::net::Net;
use ikam_kernel::{Db, MemStore, Repo};

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

fn p50(mut v: Vec<u128>) -> f64 {
    v.sort_unstable();
    v[v.len() / 2] as f64 / 1000.0
}

fn fire(r: &mut Repo<MemStore>, db: &Db, net: &Net, t: &str, tok: &str) -> Db {
    r.net_fire(db, net, t, Some(tok), &format!("{t}-{tok}"))
        .unwrap()
        .0
}

fn main() {
    let max: usize = (std::env::args().nth(1))
        .and_then(|a| a.parse().ok())
        .unwrap_or(4000);
    let net = Net::parse(LOOP).unwrap();
    let mut r = Repo::new(MemStore::default());
    let db = r.db_empty().unwrap();
    let mut db = r.net_init_sum(&db, &net).unwrap();
    let mut held = 0;
    let mut next = 0;
    let levels: Vec<usize> = [0, 250, 500, 1000, 2000, 4000, 8000]
        .into_iter()
        .filter(|l| *l <= max)
        .collect();
    println!("tokens held | net_enabled | limit 1 | runner step, enabled | runner step, limit 1");
    for level in levels {
        while held < level {
            let tok = format!("E{next:05}");
            next += 1;
            for t in ["propose", "start", "finish", "ask"] {
                db = fire(&mut r, &db, &net, t, &tok);
            }
            held += 1;
        }
        let tok = format!("E{next:05}");
        next += 1;
        db = fire(&mut r, &db, &net, "propose", &tok);
        let (mut full, mut one) = (vec![], vec![]);
        for _ in 0..30 {
            let s = Instant::now();
            let a = r.net_enabled(&db, &net, true).unwrap();
            full.push(s.elapsed().as_micros());
            let s = Instant::now();
            let b = r.net_enabled_limit(&db, &net, true, 1).unwrap();
            one.push(s.elapsed().as_micros());
            assert_eq!(a[..1], b[..], "first answer");
        }
        // A runner step: ask, then fire what came first (start, then
        // finish, then ask for this candidate), timed with each call.
        let mut steps = [vec![], vec![]];
        for (i, which) in steps.iter_mut().enumerate() {
            let mut d = db;
            for _ in 0..3 {
                let s = Instant::now();
                let pick = if i == 0 {
                    r.net_enabled(&d, &net, true).unwrap().remove(0)
                } else {
                    r.net_enabled_limit(&d, &net, true, 1).unwrap().remove(0)
                };
                let (t, tk) = (pick.0, pick.1.unwrap());
                d = r
                    .net_fire(&d, &net, &t, Some(&tk), &format!("{t}-{tk}-{i}"))
                    .unwrap()
                    .0;
                which.push(s.elapsed().as_micros());
            }
            if i == 1 {
                db = d;
                held += 1;
            }
        }
        let [a, b] = steps;
        println!(
            "{level:>11} | {:>8.2} ms | {:>5.2} ms | {:>17.2} ms | {:>17.2} ms",
            p50(full),
            p50(one),
            p50(a),
            p50(b)
        );
    }
    let m = r.net_marking(&db, "loop").unwrap();
    assert_eq!(r.net_replay(&db, &net).unwrap(), m, "replay");
    println!("replay equal; {held} waiting");
}
