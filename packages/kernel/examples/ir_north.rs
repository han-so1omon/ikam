//! E222: the E216 setting on the north corpora.
//!
//! `cargo run --release --example ir_north`
//!
//! The bench's north score (E099) stores three mixed-type corpora (the
//! fixture cases' documents; the same 24 texts as md, html, docx and pdf;
//! the same records in six formats) in one repository each. Before the IR
//! goes behind a put or repack option: what would it do there? Each corpus
//! is stored by the kernel as it is, and with its text files (md, html,
//! json, jsonl, csv, log, py) replaced by one IR pack of them under E216's
//! setting (untuned), office and pdf files left to the kernel. Also the text
//! files alone, in the kernel and as the IR pack in the kernel, to tell the
//! IR's own gain from what the kernel shares across formats.

mod ir_common;

use std::path::Path;
use std::time::Instant;

use ikam_kernel::ir::{self, Options};
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

fn main() {
    let kernel = Path::new(env!("CARGO_MANIFEST_DIR"));
    let keep_cases = [
        "md", "pdf", "docx", "xlsx", "pptx", "json", "jsonl", "log", "py", "html",
    ];
    let corpora = [
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
    println!(
        "| corpus | files (text) | kernel, every file | text as one IR pack | text files alone: kernel / IR pack in the kernel | encode s |"
    );
    println!("|---|---:|---:|---:|---:|---:|");
    let mut logs = [0.0f64; 2];
    for (name, files) in &corpora {
        let (text, rest): (Files, Files) = files.iter().cloned().partition(|(n, _)| is_text(n));
        let all = stored(&kernel_store(files));
        let t = Instant::now();
        let parts = ir::encode(&text, setting);
        let enc = t.elapsed().as_secs_f64();
        assert!(ir::decode(&parts).unwrap() == text);
        let pack = ir::pack(&parts);
        let mut mixed = rest.clone();
        mixed.push(("ir/pack".to_string(), pack.clone()));
        let mixed = stored(&kernel_store(&mixed));
        let text_k = stored(&kernel_store(&text));
        let text_ir = stored(&kernel_store(&vec![("ir/pack".to_string(), pack)]));
        logs[0] += (mixed as f64 / all as f64).ln();
        logs[1] += (text_ir as f64 / text_k as f64).ln();
        println!(
            "| {name} | {} ({}) | {all} B | {mixed} B ({:.3}) | {text_k} B / {text_ir} B ({:.3}) | {enc:.1} |",
            files.len(),
            text.len(),
            mixed as f64 / all as f64,
            text_ir as f64 / text_k as f64
        );
    }
    let n = corpora.len() as f64;
    println!(
        "\nGeometric mean against the kernel: whole corpus {:.3}, text files alone {:.3}.",
        (logs[0] / n).exp(),
        (logs[1] / n).exp()
    );
}
