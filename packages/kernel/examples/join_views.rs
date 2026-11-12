//! E246: k views over the join of table `a` with table `b` (E243's tables
//! and changes), kept three ways: `apart` (each view its own arrangements
//! and join), `shared_arrangements` (one arrangement per table, a join per
//! view), and `shared_join` (one join that every view reads). Views in the
//! first two differ in their combine function, so each is its own result;
//! a shared join serves views that read the same rows.
//! `cargo run --release --example join_views -- N G BATCH...`
//! Prints median milliseconds and reads per update for k = 1, 2, 4, 8,
//! with E243's defaults and with E245's `plain_writes` and `memo`.
//! Synthetic data only.

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

/// View `I`'s combine: the pair, tagged with the view.
fn tag<const I: u8>(l: &[u8], r: &[u8]) -> Vec<u8> {
    [l, b"|", r, &[b'0' + I]].concat()
}

const TAGS: [fn(&[u8], &[u8]) -> Vec<u8>; 8] = [
    tag::<0>, tag::<1>, tag::<2>, tag::<3>, tag::<4>, tag::<5>, tag::<6>, tag::<7>,
];

fn a_row(rng: &mut Lcg, g: u64) -> Vec<u8> {
    let s = ["ok", "warn"][rng.next() as usize % 2];
    format!("g{:05},{},{s}", rng.next() % g, rng.next() % 10_000).into_bytes()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

/// E243's batch mix without table `c`: 90% changes to `a`, 10% to `b`.
fn change(rng: &mut Lcg, n: u64, g: u64, id: &str) -> Event {
    match rng.next() % 10 {
        0 => Event::upsert(
            &format!("b/g{:05}", rng.next() % g),
            format!("renamed{id},1").as_bytes(),
            id,
        ),
        1 => Event::remove(&format!("a/{:08}", rng.next() % n), false, id),
        _ => Event::upsert(&format!("a/{:08}", rng.next() % n), &a_row(rng, g), id),
    }
}

/// Arrangements and joins for `k` views kept `way`.
fn plan(way: &str, k: usize) -> (Vec<Arrangement>, Vec<Join>) {
    let key: fn(&str, &[u8]) -> Option<String> = group;
    let arr = |name: String, table: &str, key| Arrangement {
        name,
        table: table.into(),
        key,
    };
    let join = |i: usize, l: String, r: String| Join {
        name: format!("v{i}"),
        left: l,
        right: r,
        combine: TAGS[i],
    };
    match way {
        "apart" => (
            (0..k)
                .flat_map(|i| {
                    [
                        arr(format!("a{i}"), "a", key),
                        arr(format!("b{i}"), "b", own_key),
                    ]
                })
                .collect(),
            (0..k)
                .map(|i| join(i, format!("a{i}"), format!("b{i}")))
                .collect(),
        ),
        "shared_arrangements" => (
            vec![arr("a".into(), "a", key), arr("b".into(), "b", own_key)],
            (0..k).map(|i| join(i, "a".into(), "b".into())).collect(),
        ),
        _ => (
            vec![arr("a".into(), "a", key), arr("b".into(), "b", own_key)],
            vec![join(0, "a".into(), "b".into())],
        ),
    }
}

fn main() {
    let args: Vec<u64> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let (n, g, batches) = (args[0], args[1], args[2..].to_vec());
    let mut rng = Lcg(42);
    let mut repo = Repo::new(Counting::default());
    let mut events: Vec<Event> = (0..n)
        .map(|i| Event::upsert(&format!("a/{i:08}"), &a_row(&mut rng, g), &format!("a{i}")))
        .collect();
    for i in 0..g {
        events.push(Event::upsert(
            &format!("b/g{i:05}"),
            format!("name{i},{}", i % 7).as_bytes(),
            &format!("b{i}"),
        ));
    }
    let mut db = repo.db_empty().unwrap();
    for chunk in events.chunks(10_000) {
        db = repo.db_append(&db, chunk).unwrap();
    }
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
    let fast = JoinOpts {
        read_once: true,
        plain_writes: true,
        memo: true,
        ..Default::default()
    };
    for (opts_name, opts) in [("defaults", JoinOpts::default()), ("plain_memo", fast)] {
        for way in ["apart", "shared_arrangements", "shared_join"] {
            for k in [1, 2, 4, 8] {
                let (arrs, joins) = plan(way, k);
                let mut r = Repo::new(repo.store().copy());
                r.object_templates = false;
                let mut state = r.joins_build(&arrs, &joins, &db).unwrap();
                let (mut d, mut per) = (db, BTreeMap::<u64, (Vec<f64>, Vec<f64>)>::new());
                for (size, batch) in &rounds {
                    d = r.db_append(&d, batch).unwrap();
                    r.store().take();
                    let t = Instant::now();
                    state = r
                        .joins_update_with(&arrs, &joins, &state, &d, &opts)
                        .unwrap();
                    let e = per.entry(*size).or_default();
                    e.0.push(t.elapsed().as_secs_f64() * 1e3);
                    e.1.push(r.store().take().reads as f64);
                }
                // Each view's rows equal its join recomputed now.
                for j in &joins {
                    let want = r.join_recompute_with(&arrs, j, &d, &fast).unwrap().1;
                    assert_eq!(
                        r.join_rows_with(&state, &j.name, &fast).unwrap(),
                        want,
                        "{way} {k} {}",
                        j.name
                    );
                }
                for (size, (ms, reads)) in per {
                    let (ms, reads) = (median(ms), median(reads));
                    println!(
                        "{{\"rows\":{n},\"opts\":\"{opts_name}\",\"way\":\"{way}\",\"views\":{k},\"batch\":{size},\"update_ms\":{ms:.1},\"update_reads\":{reads}}}"
                    );
                }
            }
        }
    }
}
