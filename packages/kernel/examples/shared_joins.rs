//! E243: two joins over one large table, kept up to date three ways.
//! `cargo run --release --example shared_joins -- [N] [G] [BATCH...]`
//!
//! Table `a` has N generated rows, each in one of G groups; tables `b` and
//! `c` have one row per group. Two joins, a with b and a with c, both on the
//! group. Batches of changes (80% to `a`, 10% each to `b` and `c`; a change
//! to `b` or `c` reaches every `a` row in its group) are appended, then
//! the joins are brought up to date: incrementally with one shared
//! arrangement of `a`, incrementally with one arrangement of `a` per join,
//! and by recomputing both joins. After every batch the incremental joins
//! must equal the recomputed rows. Prints median milliseconds and store
//! reads per update, and the arrangements' stored bytes.

mod db_common;

use std::time::Instant;

use db_common::{Counting, Lcg};
use ikam_kernel::join::{Arrangement, Join};
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
    format!(
        "g{:05},{},{}",
        rng.next() % g,
        rng.next() % 10_000,
        ["ok", "warn"][rng.next() as usize % 2]
    )
    .into_bytes()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let args: Vec<u64> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let (n, g) = (
        args.first().copied().unwrap_or(100_000),
        args.get(1).copied().unwrap_or(1000),
    );
    let batches = if args.len() > 2 {
        args[2..].to_vec()
    } else {
        vec![10, 100, 1000]
    };
    let mut rng = Lcg(42);
    let mut repo = Repo::new(Counting::default());
    let mut db = repo.db_empty().unwrap();
    let mut events = Vec::new();
    for i in 0..n {
        events.push(Event::upsert(
            &format!("a/{i:08}"),
            &a_row(&mut rng, g),
            &format!("a{i}"),
        ));
    }
    for i in 0..g {
        events.push(Event::upsert(
            &format!("b/g{i:05}"),
            format!("name{i},{}", i % 7).as_bytes(),
            &format!("b{i}"),
        ));
        events.push(Event::upsert(
            &format!("c/g{i:05}"),
            format!("target{}", i % 13).as_bytes(),
            &format!("c{i}"),
        ));
    }
    for chunk in events.chunks(10_000) {
        db = repo.db_append(&db, chunk).unwrap();
    }
    let arr = |name: &str, table: &str, key| Arrangement {
        name: name.into(),
        table: table.into(),
        key,
    };
    let shared = vec![
        arr("a", "a", group as fn(&str, &[u8]) -> Option<String>),
        arr("b", "b", own_key),
        arr("c", "c", own_key),
    ];
    let apart = vec![
        arr("a1", "a", group),
        arr("a2", "a", group),
        arr("b", "b", own_key),
        arr("c", "c", own_key),
    ];
    let join = |name: &str, l: &str, r: &str| Join {
        name: name.into(),
        left: l.into(),
        right: r.into(),
        combine: pair,
    };
    let j_shared = vec![join("ab", "a", "b"), join("ac", "a", "c")];
    let j_apart = vec![join("ab", "a1", "b"), join("ac", "a2", "c")];
    // Every batch, made up front so each way applies the same ones.
    let mut idem = 0;
    let rounds: Vec<(u64, Vec<Event>)> = batches
        .iter()
        .flat_map(|&size| std::iter::repeat_n(size, 5))
        .map(|size| {
            let batch = (0..size)
                .map(|_| {
                    idem += 1;
                    let id = format!("x{idem}");
                    match rng.next() % 10 {
                        0 => Event::upsert(
                            &format!("b/g{:05}", rng.next() % g),
                            format!("renamed{idem},1").as_bytes(),
                            &id,
                        ),
                        1 => Event::upsert(
                            &format!("c/g{:05}", rng.next() % g),
                            format!("target{idem}").as_bytes(),
                            &id,
                        ),
                        2 => Event::remove(&format!("a/{:08}", rng.next() % n), false, &id),
                        _ => Event::upsert(
                            &format!("a/{:08}", rng.next() % n),
                            &a_row(&mut rng, g),
                            &id,
                        ),
                    }
                })
                .collect();
            (size, batch)
        })
        .collect();
    let templates = std::env::var("TEMPLATES").is_ok_and(|v| v == "1");
    // Each way on its own copy of the store, so none reuses another's writes.
    let mut finals = Vec::new();
    for way in ["shared_arrangement", "arrangement_per_join", "recompute"] {
        let mut r = Repo::new(repo.store().copy());
        r.object_templates = templates;
        let (arrs, joins) = match way {
            "arrangement_per_join" => (&apart, &j_apart),
            _ => (&shared, &j_shared),
        };
        let t = Instant::now();
        let mut state = r.joins_build(arrs, joins, &db).unwrap();
        let build_ms = t.elapsed().as_secs_f64() * 1e3;
        if way != "recompute" {
            println!(
                "{{\"rows\":{n},\"groups\":{g},\"way\":\"{way}\",\"object_templates\":{templates},\"build_ms\":{build_ms:.1},\"join_rows\":{}}}",
                r.join_rows(&state, "ab").unwrap().len()
            );
        }
        let mut d = db;
        let mut per: std::collections::BTreeMap<u64, (Vec<f64>, Vec<f64>)> = Default::default();
        let mut last = (Default::default(), Default::default());
        for (size, batch) in &rounds {
            d = r.db_append(&d, batch).unwrap();
            r.store().take();
            let t = Instant::now();
            if way == "recompute" {
                last = (
                    r.join_recompute(arrs, &joins[0], &d).unwrap().1,
                    r.join_recompute(arrs, &joins[1], &d).unwrap().1,
                );
            } else {
                state = r.joins_update(arrs, joins, &state, &d).unwrap();
            }
            let e = per.entry(*size).or_default();
            e.0.push(t.elapsed().as_secs_f64() * 1e3);
            e.1.push(r.store().take().reads as f64);
            if way != "recompute" {
                last = (
                    r.join_rows(&state, "ab").unwrap(),
                    r.join_rows(&state, "ac").unwrap(),
                );
            }
        }
        for (size, (ms, reads)) in per {
            println!(
                "{{\"rows\":{n},\"batch\":{size},\"way\":\"{way}\",\"object_templates\":{templates},\"update_ms\":{:.1},\"update_reads\":{}}}",
                median(ms),
                median(reads)
            );
        }
        finals.push(last);
    }
    assert!(
        finals.windows(2).all(|w| w[0] == w[1]),
        "every way ends with the same join rows"
    );
    // A join kept per way is checked against a recompute at the end; the
    // test suite checks every batch (tests/join.rs).
    let mut r = Repo::new(repo.store().copy());
    let s_shared = r.joins_build(&shared, &j_shared, &db).unwrap();
    let s_apart = r.joins_build(&apart, &j_apart, &db).unwrap();
    // Identical arrangements are one graph in a content-addressed store, so
    // keeping one per join costs update work, not bytes.
    let same = s_apart.arranged["a1"] == s_shared.arranged["a"]
        && s_apart.arranged["a2"] == s_shared.arranged["a"];
    println!("{{\"rows\":{n},\"per_join_arrangements_are_the_shared_graph\":{same}}}");
}
