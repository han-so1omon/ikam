//! E217: the E216 setting in the store.
//!
//! `cargo run --release --example ir_store [corpus ...]`
//!
//! E216's setting (columns, numbers, siblings, nearest 16, predict, step,
//! prose, route, sort, transpose, bases, a 1 MB sample; tuned and not) was
//! measured as one pack under xz. ikam stores files in its kernel, so this
//! measures what the kernel stores: the files themselves; the pack as one
//! entry; and blocks of 20 and 50 files with a keyframe (E197), which read
//! one block instead of every file. Each against the files in the kernel
//! (and xz -9 over the files), with the time to read one file from the
//! middle of the set (in memory, decoding its block, best of 5).

mod ir_common;

use std::time::Instant;

use ikam_kernel::ir::{self, Options, Parts};
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

fn blocks_kernel(files: &Files, o: Options, per: usize) -> (usize, f64) {
    let b = ir::encode_blocks(files, o, per, true);
    assert!(ir::decode_blocks(&b).unwrap() == *files);
    let mut entries = vec![(
        "ir/skeletons".to_string(),
        ir::pack(&Parts {
            skeletons: b.skeletons.clone(),
            ..Parts::default()
        }),
    )];
    entries.extend(
        b.blocks
            .iter()
            .enumerate()
            .map(|(i, p)| (format!("ir/block-{i:05}"), ir::pack(p))),
    );
    let mid = files.len() / 2;
    let mut best = f64::MAX;
    for _ in 0..5 {
        let t = Instant::now();
        let mut r = ir::reader(&b);
        let got = r.block(mid / per).unwrap();
        assert!(got.iter().any(|f| *f == files[mid]));
        best = best.min(t.elapsed().as_secs_f64() * 1e3);
    }
    (stored(&kernel_store(&entries)), best)
}

fn main() {
    let setting = Options {
        numbers: true,
        siblings: true,
        nearest: 16,
        predict: true,
        step: true,
        prose: true,
        route: true,
        sort: true,
        transpose: true,
        bases: true,
        sample: 1 << 20,
        ..TC
    };
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
        "| corpus | files | xz -9 | files in the kernel | variant | pack, one entry | blocks of 20 (read ms) | blocks of 50 (read ms) |"
    );
    println!("|---|---:|---:|---:|---|---:|---:|---:|");
    for (name, files) in &corpora {
        let x = xz(&solid(files)).len();
        let k = stored(&kernel_store(files));
        for (vn, o) in [
            ("untuned", setting),
            (
                "tuned",
                Options {
                    tune: true,
                    ..setting
                },
            ),
        ] {
            let pack = ir::pack(&ir::encode(files, o));
            let one = stored(&kernel_store(&vec![("ir/pack".to_string(), pack)]));
            let (b20, r20) = blocks_kernel(files, o, 20);
            let (b50, r50) = blocks_kernel(files, o, 50);
            let f = |v: usize| format!("{:.3} / {:.3}", v as f64 / k as f64, v as f64 / x as f64);
            println!(
                "| {name} | {} | {x} B | {k} B ({:.3} of xz) | {vn} | {} | {} ({r20:.1}) | {} ({r50:.1}) |",
                files.len(),
                k as f64 / x as f64,
                f(one),
                f(b20),
                f(b50)
            );
        }
    }
    println!("\nEach cell: against the files in the kernel / against xz -9 over the files.");
}
