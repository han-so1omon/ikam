//! E221: each block's raw part as its own entry in the store.
//!
//! `cargo run --release --example ir_blockraw [corpus ...]`
//!
//! E217 stored each block (E197) as one entry, its raw part (E198, the
//! files kept as they are) packed with its streams. For prose-heavy folders
//! blocks then lost a fifth of the gain of one whole pack, since each
//! block's raw text was compressed apart from the others. Here each block's
//! raw part is its own entry, so the kernel's repack (templates, trained
//! dictionaries) sees the raw parts of all blocks as similar entries. The
//! E216 setting, untuned, blocks of 20 and 50 with a keyframe; against the
//! files in the kernel.

mod ir_common;

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

/// Blocks in the store: `split` puts each block's raw part in its own entry.
fn blocks_kernel(files: &Files, o: Options, per: usize, split: bool) -> usize {
    let b = ir::encode_blocks(files, o, per, true);
    assert!(ir::decode_blocks(&b).unwrap() == *files);
    let mut entries = vec![(
        "ir/skeletons".to_string(),
        ir::pack(&Parts {
            skeletons: b.skeletons.clone(),
            ..Parts::default()
        }),
    )];
    for (i, p) in b.blocks.iter().enumerate() {
        if split && !p.raw.is_empty() {
            entries.push((format!("ir/raw-{i:05}"), p.raw.clone()));
            let rest = Parts {
                raw: Vec::new(),
                ..p.clone()
            };
            entries.push((format!("ir/block-{i:05}"), ir::pack(&rest)));
        } else {
            entries.push((format!("ir/block-{i:05}"), ir::pack(p)));
        }
    }
    stored(&kernel_store(&entries))
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
        "| corpus | files in the kernel | whole pack | blocks of 20: together / raw apart | blocks of 50: together / raw apart |"
    );
    println!("|---|---:|---:|---:|---:|");
    for (name, files) in corpora
        .iter()
        .filter(|(n, _)| args.is_empty() || args.iter().any(|a| n.contains(a.as_str())))
    {
        let k = stored(&kernel_store(files)) as f64;
        let pack = ir::pack(&ir::encode(files, setting));
        let one = stored(&kernel_store(&vec![("ir/pack".to_string(), pack)])) as f64;
        let r = |per, split| blocks_kernel(files, setting, per, split) as f64 / k;
        println!(
            "| {name} | {k} B | {:.3} | {:.3} / {:.3} | {:.3} / {:.3} |",
            one / k,
            r(20, false),
            r(20, true),
            r(50, false),
            r(50, true)
        );
    }
    println!("\nEach cell against the files in the kernel.");
}
