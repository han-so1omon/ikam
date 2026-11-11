//! E192 to E197: intermediate representations as compression, and what
//! else they buy.
//!
//! `cargo run --release --example ir_compress [corpus ...]`
//!
//! Corpora (the report ones are synthetic, deterministic, modelled on
//! mercante-personal's lead-lag report renderer, `leadlag/cli.py render()`):
//! - `reports-drift`: 300 runs; numbers change only when a month of data
//!   arrives (about every 20 runs), as rerunning a study on the same data does;
//! - `reports-random`: 300 runs with every number redrawn each run (worst case);
//! - `reports-30`: 30 random runs (the shape of the Mercante demo's test);
//! - `exports`: 100 drift runs, each report's table also as csv and json;
//! - `tables`: `fixtures/tables` csv, json and md (24 real workbooks);
//! - `texts`: `fixtures/multiformat` md (24 prose texts, a control);
//! - `dir:<path>`: every file directly in a folder (only sizes are printed).
//!
//! Methods: gzip -9, xz -9 and zstd -19 over all files as one stream; zstd 3
//! with a trained dictionary per file (the bench's baseline); the kernel
//! (one repository at the archive read weight, repacked); and the IR codec
//! (`ikam_kernel::ir`) with options added one at a time, then xz -9 over its
//! pack. The best IR is also measured with zstd -19 per part (each column
//! readable alone) and stored in the kernel. Every method must give back
//! every file byte-exact. Then the benefits beyond size, on `reports-drift`.

mod ir_common;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use ikam_kernel::ir::{self, Options, Parts};
use ikam_kernel::{Id, Kind, Object, Store, TreeEntry};
use ir_common::*;

fn sizes(name: &str, files: &Files) {
    let raw: usize = files.iter().map(|(_, b)| b.len()).sum();
    let mut rows = vec![
        solid_method("gzip -9", files, gz, ungz),
        solid_method("xz -9", files, xz, unxz),
        solid_method("zstd -19", files, zst, unzst),
        dict_method(files),
        kernel_method("kernel (one graph, repacked)", files),
    ];
    let variants = [
        ("IR templates, xz", T),
        ("IR +columns, xz", TC),
        ("IR +numbers as deltas, xz", TCN),
        ("IR +row keys, xz", TCNR),
        ("IR +siblings (canonical), xz", ALL),
        (
            "IR numbers+siblings, no row keys, xz",
            Options { rows: false, ..ALL },
        ),
    ];
    let ir_rows: Vec<Row> = variants
        .iter()
        .map(|(n, o)| ir_method(n, files, *o))
        .collect();
    let best = variants[(0..ir_rows.len())
        .min_by_key(|&i| ir_rows[i].bytes)
        .unwrap()];
    rows.extend(ir_rows);
    rows.push(ir_zstd_parts(files, best.1));
    rows.push(ir_kernel(files, best.1));
    println!("\nbest IR for {name}: {}", best.0);
    let xz_bytes = rows[1].bytes as f64;
    println!("\n## {name}: {} files, {raw} bytes", files.len());
    println!("| method | bytes | of raw | vs xz | encode ms | decode ms |");
    println!("|---|---|---|---|---|---|");
    for r in &rows {
        println!(
            "| {} | {} | {:.4} | {:.3} | {:.1} | {:.1} |",
            r.method,
            r.bytes,
            r.bytes as f64 / raw as f64,
            r.bytes as f64 / xz_bytes,
            r.enc_ms,
            r.dec_ms
        );
    }
    let parts = ir::encode(files, best.1);
    let layout = xz(&ir::pack(&Parts {
        streams: Vec::new(),
        ..parts.clone()
    }))
    .len();
    let values = xz(&ir::pack(&Parts {
        skeletons: Vec::new(),
        files: Vec::new(),
        directory: Vec::new(),
        ..parts.clone()
    }))
    .len();
    println!(
        "IR shape: {} distinct line skeletons, {} streams; layout {} B and values {} B under xz",
        parts.skeletons.len(),
        parts.streams.len(),
        layout,
        values
    );
}

// ---------------------------------------------------------------- benefits

/// Cells of every table row in `files` for `link` at cell `col`, by text
/// parsing.
fn scan_column(files: &Files, link: &str, col: usize) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let prefix = format!("| {link} |");
    for (_, b) in files {
        for line in std::str::from_utf8(b).unwrap().lines() {
            if line.starts_with(&prefix) {
                let cells: Vec<&str> = line
                    .trim_matches(|c| c == '|' || c == ' ')
                    .split(" | ")
                    .collect();
                out.push(cells[col].as_bytes().to_vec());
            }
        }
    }
    out
}

/// The table row skeleton: the one with 11 cells.
fn row_skeleton(p: &Parts) -> usize {
    p.skeletons
        .iter()
        .position(|s| ir::segments(s).unwrap().len() == 12)
        .unwrap()
}

fn query(files: &Files) {
    let parts = ir::encode(files, TCNR);
    let dir = ir::directory(&parts).unwrap();
    let zs: Vec<Vec<u8>> = parts.streams.iter().map(|s| zst(s)).collect();
    let archive = xz(&solid(files));
    let rs = row_skeleton(&parts);
    let is_col = |k: &(usize, usize, Vec<u8>)| k.0 == rs && k.1 == 3 && k.2 != b"link";
    let longest = (0..dir.len())
        .filter(|&i| is_col(&dir[i]))
        .max_by_key(|&i| parts.streams[i].len())
        .unwrap();
    let link = String::from_utf8(dir[longest].2.clone()).unwrap();
    let zdir = zst(&parts.directory);
    // One link's dev rho over every run.
    let (ir_ms, (ir_vals, read)) = timed(|| {
        let i = dir
            .iter()
            .position(|k| k.0 == rs && k.1 == 3 && k.2 == link.as_bytes())
            .unwrap();
        let _ = ir::unpack(&ir::pack(&Parts {
            directory: unzst(&zdir),
            ..Parts::default()
        }));
        (
            ir::stream_values(&unzst(&zs[i])).unwrap(),
            zs[i].len() + zdir.len(),
        )
    });
    let (raw_ms, raw_vals) = timed(|| scan_column(files, &link, 3));
    let (xz_ms, xz_vals) = timed(|| scan_column(&unsolid(&unxz(&archive)), &link, 3));
    assert_eq!(ir_vals, raw_vals);
    assert_eq!(xz_vals, raw_vals);
    println!(
        "\n### Column query: `{link}` dev rho over {} runs\n- IR: {ir_ms:.3} ms reading {read} B (one column and the directory)\n- text in memory: {raw_ms:.3} ms reading {} B\n- from the xz archive: {xz_ms:.3} ms reading {} B",
        ir_vals.len(),
        files.iter().map(|f| f.1.len()).sum::<usize>(),
        archive.len()
    );
    // The price: one file is read by decoding every stream up to it.
    let repo = kernel_store(files);
    let latest = &files.last().unwrap().1;
    let (one_ms, _) = timed(|| repo.read_content(&Id::of_content(latest)).unwrap());
    let (all_ms, _) = timed(|| ir::decode(&parts).unwrap());
    println!(
        "- reading the latest report alone: kernel {one_ms:.3} ms; IR {all_ms:.1} ms (decodes every file)"
    );
    // Every link's mean dev rho (an aggregate over a column family).
    let (agg_ms, means) = timed(|| {
        let mut m: BTreeMap<Vec<u8>, (f64, usize)> = BTreeMap::new();
        for (i, k) in dir.iter().enumerate().filter(|(_, k)| is_col(k)) {
            for v in ir::stream_values(&unzst(&zs[i])).unwrap() {
                let e = m.entry(k.2.clone()).or_default();
                e.0 += std::str::from_utf8(&v).unwrap().parse::<f64>().unwrap();
                e.1 += 1;
            }
        }
        m
    });
    let agg_read: usize = dir
        .iter()
        .enumerate()
        .filter(|(_, k)| is_col(k))
        .map(|(i, _)| zs[i].len())
        .sum::<usize>()
        + zdir.len();
    let (scan_ms, scanned) = timed(|| {
        let mut m: BTreeMap<Vec<u8>, (f64, usize)> = BTreeMap::new();
        for (_, b) in files {
            for line in std::str::from_utf8(b)
                .unwrap()
                .lines()
                .filter(|l| l.starts_with("| ") && !l.starts_with("| link"))
            {
                let cells: Vec<&str> = line
                    .trim_matches(|c| c == '|' || c == ' ')
                    .split(" | ")
                    .collect();
                let e = m.entry(cells[0].as_bytes().to_vec()).or_default();
                e.0 += cells[3].parse::<f64>().unwrap();
                e.1 += 1;
            }
        }
        m
    });
    assert!(
        means
            .iter()
            .zip(&scanned)
            .all(|(a, b)| a.0 == b.0 && a.1.1 == b.1.1 && (a.1.0 - b.1.0).abs() < 1e-9)
    );
    println!(
        "- every link's mean dev rho ({} links): IR {agg_ms:.3} ms reading {agg_read} B; text scan {scan_ms:.3} ms",
        means.len()
    );
}

/// Bytes the kernel adds when `new` is committed over `old`, its ms, and
/// how much the store grew once repacked.
fn kernel_edit(old: &Files, new: &Files) -> (usize, f64, i64) {
    let mut repo = kernel_store(old);
    let before: HashSet<Id> = repo.store().ids().unwrap().into_iter().collect();
    let t = Instant::now();
    let tree: Vec<TreeEntry> = new
        .iter()
        .map(|(n, b)| TreeEntry {
            name: n.replace('/', "|"),
            kind: Kind::File,
            id: repo.put_content(b).unwrap().id,
        })
        .collect();
    let tree = repo.put(&Object::tree(tree).unwrap()).unwrap();
    repo.commit("main", tree, "").unwrap();
    let took = ms(t);
    assert!(
        new.iter()
            .all(|(_, b)| repo.read_content(&Id::of_content(b)).unwrap() == *b)
    );
    let s = repo.store();
    let added = s
        .ids()
        .unwrap()
        .iter()
        .filter(|i| !before.contains(i))
        .map(|i| s.read(i).unwrap().len())
        .sum();
    let size0: usize = before.iter().map(|i| s.read(i).unwrap().len()).sum();
    repo.repack().unwrap();
    assert!(
        new.iter()
            .all(|(_, b)| repo.read_content(&Id::of_content(b)).unwrap() == *b)
    );
    (added, took, stored(&repo) as i64 - size0 as i64)
}

fn propagate(files: &Files) {
    // Revise one link's mechanism sentence in every report that lists it.
    let text = String::from_utf8(files[0].1.clone()).unwrap();
    let line = text
        .lines()
        .rfind(|l| l.starts_with("- **"))
        .unwrap()
        .to_string();
    let fixed = line.replace(". ", "; revised. ").replace(".\u{0}", "") + " (revised)";
    let edited: Files = files
        .iter()
        .map(|(n, b)| {
            (
                n.clone(),
                String::from_utf8(b.clone())
                    .unwrap()
                    .replace(&line, &fixed)
                    .into_bytes(),
            )
        })
        .collect();
    let touched = files
        .iter()
        .zip(&edited)
        .filter(|(a, b)| a.1 != b.1)
        .count();
    let (old_p, new_p) = (ir::encode(files, TCNR), ir::encode(&edited, TCNR));
    let changed = old_p
        .skeletons
        .iter()
        .zip(&new_p.skeletons)
        .filter(|(a, b)| a != b)
        .count();
    let (raw_added, raw_ms, raw_grew) = kernel_edit(files, &edited);
    let (ir_added, ir_ms, ir_grew) = kernel_edit(&part_entries(&old_p), &part_entries(&new_p));
    println!(
        "\n### Edit propagation: one link's mechanism sentence revised\n- {touched} reports change; in the IR {changed} line skeleton changes and every other part is identical: {}\n- kernel adds {raw_added} B storing the {touched} edited reports ({raw_ms:.1} ms; {raw_grew:+} B after repack), {ir_added} B storing the edited IR ({ir_ms:.1} ms; {ir_grew:+} B after repack); the skeleton table is {} B",
        old_p.files == new_p.files
            && old_p.streams == new_p.streams
            && old_p.directory == new_p.directory,
        part_entries(&old_p)[0].1.len()
    );
}

fn drift(files: &Files) {
    // Ingest in run order; a report's new line skeletons and its largest
    // jump in a row-keyed numeric column flag a layout change or an outlier.
    let mut seen: HashSet<Vec<u8>> = HashSet::new();
    let mut last: HashMap<(Vec<u8>, usize, Vec<u8>), f64> = HashMap::new();
    let mut max_jump: HashMap<usize, f64> = HashMap::new();
    let mut normal_new = Vec::new();
    let mut normal_jump: f64 = 0.0;
    let text = String::from_utf8(files.last().unwrap().1.clone()).unwrap();
    let layout = text
        .replace("| earlier runs |", "| earlier runs | notes |")
        .replace("---|\n", "---|---|\n");
    let layout = layout
        .lines()
        .map(|l| {
            if l.starts_with("| ") && !l.starts_with("| link") {
                format!("{l} - |")
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let outlier = {
        let row = text
            .lines()
            .find(|l| l.starts_with("| ") && !l.starts_with("| link"))
            .unwrap();
        let mut cells: Vec<&str> = row.split(" | ").collect();
        cells[3] = "7.312";
        text.replace(row, &cells.join(" | "))
    };
    let mut all = files.clone();
    all.push(("reports/new-layout.md".into(), layout.into_bytes()));
    all.push(("reports/outlier.md".into(), outlier.into_bytes()));
    let t = Instant::now();
    let mut flags = Vec::new();
    for (fi, (name, b)) in all.iter().enumerate() {
        let lines = ir::lex_lines(name, b);
        let fresh: HashSet<&Vec<u8>> = lines
            .iter()
            .map(|l| &l.0)
            .filter(|s| !seen.contains(*s))
            .collect();
        let new = lines.iter().filter(|l| fresh.contains(&l.0)).count();
        lines.iter().for_each(|l| {
            seen.insert(l.0.clone());
        });
        let mut jump: f64 = 0.0;
        for (s, cells) in &lines {
            for (k, c) in cells.iter().enumerate().skip(1) {
                let Some(v) = std::str::from_utf8(c)
                    .ok()
                    .and_then(|c| c.parse::<f64>().ok())
                else {
                    continue;
                };
                let key = (s.clone(), k, cells[0].clone());
                if let Some(p) = last.insert(key, v) {
                    let m = max_jump.entry(k).or_insert(0.0);
                    let ratio = if *m > 0.0 { (v - p).abs() / *m } else { 0.0 };
                    jump = jump.max(ratio);
                    if fi < files.len() {
                        *m = m.max((v - p).abs());
                    }
                }
            }
        }
        if fi >= 20 && fi < files.len() {
            normal_new.push(new);
            normal_jump = normal_jump.max(jump);
        }
        if fi >= files.len() {
            flags.push((name.clone(), new, jump));
        }
    }
    let took = ms(t);
    normal_new.sort_unstable();
    println!(
        "\n### Drift checks at ingest ({took:.1} ms for {} reports)\n- normal reports after the first 20: lines on a new skeleton median {}, max {} (a new link); largest jump against a column's biggest earlier move: {normal_jump:.2}x",
        all.len(),
        normal_new[normal_new.len() / 2],
        normal_new.last().unwrap()
    );
    for (n, new, jump) in flags {
        println!("- {n}: {new} lines on a new skeleton, largest jump {jump:.1}x");
    }
}

fn consistency() {
    // Five csv exports left stale (one cell each); the md/csv cell diff
    // finds them.
    let mut files = exports(100);
    let mut stale = Vec::new();
    for k in [3, 27, 51, 70, 98] {
        let i = k * 3;
        let csv = String::from_utf8(files[i].1.clone()).unwrap();
        let line = csv.lines().nth(1).unwrap();
        let cells: Vec<&str> = line.split(',').collect();
        let mut c2 = cells.clone();
        let bumped = format!("{}9", cells[2]);
        c2[2] = &bumped;
        files[i].1 = csv.replacen(line, &c2.join(","), 1).into_bytes();
        stale.push(files[i].0.clone());
    }
    let t = Instant::now();
    let mut flagged = Vec::new();
    for tri in files.chunks(3) {
        let (csv, md) = (&tri[0], &tri[2]);
        let a: Vec<Vec<u8>> = ir::lex_lines(&csv.0, &csv.1)
            .into_iter()
            .flat_map(|l| l.1)
            .collect();
        let b: Vec<Vec<u8>> = ir::lex_lines(&md.0, &md.1)
            .into_iter()
            .flat_map(|l| l.1)
            .collect();
        let changed: usize = similar::capture_diff_slices(similar::Algorithm::Myers, &a, &b)
            .iter()
            .map(|op| match op {
                similar::DiffOp::Replace { old_len, .. } => *old_len,
                similar::DiffOp::Delete { old_len, .. } => *old_len,
                _ => 0,
            })
            .sum();
        if changed > 0 {
            flagged.push(csv.0.clone());
        }
    }
    let took = ms(t);
    let hits = flagged.iter().filter(|f| stale.contains(f)).count();
    println!(
        "\n### Export consistency (100 md reports with csv exports, 5 left stale)\n- flagged {} csv files in {took:.1} ms: {hits} of 5 stale found, {} false alarms",
        flagged.len(),
        flagged.len() - hits
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let names = [
        "reports-drift",
        "reports-random",
        "reports-30",
        "exports",
        "tables",
        "texts",
    ];
    let want = |n: &str| args.is_empty() || args.iter().any(|a| a == n);
    for n in names.iter().filter(|n| want(n)) {
        sizes(n, &corpus(n));
    }
    for d in args.iter().filter(|a| a.starts_with("dir:")) {
        sizes("folder", &corpus(d));
    }
    if want("benefits") {
        let drifted = corpus("reports-drift");
        query(&drifted);
        propagate(&drifted);
        drift(&drifted);
        consistency();
    }
}
