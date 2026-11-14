//! E245: where a join update's time goes at large batches, and a switch to
//! recomputing above a share of touched join keys.
//! `cargo run --release --example join_switch -- N G SHARE[,SHARE...] BATCH...`
//!
//! The tables and changes of E243 (`shared_joins`): table `a` has N rows in
//! G groups, `b` and `c` one row per group, joins `a` with `b` and `a` with
//! `c`; a batch is 80% changes to `a` and 10% each to `b` and `c`. Ways:
//! `update` and `recompute` (E243), each E245 option alone on an update
//! (`plain`, `memo`), both (`update_fast`), the switch (`switch`: both plus
//! `recompute_above: SHARE`, one way per share), and a recompute with them
//! (`recompute_fast`).
//! Each way runs on its own copy of the store; its stored bytes after the
//! batches are printed with its times. Joins are read back with
//! `read_once` (E243's harness read them row by row, which decodes the
//! join's one value content once per row and stalled at 100,000 rows).
//! Every way must end with the same rows. Prints median milliseconds and
//! reads per batch size (5 batches each). Synthetic data only.

mod db_common;

use std::collections::BTreeMap;
use std::time::Instant;

use db_common::{Counting, Lcg};
use ikam_kernel::join::{Arrangement, Join, JoinOpts};
use ikam_kernel::{Event, Repo};

fn group(_: &str, v: &[u8]) -> Option<String> {
    String::from_utf8_lossy(v)
        .split(',')
        .next()
        .map(str::to_string)
}

fn own_key(k: &str, _: &[u8]) -> Option<String> {
    k.rsplit('/').next().map(str::to_string)
}

fn pair(l: &[u8], r: &[u8]) -> Vec<u8> {
    [l, b"|", r].concat()
}

fn a_row(rng: &mut Lcg, g: u64) -> Vec<u8> {
    let s = ["ok", "warn"][rng.next() as usize % 2];
    format!("g{:05},{},{s}", rng.next() % g, rng.next() % 10_000).into_bytes()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn change(rng: &mut Lcg, n: u64, g: u64, id: &str) -> Event {
    let (b, c) = (
        format!("b/g{:05}", rng.next() % g),
        format!("c/g{:05}", rng.next() % g),
    );
    match rng.next() % 10 {
        0 => Event::upsert(&b, format!("renamed{id},1").as_bytes(), id),
        1 => Event::upsert(&c, format!("target{id}").as_bytes(), id),
        2 => Event::remove(&format!("a/{:08}", rng.next() % n), false, id),
        _ => Event::upsert(&format!("a/{:08}", rng.next() % n), &a_row(rng, g), id),
    }
}

/// Table `a` of `n` rows in `g` groups and tables `b`, `c` of one row per group.
fn tables(repo: &mut Repo<Counting>, rng: &mut Lcg, n: u64, g: u64) -> ikam_kernel::Db {
    let mut events: Vec<Event> = (0..n)
        .map(|i| Event::upsert(&format!("a/{i:08}"), &a_row(rng, g), &format!("a{i}")))
        .collect();
    for i in 0..g {
        let (b, c) = (format!("name{i},{}", i % 7), format!("target{}", i % 13));
        events.push(Event::upsert(
            &format!("b/g{i:05}"),
            b.as_bytes(),
            &format!("b{i}"),
        ));
        events.push(Event::upsert(
            &format!("c/g{i:05}"),
            c.as_bytes(),
            &format!("c{i}"),
        ));
    }
    let mut db = repo.db_empty().unwrap();
    for chunk in events.chunks(10_000) {
        db = repo.db_append(&db, chunk).unwrap();
    }
    db
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (n, g): (u64, u64) = (args[0].parse().unwrap(), args[1].parse().unwrap());
    let shares: Vec<f64> = args[2].split(',').map(|a| a.parse().unwrap()).collect();
    let batches: Vec<u64> = args[3..].iter().map(|a| a.parse().unwrap()).collect();
    let mut rng = Lcg(42);
    let mut repo = Repo::new(Counting::default());
    let db = tables(&mut repo, &mut rng, n, g);
    let arr = |name: &str, key| Arrangement {
        name: name.into(),
        table: name.into(),
        key,
    };
    let key: fn(&str, &[u8]) -> Option<String> = group;
    let arrs = vec![arr("a", key), arr("b", own_key), arr("c", own_key)];
    let join = |name: &str, r: &str| Join {
        name: name.into(),
        left: "a".into(),
        right: r.into(),
        combine: pair,
    };
    let joins = vec![join("ab", "b"), join("ac", "c")];
    let mut idem = 0;
    let rounds: Vec<(u64, Vec<Event>)> = batches
        .iter()
        .flat_map(|&size| std::iter::repeat_n(size, 5))
        .map(|size| {
            let batch = (0..size)
                .map(|_| {
                    idem += 1;
                    change(&mut rng, n, g, &format!("x{idem}"))
                })
                .collect();
            (size, batch)
        })
        .collect();
    let once = JoinOpts {
        read_once: true,
        ..Default::default()
    };
    let mut finals = Vec::new();
    let fast = JoinOpts {
        read_once: true,
        plain_writes: true,
        memo: true,
        ..Default::default()
    };
    let mut ways: Vec<(&str, f64)> = [
        "update",
        "plain",
        "memo",
        "update_fast",
        "update_fast_pruned",
    ]
    .map(|w| (w, 0.0))
    .to_vec();
    ways.extend(shares.iter().map(|&s| ("switch", s)));
    ways.extend([("recompute", 0.0), ("recompute_fast", 0.0)]);
    for (way, share) in ways {
        let mut r = Repo::new(repo.store().copy());
        r.object_templates = false;
        r.prune_diff = way.ends_with("pruned"); // E249
        let opts = match way {
            "plain" => JoinOpts {
                plain_writes: true,
                ..Default::default()
            },
            "memo" => JoinOpts {
                memo: true,
                ..Default::default()
            },
            "update_fast" | "update_fast_pruned" | "recompute_fast" => fast,
            "switch" => JoinOpts {
                recompute_above: Some(share),
                ..fast
            },
            _ => JoinOpts::default(),
        };
        let before = r.store().stored();
        let mut state = r.joins_build(&arrs, &joins, &db).unwrap();
        let (mut d, mut per) = (db, BTreeMap::<u64, (Vec<f64>, Vec<f64>)>::new());
        for (size, batch) in &rounds {
            d = r.db_append(&d, batch).unwrap();
            r.store().take();
            let t = Instant::now();
            if !way.starts_with("recompute") {
                state = r
                    .joins_update_with(&arrs, &joins, &state, &d, &opts)
                    .unwrap();
            } else {
                for j in &joins {
                    r.join_recompute_with(&arrs, j, &d, &opts).unwrap();
                }
            }
            let e = per.entry(*size).or_default();
            e.0.push(t.elapsed().as_secs_f64() * 1e3);
            e.1.push(r.store().take().reads as f64);
        }
        let grown = r.store().stored() - before;
        for (size, (ms, reads)) in per {
            let (ms, reads) = (median(ms), median(reads));
            println!(
                "{{\"rows\":{n},\"groups\":{g},\"batch\":{size},\"way\":\"{way}\",\"share\":{share},\"update_ms\":{ms:.1},\"update_reads\":{reads},\"stored_bytes_added\":{grown}}}"
            );
        }
        let rows: Vec<_> = match way {
            w if !w.starts_with("recompute") => joins
                .iter()
                .map(|j| r.join_rows_with(&state, &j.name, &once).unwrap())
                .collect(),
            _ => joins
                .iter()
                .map(|j| r.join_recompute_with(&arrs, j, &d, &once).unwrap().1)
                .collect(),
        };
        finals.push(rows);
    }
    assert!(
        finals.windows(2).all(|w| w[0] == w[1]),
        "every way ends with the same rows"
    );
    println!("{{\"rows\":{n},\"same_rows_every_way\":true}}");
}
