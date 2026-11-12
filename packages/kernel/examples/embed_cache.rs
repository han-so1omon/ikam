//! E255: embeddings keyed by content hash (src/embeds.rs) against
//! embedding every fragment where it occurs.
//! `cargo run --release --example embed_cache -- N POOL BATCH...`
//!
//! Rows of 3 lines, each line drawn from a pool of POOL lines with Zipf
//! frequencies (line `r` about as common as 1/r), so fragments repeat as
//! boilerplate does. The model is synthetic: character trigrams hashed into
//! 64 dimensions with a seed, normalized (a stand-in; a real model costs far
//! more a call, so calls are the measure). Prints embedding calls, time and
//! bytes for the build, for batches of changed rows, and for a model change
//! (a new seed and model id), each with the cache and without it.
//! Synthetic data only.

mod db_common;

use std::collections::BTreeMap;
use std::time::Instant;

use db_common::{Counting, Lcg};
use ikam_kernel::embeds::Embeds;
use ikam_kernel::{Event, Repo};

const DIM: usize = 64;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// The synthetic model `seed`: hashed character trigrams, normalized.
fn embed(seed: u64, f: &[u8]) -> Vec<f32> {
    let mut v = vec![0f32; DIM];
    for w in f.windows(3) {
        let h = w.iter().fold(seed ^ 0x9e37_79b9_7f4a_7c15, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        });
        v[(h % DIM as u64) as usize] += if h >> 63 == 0 { 1.0 } else { -1.0 };
    }
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    v.into_iter().map(|x| x / n).collect()
}

fn zipf(rng: &mut Lcg, pool: usize) -> usize {
    let u = (rng.next() % 1_000_000) as f64 / 1e6;
    ((pool as f64).powf(u) as usize).saturating_sub(1)
}

fn line(r: usize) -> String {
    format!(
        "fragment {r}: the quick reading of section {} under clause {}",
        r * 7 % 1000,
        r % 97
    )
}

fn row(rng: &mut Lcg, pool: usize) -> String {
    (0..3)
        .map(|_| line(zipf(rng, pool)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let (n, pool, batches) = (args[0], args[1], args[2..].to_vec());
    let mut rng = Lcg(3);
    let mut repo = Repo::new(Counting::default());
    let empty = repo.db_empty().unwrap();
    let mut db = empty;
    let rows: Vec<Event> = (0..n)
        .map(|i| {
            Event::upsert(
                &format!("r/{i:08}"),
                row(&mut rng, pool).as_bytes(),
                &format!("s{i}"),
            )
        })
        .collect();
    for chunk in rows.chunks(10_000) {
        db = repo.db_append(&db, chunk).unwrap();
    }
    repo.object_templates = false;
    repo.prune_diff = true; // E249
    let occurrences = 3 * n;
    // Without the cache: one call a fragment occurrence.
    let t = Instant::now();
    let mut sink = 0f32;
    for e in &rows {
        let ikam_kernel::Op::Upsert(v) = &e.op else {
            continue;
        };
        for f in v.split(|&b| b == b'\n') {
            sink += embed(1, f)[0];
        }
    }
    let plain_ms = ms(t);
    println!(
        "{{\"rows\":{n},\"pool\":{pool},\"way\":\"every_occurrence\",\"phase\":\"build\",\"calls\":{occurrences},\"ms\":{plain_ms:.0},\"vector_bytes\":{},\"sink\":{sink:.1}}}",
        occurrences * DIM * 4
    );
    // With the cache.
    let stored0 = repo.store().stored();
    let e0 = repo.embeds_empty().unwrap();
    let t = Instant::now();
    let (mut e, s) = repo
        .embeds_update(&e0, &empty, &db, "r/", "m1", &mut |f| embed(1, f))
        .unwrap();
    println!(
        "{{\"rows\":{n},\"pool\":{pool},\"way\":\"content_hash\",\"phase\":\"build\",\"calls\":{},\"cached\":{},\"ms\":{:.0},\"stored_bytes\":{}}}",
        s.embedded,
        s.cached,
        ms(t),
        repo.store().stored() - stored0
    );
    let mut per: BTreeMap<usize, Vec<(f64, usize, usize)>> = BTreeMap::new();
    let mut idem = 0;
    for &size in &batches {
        for _ in 0..5 {
            let ev: Vec<Event> = (0..size)
                .map(|_| {
                    idem += 1;
                    Event::upsert(
                        &format!("r/{:08}", rng.next() as usize % n),
                        row(&mut rng, pool).as_bytes(),
                        &format!("u{idem}"),
                    )
                })
                .collect();
            let next = repo.db_append(&db, &ev).unwrap();
            let t = Instant::now();
            let (ne, s) = repo
                .embeds_update(&e, &db, &next, "r/", "m1", &mut |f| embed(1, f))
                .unwrap();
            per.entry(size)
                .or_default()
                .push((ms(t), s.embedded, s.cached));
            (e, db) = (ne, next);
        }
    }
    for (size, v) in per {
        let k = v.len() as f64;
        let (t, c, h) = v
            .iter()
            .fold((0.0, 0, 0), |a, x| (a.0 + x.0, a.1 + x.1, a.2 + x.2));
        println!(
            "{{\"rows\":{n},\"pool\":{pool},\"way\":\"content_hash\",\"phase\":\"update\",\"batch\":{size},\"calls\":{:.1},\"cached\":{:.1},\"every_occurrence_calls\":{},\"ms\":{:.2}}}",
            c as f64 / k,
            h as f64 / k,
            3 * size,
            t / k
        );
    }
    // A model change: a new id, the row view rebuilt from the rows.
    let before = repo.store().stored();
    let t = Instant::now();
    let fresh = Embeds { rows: e0.rows, ..e };
    let (_, s) = repo
        .embeds_update(&fresh, &empty, &db, "r/", "m2", &mut |f| embed(2, f))
        .unwrap();
    println!(
        "{{\"rows\":{n},\"pool\":{pool},\"way\":\"content_hash\",\"phase\":\"model_change\",\"calls\":{},\"every_occurrence_calls\":{occurrences},\"ms\":{:.0},\"stored_bytes\":{}}}",
        s.embedded,
        ms(t),
        repo.store().stored() - before
    );
}
