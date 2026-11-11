//! E211: a table's rows stored in the order that compresses best.
//!
//! `cargo run --release --example ir_sort [corpus ...]`
//!
//! A Python prototype (E209's side finding) put the real registry, its rows
//! sorted by NAICS code with that column also kept in file order, at 0.909
//! of xz, against 0.961 for the best IR. With `sort`, each CSV table whose
//! lines are its rows is rewritten before the IR: the rows sorted (stably)
//! by the column whose order xz -6 judges smallest by column, then that
//! column in file order, from which the decoder recovers the order. For
//! each corpus and variant (all with `prose` and `route`), xz -9 over the
//! pack without and with `sort`, against xz -9 over the files, the column
//! each table was sorted by (by header name), and the encode time.
//! Registry values are not printed.

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
        ("numbers, tuned", Options { tune: true, ..TCN }),
        ("predict, step, tuned", Options { tune: true, ..ps }),
    ];
    let cases = load(
        "tests/fixtures/cases",
        &["md", "json", "csv", "log", "py", "txt"],
    );
    let registry = load("tests/fixtures/san-diego-business-registry", &["csv"]);
    let corpora = [
        ("exports", corpus("exports")),
        ("tables", corpus("tables")),
        ("cases text (real)", cases),
        ("registry (real)", registry),
    ];
    let args: Vec<String> = std::env::args().skip(1).collect();
    let corpora: Vec<_> = corpora
        .into_iter()
        .filter(|(n, _)| args.is_empty() || args.iter().any(|a| n.contains(a.as_str())))
        .collect();
    println!(
        "| corpus | xz -9 | best without sort | best with sort | variant: without / with sort | sorted by | encode ms of the best (without / with) |"
    );
    println!("|---|---:|---:|---:|---|---|---|");
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
            let r = [o, Options { sort: true, ..o }];
            let x = r.map(|o| ir_xz(files, o));
            for (b, (z, ms)) in best.iter_mut().zip(&x) {
                let v = *z as f64 / base;
                if v < b.0 {
                    *b = (v, *ms);
                }
            }
            cells.push(format!(
                "{vn}: {:.3} / {:.3}",
                x[0].0 as f64 / base,
                x[1].0 as f64 / base
            ));
        }
        println!(
            "| {name} | {base} B | {:.3} | {:.3} | {} | {} | {:.0} / {:.0} |",
            best[0].0,
            best[1].0,
            cells.join("; "),
            keys(files, Options { sort: true, ..TC }),
            best[0].1,
            best[1].1
        );
    }
}
