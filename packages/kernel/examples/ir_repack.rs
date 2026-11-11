//! E223: IR packs in the store, judged by the store.
//!
//! `cargo run --release --example ir_repack [corpus ...]`
//!
//! E222 measured an IR pack as one more file beside the corpus. Here the
//! text files (md, html, json, jsonl, csv, log, py) go into the store as
//! members of one IR pack (`Repo::put_ir`, E216's setting untuned): the pack
//! is content, each file the fact `ir_member(pack, index)`, so repack keeps
//! the pack and drops the files' own bytes, and every file still reads back
//! by its id. Each corpus is stored both ways, and the judge keeps the
//! smaller store. For each: bytes after repack, repack time, and the time to
//! read one text file (its pack decoded) and a second one from the same pack.

mod ir_common;

use std::path::Path;
use std::time::Instant;

use ikam_kernel::ir::Options;
use ikam_kernel::{Id, Kind, MemStore, Object, Repo, TreeEntry};
use ir_common::*;

fn walk(root: &Path, keep: &dyn Fn(&Path) -> bool) -> Files {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if keep(&path) {
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                out.push((name, std::fs::read(&path).unwrap()));
            }
        }
    }
    out.sort();
    out
}

const TEXT: [&str; 7] = ["md", "html", "json", "jsonl", "csv", "log", "py"];

fn is_text(name: &str) -> bool {
    TEXT.iter().any(|e| name.ends_with(&format!(".{e}")))
}

/// The corpus in one repository, its text files as themselves, as one IR
/// pack under `o`, or as IR blocks of `per` files: bytes stored after repack, repack ms,
/// and ms to read the middle text file, then the next one. `short`: short
/// derivation records (`short_records`, E104).
#[derive(Clone, Copy)]
enum Mode {
    Files,
    Whole,
    Blocks(usize),
}

fn store(files: &Files, o: Options, mode: Mode, short: bool) -> (usize, f64, f64, f64) {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = ARCHIVE_READ_WEIGHT;
    repo.short_records = short;
    let text: Files = files.iter().filter(|(n, _)| is_text(n)).cloned().collect();
    let mut ids: Vec<Id> = Vec::with_capacity(files.len());
    let ir_ids = match mode {
        _ if text.is_empty() => None,
        Mode::Files => None,
        Mode::Whole => Some(repo.put_ir(&text, o).unwrap()),
        Mode::Blocks(per) => Some(repo.put_ir_blocks(&text, o, per).unwrap()),
    };
    let mut next_ir = 0;
    for (name, bytes) in files {
        match &ir_ids {
            Some(t) if is_text(name) => {
                ids.push(t[next_ir]);
                next_ir += 1;
            }
            _ => ids.push(repo.put_content(bytes).unwrap().id),
        }
    }
    let tree: Vec<TreeEntry> = files
        .iter()
        .zip(&ids)
        .map(|((n, _), id)| TreeEntry {
            name: n.replace('/', "|"),
            kind: Kind::File,
            id: *id,
        })
        .collect();
    let tree = repo.put(&Object::tree(tree).unwrap()).unwrap();
    repo.commit("main", tree, "").unwrap();
    let t = Instant::now();
    repo.repack().unwrap();
    let repack = ms(t);
    // Read in a fresh thread so no pack is cached from the repack.
    let reads = std::thread::scope(|s| {
        s.spawn(|| {
            let mid: Vec<&(String, Vec<u8>)> = files.iter().filter(|(n, _)| is_text(n)).collect();
            if mid.len() < 2 {
                return (0.0, 0.0);
            }
            let (a, b) = (mid[mid.len() / 2], mid[mid.len() / 2 + 1]);
            let t = Instant::now();
            assert!(repo.read_content(&Id::of_content(&a.1)).unwrap() == a.1);
            let first = ms(t);
            let t = Instant::now();
            assert!(repo.read_content(&Id::of_content(&b.1)).unwrap() == b.1);
            (first, ms(t))
        })
        .join()
        .unwrap()
    });
    assert!(
        files
            .iter()
            .all(|(_, b)| repo.read_content(&Id::of_content(b)).unwrap() == *b)
    );
    assert!(repo.fsck().unwrap().is_empty());
    (stored(&repo), repack, reads.0, reads.1)
}

fn main() {
    let kernel = Path::new(env!("CARGO_MANIFEST_DIR"));
    let keep_cases = [
        "md", "pdf", "docx", "xlsx", "pptx", "json", "jsonl", "log", "py", "html",
    ];
    let mut corpora = vec![
        (
            "north/cases",
            walk(&kernel.join("../../tests/fixtures/cases"), &|p| {
                !p.to_string_lossy().contains("_image_tools")
                    && p.extension()
                        .is_some_and(|e| keep_cases.iter().any(|k| e == *k))
            }),
        ),
        (
            "north/multiformat",
            walk(&kernel.join("fixtures/multiformat"), &|p| {
                p.extension().is_some_and(|e| e != "txt")
            }),
        ),
        (
            "north/tables",
            walk(&kernel.join("fixtures/tables"), &|p| {
                p.extension().is_some_and(|e| e != "txt" && e != "py")
            }),
        ),
    ];
    corpora.push(("reports-drift", corpus("reports-drift")));
    corpora.push(("exports", corpus("exports")));
    corpora.push((
        "cases text",
        walk(&kernel.join("../../tests/fixtures/cases"), &|p| {
            p.to_str().is_some_and(is_text)
        }),
    ));
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
    let args: Vec<String> = std::env::args().skip(1).collect();
    println!(
        "| corpus | files (text) | files | short records: files | IR pack (read ms first / next) | IR blocks of 50 (read ms) | IR blocks of 20 (read ms) | judged |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|");
    let mut logs = [0.0f64; 3];
    let mut north = 0;
    for (name, files) in corpora
        .iter()
        .filter(|(n, _)| args.is_empty() || args.iter().any(|a| n.contains(a.as_str())))
    {
        let text = files.iter().filter(|(n, _)| is_text(n)).count();
        let plain = store(files, setting, Mode::Files, false);
        let f = store(files, setting, Mode::Files, true);
        let w = store(files, setting, Mode::Whole, true);
        let b50 = store(files, setting, Mode::Blocks(50), true);
        let b20 = store(files, setting, Mode::Blocks(20), true);
        let r = |x: usize| x as f64 / f.0 as f64;
        let judged = [w.0, b50.0, b20.0, f.0].into_iter().min().unwrap();
        if name.starts_with("north/") {
            logs[0] += (f.0 as f64 / plain.0 as f64).ln();
            logs[1] += (judged as f64 / plain.0 as f64).ln();
            logs[2] += r(judged).ln();
            north += 1;
        }
        println!(
            "| {name} | {} ({text}) | {} B | {} B | {:.3} ({:.1} / {:.2}) | {:.3} ({:.1}) | {:.3} ({:.1}) | {:.3} |",
            files.len(),
            plain.0,
            f.0,
            r(w.0),
            w.2,
            w.3,
            r(b50.0),
            b50.2,
            r(b20.0),
            b20.2,
            r(judged)
        );
    }
    if north > 0 {
        let n = f64::from(north);
        println!(
            "\nNorth corpora, geometric mean against the files with plain records: short records {:.3}; judged with IR {:.3} ({:.3} of short records alone).",
            (logs[0] / n).exp(),
            (logs[1] / n).exp(),
            (logs[2] / n).exp()
        );
    }
    println!("\nRatios against the files with short records; judged: the smallest of the four.");
}
