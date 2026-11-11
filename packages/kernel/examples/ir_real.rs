//! E207: the IR on real files, not generated ones.
//!
//! `cargo run --release --example ir_real`
//!
//! E192 to E206 measured on generated reports. Two folders in the repo hold
//! files made elsewhere: `tests/fixtures/san-diego-business-registry` (the
//! city's public business tax registry, two CSV files, 51,000 rows, 10.8
//! MB) and the text files of `tests/fixtures/cases` (526 notes, playbooks,
//! forms, scripts and logs of 18 sample businesses, 832 KB: md, json, csv,
//! log, py, txt). For each, xz -9 and zstd -19 over the files, and each IR
//! variant with xz -9 over its pack, with and without `prose`; then the
//! schema (E202) and a column query (E203) on the registry. Values from
//! the registry are not printed, only column names and counts.

mod ir_common;

use std::path::Path;
use std::time::Instant;

use ikam_kernel::ir::{self, Options, query, schema};
use ir_common::*;

fn walk(root: &Path, dir: &Path, exts: &[&str], out: &mut Files) {
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

fn load(rel: &str, exts: &[&str]) -> Files {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    let mut out = Vec::new();
    walk(&root, &root, exts, &mut out);
    out
}

fn main() {
    let registry = load("tests/fixtures/san-diego-business-registry", &["csv"]);
    let cases = load(
        "tests/fixtures/cases",
        &["md", "json", "csv", "log", "py", "txt"],
    );
    let p = Options {
        predict: true,
        ..TCN
    };
    let ps = Options { step: true, ..p };
    let variants = [
        ("templates", T),
        ("columns", TC),
        ("numbers", TCN),
        ("row keys", TCNR),
        ("predict", p),
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
    for (name, files) in [("registry", &registry), ("cases text", &cases)] {
        let bytes: usize = files.iter().map(|f| f.1.len()).sum();
        let s = solid(files);
        let t = Instant::now();
        let base = xz(&s).len() as f64;
        let xz_ms = t.elapsed().as_secs_f64() * 1e3;
        let zs = zstd::bulk::compress(&s, 19).unwrap().len() as f64;
        println!(
            "\n## {name}: {} files, {bytes} B; xz -9 {base} B ({:.3} of the bytes, {xz_ms:.0} ms); zstd -19 {:.3} of xz\n",
            files.len(),
            base / bytes as f64,
            zs / base
        );
        println!(
            "| IR variant | xz over the pack (of xz -9 over the files) | with prose | encode ms | decode ms |"
        );
        println!("|---|---:|---:|---:|---:|");
        for (vn, o) in variants {
            let mut cells = Vec::new();
            let mut times = (0.0, 0.0);
            for prose in [false, true] {
                let o = Options { prose, ..o };
                let t = Instant::now();
                let parts = ir::encode(files, o);
                let enc = t.elapsed().as_secs_f64() * 1e3;
                let z = xz(&ir::pack(&parts));
                let t = Instant::now();
                let back = ir::decode(&ir::unpack(&unxz(&z)).unwrap()).unwrap();
                let dec = t.elapsed().as_secs_f64() * 1e3;
                assert!(back == *files, "{name} {vn}: not exact");
                cells.push(format!("{:.3}", z.len() as f64 / base));
                if !prose {
                    times = (enc, dec);
                }
            }
            println!(
                "| {vn} | {} | {} | {:.0} | {:.0} |",
                cells[0], cells[1], times.0, times.1
            );
        }
    }

    // the registry's schema: which column follows from which
    let header: Vec<String> = String::from_utf8_lossy(&registry[0].1)
        .lines()
        .next()
        .unwrap()
        .split(',')
        .map(str::to_string)
        .collect();
    let t = Instant::now();
    let deps = schema::dependencies(&registry, 50, 0.9);
    let ev = schema::events(&registry, &deps);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let col = |k: usize| {
        header
            .get(k)
            .cloned()
            .unwrap_or_else(|| format!("slot {k}"))
    };
    println!(
        "\n## registry schema ({ms:.0} ms): {} dependencies on the 14-column row, in the order of the columns\n",
        deps.iter()
            .filter(|d| ir::segments(&d.skeleton).unwrap().len() == 15)
            .count()
    );
    println!("| column | follows from | held | of lines | keys | blips | conflicts | changes |");
    println!("|---|---|---:|---:|---:|---:|---:|---:|");
    for (di, d) in deps.iter().enumerate() {
        if ir::segments(&d.skeleton).unwrap().len() != 15 {
            continue;
        }
        let count = |k| ev.iter().filter(|e| e.dep == di && e.kind == k).count();
        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            col(d.to),
            col(d.from),
            d.held,
            d.seen + d.keys,
            d.keys,
            count(schema::Kind::Blip),
            count(schema::Kind::Conflict),
            count(schema::Kind::Change)
        );
    }

    // a column query on the registry: how many rows per ownership type
    let parts = ir::encode(&registry, ps);
    let ix = query::index(&parts, &parts.skeletons).unwrap();
    let counts = ix.counts();
    let id = (0..parts.skeletons.len())
        .filter(|&i| ir::segments(&parts.skeletons[i]).unwrap().len() == 15)
        .max_by_key(|&i| counts[i])
        .unwrap();
    let mut q_ms = f64::MAX;
    let mut got = std::collections::BTreeMap::new();
    for _ in 0..5 {
        let t = Instant::now();
        let ix = query::index(&parts, &parts.skeletons).unwrap();
        let mut f = |i: usize| Ok(parts.streams[i].clone());
        let mut m = std::collections::BTreeMap::<Vec<u8>, usize>::new();
        for (_, v) in query::column(&ix, id, 2, &mut f).unwrap() {
            *m.entry(v).or_default() += 1;
        }
        q_ms = q_ms.min(t.elapsed().as_secs_f64() * 1e3);
        got = m;
    }
    let mut f_ms = f64::MAX;
    for _ in 0..5 {
        let t = Instant::now();
        let mut m = std::collections::BTreeMap::<Vec<u8>, usize>::new();
        for (n, b) in &registry {
            for (sk, cells) in ir::lex_lines(n, b) {
                if sk == parts.skeletons[id] {
                    *m.entry(cells[2].clone()).or_default() += 1;
                }
            }
        }
        f_ms = f_ms.min(t.elapsed().as_secs_f64() * 1e3);
        assert!(m == got);
    }
    println!(
        "\nRows per ownership type ({} types, {} rows) from the IR's column (predict, step, in memory): {q_ms:.1} ms; splitting the files: {f_ms:.1} ms.",
        got.len(),
        got.values().sum::<usize>()
    );
}
