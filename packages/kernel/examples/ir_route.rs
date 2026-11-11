//! E208: never lose to xz: each kind of file goes through the IR or is
//! stored as it is, whichever compresses smaller.
//!
//! `cargo run --release --example ir_route [corpus ...]`
//!
//! E207 found the IR loses on real mixed folders (the cases text, 1.08 of
//! xz at best) and gains 3% on a real table. With `route`, files are grouped
//! by extension and first-line template; each group is judged on its own
//! (xz -6 over its IR pack against its files stored plain) and goes the
//! smaller way, and the routed set is then judged against all IR and all
//! plain. For each corpus and each IR variant (all with `prose`), xz -9 over
//! the pack without and with `route`, against xz -9 over the files, the
//! kinds `route` stored plain, and the encode time. Then blocks of 20 with a
//! keyframe in the kernel. Registry values are not printed.

mod ir_common;

use std::collections::BTreeSet;
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

fn plain_kinds(files: &Files, o: Options) -> String {
    let plain = ir::route(files, o);
    let kinds: BTreeSet<&str> = files
        .iter()
        .zip(&plain)
        .filter(|(_, p)| **p)
        .map(|((n, _), _)| n.rsplit_once('.').map_or("", |(_, e)| e))
        .collect();
    if kinds.is_empty() {
        "none".into()
    } else {
        kinds.into_iter().collect::<Vec<_>>().join(", ")
    }
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
        ("columns", TC),
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
    let mut mixed = corpus("reports-drift");
    mixed.extend(corpus("texts"));
    let mut project = cases.clone();
    project.extend(corpus("reports-drift"));
    let corpora = [
        ("reports-drift", corpus("reports-drift")),
        ("reports-random", corpus("reports-random")),
        ("exports", corpus("exports")),
        ("tables", corpus("tables")),
        ("texts", corpus("texts")),
        ("drift + texts", mixed),
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
        "| corpus | xz -9 | best IR with prose | best with route | variant: prose / route (kinds stored plain) | encode ms of the best (prose / route) |"
    );
    println!("|---|---:|---:|---:|---|---|");
    let mut worst = 0.0f64;
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        let mut cells = Vec::new();
        let mut best = [(f64::MAX, 0.0, ps); 2];
        for (vn, o) in variants {
            let o = Options { prose: true, ..o };
            let r = [o, Options { route: true, ..o }];
            let x = r.map(|o| ir_xz(files, o));
            for ((b, (z, ms)), o) in best.iter_mut().zip(&x).zip(r) {
                let v = *z as f64 / base;
                if v < b.0 {
                    *b = (v, *ms, o);
                }
            }
            cells.push(format!(
                "{vn}: {:.3} / {:.3} ({})",
                x[0].0 as f64 / base,
                x[1].0 as f64 / base,
                plain_kinds(files, r[1])
            ));
        }
        worst = worst.max(best[1].0);
        println!(
            "| {name} | {base} B | {:.3} | {:.3} | {} | {:.0} / {:.0} |",
            best[0].0,
            best[1].0,
            cells.join("; "),
            best[0].1,
            best[1].1
        );
    }
    println!("\nWorst best-with-route against xz -9 over the files: {worst:.3}\n");
    println!(
        "Blocks of 20 with a keyframe in the kernel, predict and step with prose, against xz -9 over the files:\n"
    );
    println!("| corpus | without route | with route |");
    println!("|---|---:|---:|");
    for (name, files) in &corpora {
        if name.starts_with("registry") {
            continue;
        }
        let base = xz(&solid(files)).len() as f64;
        let o = Options { prose: true, ..ps };
        let a = blocks_kernel(files, o) as f64 / base;
        let b = blocks_kernel(files, Options { route: true, ..o }) as f64 / base;
        println!("| {name} | {a:.3} | {b:.3} |");
    }
}
