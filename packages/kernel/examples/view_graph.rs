//! E242: keeping a graph of derived views current, three ways.
//! `cargo run --release --example view_graph -- [N] [BATCH...]`
//!
//! A generated table of N rows (a time, an id, a two-place reading, a
//! count, a status word) and three views: counts per id over the table,
//! the flagged rows, and counts per id over the flagged rows (a view of a
//! view). Batches of changes (70% rewrites, 20% new rows, 10% removes) are
//! appended, and the views are brought up to date incrementally and by
//! rebuilding. After every batch both states must equal a full build.
//! Prints one JSON line per batch size and way: median milliseconds and
//! store reads an update takes, and what reading the per-id view costs
//! when it is stored and when it is computed on read.

mod db_common;

use std::time::Instant;

use db_common::{Counting, Lcg, row};
use ikam_kernel::views::{Mode, View, ViewGraph, ViewOp};
use ikam_kernel::{Event, Repo};

fn field(v: &[u8], j: usize) -> &str {
    std::str::from_utf8(v).unwrap().split(',').nth(j).unwrap()
}

fn per_id(_: &str, v: &[u8]) -> Option<(String, i64)> {
    Some((field(v, 1).to_string(), field(v, 3).parse().ok()?))
}

fn flagged(k: &str, v: &[u8]) -> Option<(String, Vec<u8>)> {
    (field(v, 4) == "warn").then(|| {
        (
            format!("flagged/{k}"),
            format!("{},{}", field(v, 1), field(v, 3)).into_bytes(),
        )
    })
}

fn flagged_per_id(_: &str, v: &[u8]) -> Option<(String, i64)> {
    Some((field(v, 0).to_string(), field(v, 1).parse().ok()?))
}

fn count_ok(_: &str, v: &[u8]) -> Result<(), String> {
    let n: i64 = field(v, 1).parse().map_err(|_| "count")?;
    if n < 0 {
        Err("negative".into())
    } else {
        Ok(())
    }
}

fn graph(mode: Mode) -> ViewGraph {
    let v = |name: &str, input: &str, op, check| View {
        name: name.into(),
        input: input.into(),
        mode,
        op,
        check,
    };
    ViewGraph::new(vec![
        v("per_id", "kv", ViewOp::GroupSum(per_id), None),
        v("flagged", "kv", ViewOp::Map(flagged), Some(count_ok)),
        v(
            "flagged_per_id",
            "flagged",
            ViewOp::GroupSum(flagged_per_id),
            None,
        ),
    ])
    .unwrap()
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let n = args.first().copied().unwrap_or(100_000);
    let batches = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        vec![10, 100, 1000]
    };
    let mut rng = Lcg(42);
    let mut repo = Repo::new(Counting::default());
    // Views are derived data; template search on their graph chunks only
    // costs reads (E228's option). TEMPLATES=1 keeps the default.
    let templates = std::env::var("TEMPLATES").is_ok_and(|v| v == "1");
    let mut db = repo.db_empty().unwrap();
    let mut next = 0u64;
    while next < n as u64 {
        let batch: Vec<Event> = (next..(next + 10_000).min(n as u64))
            .map(|i| {
                Event::upsert(
                    &format!("t/{i:020}"),
                    row(&mut rng, i).as_bytes(),
                    &format!("w{i}"),
                )
            })
            .collect();
        next += batch.len() as u64;
        db = repo.db_append(&db, &batch).unwrap();
    }
    repo.object_templates = templates;
    let (inc, sto, read) = (
        graph(Mode::Incremental),
        graph(Mode::Stored),
        graph(Mode::OnRead),
    );
    let t = Instant::now();
    let mut s_inc = repo.views_build(&inc, &db).unwrap();
    let build_ms = t.elapsed().as_secs_f64() * 1e3;
    let mut s_sto = s_inc.clone();
    println!(
        "{{\"rows\":{n},\"full_build_ms\":{build_ms:.1},\"per_id_groups\":{}}}",
        repo.view_rows(&inc, &s_inc, "per_id").unwrap().len()
    );
    let mut idem = 0u64;
    for &size in &batches {
        let (mut inc_ms, mut sto_ms, mut inc_reads, mut sto_reads) =
            (vec![], vec![], vec![], vec![]);
        for _ in 0..5 {
            let events: Vec<Event> = (0..size)
                .map(|_| {
                    idem += 1;
                    let id = format!("c{idem}");
                    match rng.next() % 10 {
                        0..=6 => {
                            let i = rng.next() % next;
                            Event::upsert(&format!("t/{i:020}"), row(&mut rng, i).as_bytes(), &id)
                        }
                        7 | 8 => {
                            next += 1;
                            Event::upsert(
                                &format!("t/{:020}", next - 1),
                                row(&mut rng, next - 1).as_bytes(),
                                &id,
                            )
                        }
                        _ => Event::remove(&format!("t/{:020}", rng.next() % next), false, &id),
                    }
                })
                .collect();
            db = repo.db_append(&db, &events).unwrap();
            repo.store().take();
            let t = Instant::now();
            s_inc = repo.views_update(&inc, &s_inc, &db).unwrap();
            inc_ms.push(t.elapsed().as_secs_f64() * 1e3);
            inc_reads.push(repo.store().take().reads as f64);
            let t = Instant::now();
            s_sto = repo.views_update(&sto, &s_sto, &db).unwrap();
            sto_ms.push(t.elapsed().as_secs_f64() * 1e3);
            sto_reads.push(repo.store().take().reads as f64);
        }
        let full = repo.views_build(&sto, &db).unwrap();
        assert_eq!(s_sto, full, "batch {size}");
        for v in ["per_id", "flagged", "flagged_per_id"] {
            assert_eq!(
                repo.view_rows(&inc, &s_inc, v).unwrap(),
                repo.view_rows(&sto, &full, v).unwrap(),
                "{v}"
            );
        }
        for (way, ms, reads) in [
            ("incremental", inc_ms, inc_reads),
            ("stored_rebuild", sto_ms, sto_reads),
        ] {
            println!(
                "{{\"rows\":{n},\"object_templates\":{templates},\"batch\":{size},\"way\":\"{way}\",\"update_ms\":{:.1},\"update_reads\":{}}}",
                median(ms),
                median(reads)
            );
        }
    }
    // Reading the per-id view: stored, against computed on read.
    let s_read = repo.views_build(&read, &db).unwrap();
    repo.store().take();
    let t = Instant::now();
    let stored = repo.view_rows(&inc, &s_inc, "per_id").unwrap();
    let (stored_ms, stored_reads) = (t.elapsed().as_secs_f64() * 1e3, repo.store().take().reads);
    let t = Instant::now();
    let computed = repo.view_rows(&read, &s_read, "per_id").unwrap();
    let (read_ms, read_reads) = (t.elapsed().as_secs_f64() * 1e3, repo.store().take().reads);
    assert_eq!(stored, computed);
    println!(
        "{{\"rows\":{n},\"read_per_id\":\"stored\",\"ms\":{stored_ms:.1},\"reads\":{stored_reads}}}"
    );
    println!(
        "{{\"rows\":{n},\"read_per_id\":\"on_read\",\"ms\":{read_ms:.1},\"reads\":{read_reads}}}"
    );
}
