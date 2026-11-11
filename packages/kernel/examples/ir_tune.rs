//! E209: each column chooses its own coding.
//!
//! `cargo run --release --example ir_tune [corpus ...]`
//!
//! `numbers`, `predict` and `step` are on or off for every column at once,
//! and E207 found real tables where they cost more than they save (the
//! registry: columns alone 0.966 of xz, with numbers 1.013). With `tune`,
//! the encoder tries every stream with each of them left out, keeps per
//! stream the choice whose bytes xz judges smallest, and keeps the result
//! only if the whole pack judges smaller. For each corpus and variant (all
//! with `prose` and `route`), xz -9 over the pack without and with `tune`,
//! against xz -9 over the files, how many streams changed coding and the
//! encode time; then blocks of 20 with a keyframe in the kernel. Registry
//! values are not printed.

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

fn ir_xz(files: &Files, o: Options) -> (usize, f64) {
    let t = Instant::now();
    let parts = ir::encode(files, o);
    let enc = t.elapsed().as_secs_f64() * 1e3;
    let z = xz(&ir::pack(&parts));
    let back = ir::decode(&ir::unpack(&unxz(&z)).unwrap()).unwrap();
    assert!(back == *files, "{o:?}: not exact");
    (z.len(), enc)
}

fn tuned(files: &Files, o: Options) -> String {
    let m = ir::tune_modes(files, o);
    let total = ir::directory(&ir::encode(
        files,
        Options {
            tune: false,
            route: false,
            ..o
        },
    ))
    .unwrap()
    .len();
    let count = |bit: u8| m.iter().filter(|(_, x)| x & bit != 0).count();
    format!(
        "{} of {total}: {} no numbers, {} no predict, {} no step",
        m.len(),
        count(1),
        count(2),
        count(4)
    )
}

fn blocks_kernel(files: &Files, o: Options) -> usize {
    let b = ir::encode_blocks(files, o, 20, true);
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
    stored(&kernel_store(&entries))
}

fn main() {
    let ps = Options {
        predict: true,
        step: true,
        ..TCN
    };
    let variants = [
        ("numbers", TCN),
        ("predict, step", ps),
        (
            "predict, step, siblings, nearest 16",
            Options {
                siblings: true,
                nearest: 16,
                ..ps
            },
        ),
    ];
    let cases = load(
        "tests/fixtures/cases",
        &["md", "json", "csv", "log", "py", "txt"],
    );
    let registry = load("tests/fixtures/san-diego-business-registry", &["csv"]);
    let mut project = cases.clone();
    project.extend(corpus("reports-drift"));
    let corpora = [
        ("reports-drift", corpus("reports-drift")),
        ("reports-random", corpus("reports-random")),
        ("exports", corpus("exports")),
        ("tables", corpus("tables")),
        ("cases text (real)", cases),
        ("cases text + drift", project),
        ("registry (real)", registry),
    ];
    let args: Vec<String> = std::env::args().skip(1).collect();
    let corpora: Vec<_> = corpora
        .into_iter()
        .filter(|(n, _)| args.is_empty() || args.iter().any(|a| n.contains(a.as_str())))
        .collect();
    println!(
        "| corpus | xz -9 | best without tune | best with tune | variant: without / with (streams that changed coding) | encode ms of the best (without / with) |"
    );
    println!("|---|---:|---:|---:|---|---|");
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        let mut cells = Vec::new();
        let mut best = [(f64::MAX, 0.0); 2];
        for (vn, o) in variants {
            let o = Options {
                prose: true,
                route: true,
                ..o
            };
            let r = [o, Options { tune: true, ..o }];
            let x = r.map(|o| ir_xz(files, o));
            for (b, (z, ms)) in best.iter_mut().zip(&x) {
                let v = *z as f64 / base;
                if v < b.0 {
                    *b = (v, *ms);
                }
            }
            cells.push(format!(
                "{vn}: {:.3} / {:.3} ({})",
                x[0].0 as f64 / base,
                x[1].0 as f64 / base,
                tuned(files, r[1])
            ));
        }
        println!(
            "| {name} | {base} B | {:.3} | {:.3} | {} | {:.0} / {:.0} |",
            best[0].0,
            best[1].0,
            cells.join("; "),
            best[0].1,
            best[1].1
        );
    }
    println!(
        "\nBlocks of 20 with a keyframe in the kernel, predict and step with prose and route, against xz -9 over the files:\n"
    );
    println!("| corpus | without tune | with tune |");
    println!("|---|---:|---:|");
    for (name, files) in &corpora {
        if name.starts_with("registry") {
            continue;
        }
        let base = xz(&solid(files)).len() as f64;
        let o = Options {
            prose: true,
            route: true,
            ..ps
        };
        let a = blocks_kernel(files, o) as f64 / base;
        let b = blocks_kernel(files, Options { tune: true, ..o }) as f64 / base;
        println!("| {name} | {a:.3} | {b:.3} |");
    }
}
