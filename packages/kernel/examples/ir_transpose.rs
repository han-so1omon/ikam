//! E212: tables stored plain, laid out by column.
//!
//! `cargo run --release --example ir_transpose [corpus ...]`
//!
//! E211 left the registry at 0.942 of xz with its rows sorted; the same sort
//! over a plain transpose (Python) reached 0.909, so the IR's cell framing
//! costs about half of what the sort gains. With `transpose`, a CSV table
//! that `route` stores as it is goes into the raw part laid out by column:
//! the header, each line's field count, then each column's fields one per
//! line (E213: without the quotes of a column whose every field is
//! quoted). `route` then judges the IR against that layout. For each corpus
//! and variant (all with `prose` and `route`), xz -9 over the pack with
//! neither, `sort`, `transpose` and both, against xz -9 over the files, and
//! the size of the raw part (what was stored plain). Registry values are not
//! printed.

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
        "| corpus | xz -9 | variant | route | + sort | + transpose | + both | raw part, KB (route / both) | encode ms (route / both) |"
    );
    println!("|---|---:|---|---:|---:|---:|---:|---|---|");
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        for (vn, o) in variants {
            let o = Options {
                prose: true,
                route: true,
                ..o
            };
            let set = [
                o,
                Options { sort: true, ..o },
                Options {
                    transpose: true,
                    ..o
                },
                Options {
                    sort: true,
                    transpose: true,
                    ..o
                },
            ];
            let x = set.map(|o| ir_xz(files, o));
            let raw = |o: Options| ir::encode(files, o).raw.len() / 1000;
            println!(
                "| {name} | {base} B | {vn} | {} | {} / {} | {:.0} / {:.0} |",
                x.iter()
                    .map(|(z, _)| format!("{:.3}", *z as f64 / base))
                    .collect::<Vec<_>>()
                    .join(" | "),
                raw(set[0]),
                raw(set[3]),
                x[0].1,
                x[3].1
            );
        }
    }
}
