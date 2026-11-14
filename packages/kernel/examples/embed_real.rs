//! E256: embeddings keyed by content hash (src/embeds.rs, E255) on real
//! text: READMEs of OSI-licensed PyPI projects, a fragment per paragraph,
//! and their real release histories.
//! `cargo run --release --example embed_real -- DIR`
//!
//! DIR is what `examples/real_text/prepare.py` writes. Calls and bytes
//! embedded are the measure: the model is a stand-in of the real model's
//! dimension (hashed trigrams), and the real model's measured cost a
//! fragment (model.json, WordLlama on this machine) turns calls into time.
//! Prints, for the build and for each step of the release histories, the
//! fragments embedded by content hash against every fragment of every
//! changed row, and bytes embedded against whole changed READMEs.

mod db_common;
mod real_common;

use std::fs;
use std::time::Instant;

use db_common::Counting;
use ikam_kernel::{Db, Event, Repo};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// A stand-in of `dim` dimensions: hashed character trigrams, normalized.
fn stand_in(dim: usize, f: &[u8]) -> Vec<f32> {
    let mut v = vec![0f32; dim];
    for w in f.windows(3) {
        let h = w.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        });
        v[(h % dim as u64) as usize] += 1.0;
    }
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    v.into_iter().map(|x| x / n).collect()
}

fn frags(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n').filter(|f| !f.is_empty())
}

fn append(repo: &mut Repo<Counting>, db: &Db, rows: &[(String, String)], tag: &str) -> Db {
    let mut db = *db;
    let ev: Vec<Event> = rows
        .iter()
        .enumerate()
        .map(|(i, (k, v))| Event::upsert(k, v.as_bytes(), &format!("{tag}{i}")))
        .collect();
    for chunk in ev.chunks(5_000) {
        db = repo.db_append(&db, chunk).unwrap();
    }
    db
}

fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let model = fs::read_to_string(format!("{dir}/model.json")).unwrap();
    let dim = real_common::dim(&dir);
    let call_us = real_common::model_num(&model, "single_call_us");
    let batch_us = real_common::model_num(&model, "batch_us_per_fragment");
    let docs = real_common::docs(&dir);
    let mut repo = Repo::new(Counting::default());
    let empty = repo.db_empty().unwrap();
    let rows: Vec<(String, String)> = docs
        .iter()
        .map(|(k, t)| (format!("r/{k}"), t.clone()))
        .collect();
    let db = append(&mut repo, &empty, &rows, "b");
    repo.object_templates = false;
    repo.prune_diff = true; // E249
    let occ: usize = rows.iter().map(|r| frags(&r.1).count()).sum();
    let bytes: usize = rows
        .iter()
        .map(|r| frags(&r.1).map(str::len).sum::<usize>())
        .sum();
    let before = repo.store().stored();
    let e0 = repo.embeds_empty().unwrap();
    let mut embedded_bytes = 0usize;
    let t = Instant::now();
    let (_, s) = repo
        .embeds_update(&e0, &empty, &db, "r/", "m1", &mut |f| {
            embedded_bytes += f.len();
            stand_in(dim, f)
        })
        .unwrap();
    let view_ms = ms(t);
    println!(
        "{{\"phase\":\"build\",\"rows\":{},\"fragments\":{occ},\"calls\":{},\"cached\":{},\"fragment_bytes\":{bytes},\"embedded_bytes\":{embedded_bytes},\"view_ms\":{view_ms:.0},\"stored_bytes\":{},\"per_occurrence_bytes\":{},\"model_us_single\":{call_us:.1},\"model_us_batched\":{batch_us:.2}}}",
        rows.len(),
        s.embedded,
        s.cached,
        repo.store().stored() - before,
        occ * dim * 4
    );

    // Release histories: the oldest README of each project, then each
    // later release as one batch of updates.
    let hist = real_common::history(&dir);
    let steps = hist.values().map(Vec::len).max().unwrap_or(0);
    let mut repo = Repo::new(Counting::default());
    let empty = repo.db_empty().unwrap();
    repo.object_templates = false;
    repo.prune_diff = true;
    let mut e = repo.embeds_empty().unwrap();
    let mut db = empty;
    for step in 0..steps {
        let batch: Vec<(String, String)> = hist
            .iter()
            .filter_map(|(k, v)| v.get(step).map(|t| (format!("h/{k}"), t.clone())))
            .collect();
        let prev: std::collections::BTreeMap<String, String> = if step == 0 {
            Default::default()
        } else {
            hist.iter()
                .filter(|(_, v)| v.len() > step)
                .map(|(k, v)| (format!("h/{k}"), v[step - 1].clone()))
                .collect()
        };
        let changed: Vec<&(String, String)> = batch
            .iter()
            .filter(|(k, t)| prev.get(k) != Some(t))
            .collect();
        let occ: usize = changed.iter().map(|r| frags(&r.1).count()).sum();
        let whole_bytes: usize = changed.iter().map(|r| r.1.len()).sum();
        let next = append(&mut repo, &db, &batch, &format!("s{step}-"));
        let mut eb = 0usize;
        let t = Instant::now();
        let (ne, s) = repo
            .embeds_update(&e, &db, &next, "h/", "m1", &mut |f| {
                eb += f.len();
                stand_in(dim, f)
            })
            .unwrap();
        println!(
            "{{\"phase\":\"history\",\"step\":{step},\"rows\":{},\"changed_rows\":{},\"fragments_of_changed\":{occ},\"calls\":{},\"cached\":{},\"embedded_bytes\":{eb},\"whole_readme_bytes\":{whole_bytes},\"view_ms\":{:.1}}}",
            batch.len(),
            changed.len(),
            s.embedded,
            s.cached,
            ms(t)
        );
        (e, db) = (ne, next);
    }
}
