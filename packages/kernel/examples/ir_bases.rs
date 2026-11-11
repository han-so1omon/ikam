//! E216: route also judges whether files diff against a base.
//!
//! `cargo run --release --example ir_bases [corpus ...]`
//!
//! E215 found two settings split the corpora: predict, step and tune for
//! reports, siblings and nearest for exports and tables (where one file
//! repeats another's cells). With `bases`, `route` also judges per kind of
//! file whether its files diff against a base. For each corpus, xz -9 over
//! the pack against xz -9 over the files, and the encode and decode time,
//! for E215's best setting, with siblings and nearest 16 added, and with
//! `bases` too (tuned and not). All have `prose`, `route`, `sort`,
//! `transpose` and `sample` (1 MB), and no row keys.

mod ir_common;

use std::time::Instant;

use ikam_kernel::ir::{self, Options};
use ir_common::*;

fn load(rel: &str, exts: &[&str]) -> Files {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    let mut out = Vec::new();
    walk(&root, &root, exts, &mut out);
    out
}

fn walk(root: &std::path::Path, dir: &std::path::Path, exts: &[&str], out: &mut Files) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            walk(root, &p, exts, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
        {
            let name = p.strip_prefix(root).unwrap().to_string_lossy().into_owned();
            out.push((name, std::fs::read(&p).unwrap()));
        }
    }
}

#[allow(dead_code)]
fn ir_xz(files: &Files, o: Options) -> (usize, f64) {
    let t = Instant::now();
    let parts = ir::encode(files, o);
    let enc = t.elapsed().as_secs_f64() * 1e3;
    let z = xz(&ir::pack(&parts));
    let back = ir::decode(&ir::unpack(&unxz(&z)).unwrap()).unwrap();
    assert!(back == *files, "{o:?}: not exact");
    (z.len(), enc)
}

#[allow(dead_code)]
fn keys(files: &Files, o: Options) -> String {
    let k = ir::sort_keys(files, o);
    let mut names: Vec<String> = files
        .iter()
        .zip(&k)
        .filter_map(|((_, b), k)| {
            let k = (*k)?;
            let header = b.split(|&c| c == b'\n').next()?;
            let name = header.split(|&c| c == b',').nth(k)?;
            Some(String::from_utf8_lossy(name).trim().to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    let sorted = k.iter().filter(|k| k.is_some()).count();
    let csv = files.iter().filter(|(n, _)| n.ends_with(".csv")).count();
    format!(
        "{sorted} of {csv} csv sorted, by {}",
        if names.is_empty() {
            "-".into()
        } else {
            names.join(", ")
        }
    )
}

fn main() {
    let base_o = Options {
        prose: true,
        route: true,
        sort: true,
        transpose: true,
        sample: 1 << 20,
        ..TC
    };
    let full = Options {
        numbers: true,
        siblings: true,
        rows: true,
        nearest: 16,
        predict: true,
        step: true,
        ..base_o
    };
    let ps = Options {
        siblings: false,
        rows: false,
        nearest: 0,
        tune: true,
        ..full
    };
    let based = Options {
        rows: false,
        tune: true,
        ..full
    };
    let settings = [
        ("predict, step, tuned (E215)", ps),
        ("+ siblings, nearest", based),
        (
            "+ siblings, nearest, bases",
            Options {
                bases: true,
                ..based
            },
        ),
        (
            "same, untuned",
            Options {
                bases: true,
                tune: false,
                ..based
            },
        ),
    ];
    let cases = load(
        "tests/fixtures/cases",
        &["md", "json", "csv", "log", "py", "txt"],
    );
    let registry = load("tests/fixtures/san-diego-business-registry", &["csv"]);
    let mut drift_texts = corpus("reports-drift");
    drift_texts.extend(corpus("texts"));
    let mut cases_drift = cases.clone();
    cases_drift.extend(corpus("reports-drift"));
    let corpora = [
        ("reports-drift", corpus("reports-drift")),
        ("reports-random", corpus("reports-random")),
        ("exports", corpus("exports")),
        ("tables", corpus("tables")),
        ("texts", corpus("texts")),
        ("drift + texts", drift_texts),
        ("cases text (real)", cases),
        ("cases text + drift", cases_drift),
        ("registry (real)", registry),
    ];
    let args: Vec<String> = std::env::args().skip(1).collect();
    let corpora: Vec<_> = corpora
        .into_iter()
        .filter(|(n, _)| args.is_empty() || args.iter().any(|a| n.contains(a.as_str())))
        .collect();
    println!(
        "| corpus | xz -9 | {} |",
        settings.map(|(n, _)| n).join(" | ")
    );
    println!("|---|---:|{}", "---|".repeat(settings.len()));
    let mut worst = [0f64; 4];
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        let cells: Vec<String> = settings
            .iter()
            .zip(worst.iter_mut())
            .map(|((_, o), w)| {
                let (z, enc, dec) = ir_xz_dec(files, *o);
                let r = z as f64 / base;
                *w = w.max(r);
                format!("{r:.3} ({:.1} s, {dec:.0} ms)", enc / 1e3)
            })
            .collect();
        println!("| {name} | {base} B | {} |", cells.join(" | "));
    }
    println!(
        "\nWorst against xz -9 over the files: {}",
        settings
            .iter()
            .zip(worst)
            .map(|((n, _), w)| format!("{n} {w:.3}"))
            .collect::<Vec<_>>()
            .join("; ")
    );
}

fn ir_xz_dec(files: &Files, o: Options) -> (usize, f64, f64) {
    let t = Instant::now();
    let parts = ir::encode(files, o);
    let enc = t.elapsed().as_secs_f64() * 1e3;
    let z = xz(&ir::pack(&parts));
    let t = Instant::now();
    let back = ir::decode(&ir::unpack(&unxz(&z)).unwrap()).unwrap();
    let dec = t.elapsed().as_secs_f64() * 1e3;
    assert!(back == *files, "{o:?}: not exact");
    (z.len(), enc, dec)
}
