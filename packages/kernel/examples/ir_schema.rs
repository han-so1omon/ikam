//! E202: read the IR's columns out as a schema, and find the cells that
//! break it.
//!
//! `cargo run --release --example ir_schema`
//!
//! `predict` (E200) learns, to save bytes, which column of a line another
//! column follows from. `ir::schema::dependencies` reads that out over a
//! folder: for each line template and slot, the slot it follows from best
//! (counted in file order, so a value that changes once and holds still
//! counts as following). `ir::schema::events` then walks each dependency
//! per key and marks a value that differs from the one before and is back
//! on the next line (a blip) or holds (a change).
//!
//! For each corpus: how many dependencies, and the time. For the drift
//! reports, each one with an example. Then errors are put in: 60 times, a
//! random cell of a random line (one with at least two cells) gets another
//! value its column has elsewhere; the blips found are scored against the
//! cells changed, overall and for the cells in a column that has a
//! dependency. The clean corpus's blips are its false alarms.

mod ir_common;

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use ikam_kernel::ir::{self, schema};
use ir_common::*;

fn show(b: &[u8]) -> String {
    let s = String::from_utf8_lossy(b).replace('\n', "");
    if s.chars().count() > 40 {
        format!("{}...", s.chars().take(40).collect::<String>())
    } else {
        s
    }
}

/// The skeleton with its slots as {0}, {1}, ...
fn template(sk: &[u8]) -> String {
    let segs = ir::segments(sk).unwrap();
    let mut s = String::new();
    for (i, seg) in segs.iter().enumerate() {
        s += &String::from_utf8_lossy(seg).replace('\n', "");
        if i + 1 < segs.len() {
            s += &format!("{{{i}}}");
        }
    }
    if s.chars().count() > 60 {
        format!("{}...", s.chars().take(60).collect::<String>())
    } else {
        s
    }
}

/// A file's lines: skeleton and cells.
type Lexed = Vec<(Vec<u8>, Vec<Vec<u8>>)>;

/// Puts `n` errors in: each one cell of a line with at least two cells
/// takes another value of its column (same skeleton and slot) seen
/// elsewhere. Returns the files and the places changed with their slot.
fn corrupt(files: &Files, n: usize, seed: u64) -> (Files, Vec<(schema::Place, usize)>) {
    let mut lexed: Vec<Lexed> = files
        .iter()
        .map(|(name, b)| ir::lex_lines(name, b))
        .collect();
    // every file rebuilds from its lines
    for (f, l) in files.iter().zip(&lexed) {
        assert!(rebuild(l) == f.1, "{}", f.0);
    }
    let mut column: HashMap<(Vec<u8>, usize), Vec<Vec<u8>>> = HashMap::new();
    let mut lines = Vec::new();
    for (fi, ls) in lexed.iter().enumerate() {
        for (li, (sk, cells)) in ls.iter().enumerate() {
            if cells.len() >= 2 {
                lines.push((fi, li));
                for (k, c) in cells.iter().enumerate() {
                    column.entry((sk.clone(), k)).or_default().push(c.clone());
                }
            }
        }
    }
    let mut rng = Rng(seed);
    let mut done = HashSet::new();
    let mut out = Vec::new();
    while out.len() < n {
        let (fi, li) = lines[rng.below(lines.len() as u64) as usize];
        if !done.insert((fi, li)) {
            continue;
        }
        let (sk, cells) = &lexed[fi][li];
        let k = rng.below(cells.len() as u64) as usize;
        let pool = &column[&(sk.clone(), k)];
        let others: Vec<&Vec<u8>> = pool.iter().filter(|v| **v != cells[k]).collect();
        if others.is_empty() {
            continue;
        }
        let v = others[rng.below(others.len() as u64) as usize].clone();
        lexed[fi][li].1[k] = v;
        out.push(((fi, li), k));
    }
    let files = files
        .iter()
        .zip(&lexed)
        .map(|((n, _), l)| (n.clone(), rebuild(l)))
        .collect();
    (files, out)
}

fn rebuild(lines: &Lexed) -> Vec<u8> {
    let mut out = Vec::new();
    for (sk, cells) in lines {
        let segs = ir::segments(sk).unwrap();
        for (i, s) in segs.iter().enumerate() {
            out.extend_from_slice(s);
            if let Some(c) = cells.get(i) {
                out.extend_from_slice(c);
            }
        }
    }
    out
}

fn main() {
    let (min_rows, min_strength) = (20, 0.9);
    println!(
        "| corpus | lines with 2+ cells | dependencies | ms | blips on the clean corpus | conflicts | changes |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|");
    let names = ["reports-drift", "reports-random", "exports", "tables"];
    for name in names {
        let files = corpus(name);
        let t = Instant::now();
        let deps = schema::dependencies(&files, min_rows, min_strength);
        let ev = schema::events(&files, &deps);
        let ms = t.elapsed().as_secs_f64() * 1e3;
        let rows: usize = files
            .iter()
            .map(|(n, b)| {
                ir::lex_lines(n, b)
                    .iter()
                    .filter(|l| l.1.len() >= 2)
                    .count()
            })
            .sum();
        let count = |k| ev.iter().filter(|e| e.kind == k).count();
        println!(
            "| {name} | {rows} | {} | {ms:.0} | {} | {} | {} |",
            deps.len(),
            count(schema::Kind::Blip),
            count(schema::Kind::Conflict),
            count(schema::Kind::Change)
        );
    }

    let files = corpus("reports-drift");
    let deps = schema::dependencies(&files, min_rows, min_strength);
    println!(
        "\nreports-drift, each dependency (template, key slot to slot, held of seen, keys, example):\n"
    );
    println!("| template | follows | held | keys | example |");
    println!("|---|---|---:|---:|---|");
    for d in &deps {
        println!(
            "| `{}` | {{{}}} to {{{}}} | {} of {} | {} | {} to {} |",
            template(&d.skeleton),
            d.from,
            d.to,
            d.held,
            d.seen,
            d.keys,
            show(&d.example.0),
            show(&d.example.1)
        );
    }
    let ev = schema::events(&files, &deps);
    if let Some(e) = ev.iter().find(|e| e.kind == schema::Kind::Change) {
        let d = &deps[e.dep];
        println!(
            "\nOne change from the history: in report {}, `{}` slot {{{}}} for {} went from {} to {}.",
            files[e.at.0].0,
            template(&d.skeleton),
            d.to,
            show(&e.key),
            show(&e.was),
            show(&e.now)
        );
    }

    let ex = corpus("exports");
    let xdeps = schema::dependencies(&ex, min_rows, min_strength);
    if let Some(e) = schema::events(&ex, &xdeps)
        .iter()
        .find(|e| e.kind == schema::Kind::Conflict)
    {
        println!(
            "\nOne conflict: in {}, {} has slot {{{}}} {} on one row and {} on another.",
            ex[e.at.0].0,
            show(&e.key),
            xdeps[e.dep].to,
            show(&e.was),
            show(&e.now)
        );
    }

    println!(
        "\nErrors put in (60 per corpus, 3 seeds), blips found against them (conflicts are counted apart: the clean corpus has its own):\n"
    );
    println!(
        "| corpus | blips on | errors in a column with such a dependency | found of those | found of all | blips flagged | of them real | false on the clean corpus |"
    );
    println!("|---|---|---:|---:|---:|---:|---:|---:|");
    for (name, strict) in [
        ("reports-drift", 0.0),
        ("reports-drift", 0.97),
        ("exports", 0.0),
        ("exports", 0.97),
    ] {
        let keep = |deps: Vec<schema::Dependency>| -> Vec<schema::Dependency> {
            deps.into_iter()
                .filter(|d| d.strength() >= strict)
                .collect()
        };
        let clean = corpus(name);
        let (mut in_dep, mut found_dep, mut found, mut flagged, mut real, mut all) =
            (0, 0, 0, 0, 0, 0);
        for seed in [1, 2, 3] {
            let (files, errs) = corrupt(&clean, 60, seed);
            // the schema is read from the corrupted files themselves
            let deps = keep(schema::dependencies(&files, min_rows, min_strength));
            let dep_cols: HashSet<(&[u8], usize)> =
                deps.iter().map(|d| (d.skeleton.as_slice(), d.to)).collect();
            let blips: HashSet<(usize, usize)> = schema::events(&files, &deps)
                .into_iter()
                .filter(|e| e.kind == schema::Kind::Blip)
                .map(|e| e.at)
                .collect();
            let lexed: Vec<_> = files.iter().map(|(n, b)| ir::lex_lines(n, b)).collect();
            let err_at: HashSet<(usize, usize)> = errs.iter().map(|e| e.0).collect();
            for &(at, k) in &errs {
                let sk = &lexed[at.0][at.1].0;
                let dep = dep_cols.contains(&(sk.as_slice(), k));
                let hit = blips.contains(&at);
                in_dep += usize::from(dep);
                found_dep += usize::from(dep && hit);
                found += usize::from(hit);
            }
            all += errs.len();
            flagged += blips.len();
            real += blips.iter().filter(|b| err_at.contains(b)).count();
        }
        let deps = keep(schema::dependencies(&clean, min_rows, min_strength));
        let false_clean = schema::events(&clean, &deps)
            .iter()
            .filter(|e| e.kind == schema::Kind::Blip)
            .count();
        let on = if strict > 0.0 {
            format!("dependencies holding {:.0}%+", strict * 100.0)
        } else {
            "every dependency".to_string()
        };
        println!(
            "| {name} | {on} | {in_dep} of {all} | {found_dep} ({:.0}%) | {found} ({:.0}%) | {flagged} | {real} ({:.0}%) | {false_clean} |",
            100.0 * found_dep as f64 / in_dep.max(1) as f64,
            100.0 * found as f64 / all as f64,
            100.0 * real as f64 / flagged.max(1) as f64
        );
    }
}
