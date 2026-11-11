//! E214: route and tune judged on a sample.
//!
//! `cargo run --release --example ir_sample [corpus ...]`
//!
//! `route` (E208) and `tune` (E209) judge with xz over whole encodes, which
//! took 14 s (columns) and 48 s (tuned) on the 10.8 MB registry. With
//! `sample`, they judge on about that many bytes: every file, each over
//! its share cut to its first line and an even spread of the rest. For each corpus and variant (with `prose`, `route`, `sort` and
//! `transpose`), xz -9 over the pack against xz -9 over the files, the raw
//! part's size (what route stored plain) and the encode time, judging on
//! everything, on 1 MB and on 256 KB.

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
    let ps = Options {
        predict: true,
        step: true,
        ..TCN
    };
    let variants = [
        ("columns", TC),
        ("predict, step, tuned", Options { tune: true, ..ps }),
    ];
    let cases = load(
        "tests/fixtures/cases",
        &["md", "json", "csv", "log", "py", "txt"],
    );
    let registry = load("tests/fixtures/san-diego-business-registry", &["csv"]);
    let mut mixed = cases.clone();
    mixed.extend(corpus("reports-drift"));
    let corpora = [
        ("reports-drift", corpus("reports-drift")),
        ("reports-random", corpus("reports-random")),
        ("exports", corpus("exports")),
        ("cases text (real)", cases),
        ("cases text + drift", mixed),
        ("registry (real)", registry),
    ];
    let args: Vec<String> = std::env::args().skip(1).collect();
    let corpora: Vec<_> = corpora
        .into_iter()
        .filter(|(n, _)| args.is_empty() || args.iter().any(|a| n.contains(a.as_str())))
        .collect();
    let budgets = [0usize, 1 << 20, 256 << 10];
    println!(
        "| corpus | xz -9 | bytes | variant | all / 1 MB / 256 KB | raw part KB | encode ms |"
    );
    println!("|---|---:|---:|---|---|---|---|");
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        let bytes: usize = files.iter().map(|(_, b)| b.len()).sum();
        for (vn, o) in variants {
            let set = budgets.map(|sample| Options {
                prose: true,
                route: true,
                sort: true,
                transpose: true,
                sample,
                ..o
            });
            let x = set.map(|o| ir_xz(files, o));
            let raw = set.map(|o| ir::encode(files, o).raw.len() / 1000);
            let j = |v: Vec<String>| v.join(" / ");
            println!(
                "| {name} | {base} B | {} KB | {vn} | {} | {} | {} |",
                bytes / 1000,
                j(x.iter()
                    .map(|(z, _)| format!("{:.3}", *z as f64 / base))
                    .collect()),
                j(raw.iter().map(|r| r.to_string()).collect()),
                j(x.iter().map(|(_, ms)| format!("{ms:.0}")).collect()),
            );
        }
    }
}
