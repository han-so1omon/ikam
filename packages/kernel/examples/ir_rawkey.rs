//! E225: later blocks' raw text compressed against the keyframe's.
//!
//! `cargo run --release --example ir_rawkey [corpus ...]`
//!
//! E224's prototype found that compressing a block's raw text after the
//! first block's cuts blocks of 50 of the fixture cases by a fifth. With
//! `raw_keyframe`, `encode_blocks` (with a keyframe) stores each later
//! block's raw part as a zstd frame with the first block's raw part as its
//! dictionary, where that is smaller. The E216 setting, untuned, blocks of
//! 20 and 50 with a keyframe, each block one entry in the kernel (as E217);
//! against the files in the kernel, with the time to read the middle block
//! (keyframe decoded too, best of 5).

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

/// Blocks in the store, each one entry: bytes stored and ms to read the
/// middle block.
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
    let mid = b.blocks.len() / 2;
    let mut best = f64::MAX;
    for _ in 0..5 {
        let t = Instant::now();
        ir::reader(&b).block(mid).unwrap();
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
    let mut drift_texts = corpus("reports-drift");
    drift_texts.extend(corpus("texts"));
    let mut cases_drift = cases.clone();
    cases_drift.extend(corpus("reports-drift"));
    let corpora = [
        ("reports-drift", corpus("reports-drift")),
        ("exports", corpus("exports")),
        ("tables", corpus("tables")),
        ("texts", corpus("texts")),
        ("drift + texts", drift_texts),
        ("cases text (real)", cases),
        ("cases text + drift", cases_drift),
    ];
    let args: Vec<String> = std::env::args().skip(1).collect();
    println!(
        "| corpus | files in the kernel | whole pack | blocks of 20: before / raw keyframe (read ms) | blocks of 50: before / raw keyframe (read ms) |"
    );
    println!("|---|---:|---:|---:|---:|");
    let with = Options {
        raw_keyframe: true,
        ..setting
    };
    for (name, files) in corpora
        .iter()
        .filter(|(n, _)| args.is_empty() || args.iter().any(|a| n.contains(a.as_str())))
    {
        let k = stored(&kernel_store(files)) as f64;
        let pack = ir::pack(&ir::encode(files, setting));
        let one = stored(&kernel_store(&vec![("ir/pack".to_string(), pack)])) as f64;
        let cell = |per| {
            let (a, ra) = blocks_kernel(files, setting, per);
            let (b, rb) = blocks_kernel(files, with, per);
            format!(
                "{:.3} ({ra:.1}) / {:.3} ({rb:.1})",
                a as f64 / k,
                b as f64 / k
            )
        };
        println!(
            "| {name} | {k} B | {:.3} | {} | {} |",
            one / k,
            cell(20),
            cell(50)
        );
    }
    println!("\nEach cell against the files in the kernel.");
}
