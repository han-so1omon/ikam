//! Cross-source sharing (E077), on two corpora: the fixture cases'
//! documents (md, pdf, office, json, code; images left out), and
//! `fixtures/multiformat` (each case's idea.md also rendered to html and
//! docx by pandoc 3.1.3 and to pdf by LibreOffice 24.2: the same text in
//! four formats). Each is stored as one graph against the same files stored
//! as one store per file type. What one store saves
//! over the sum of the per-type stores is what sharing across types is
//! worth to the kernel; the same split for zstd with a trained dictionary
//! is the baseline's.
//!
//! `cargo run --release --example cross [read_weight]` (default: the
//! bench's E069 weight, 48.8; 0.001 prices bytes only).

use std::collections::BTreeMap;
use std::path::Path;

use ikam_kernel::{Id, Kind, MemStore, Object, Repo, Store, TreeEntry};

fn files(dir: &str) -> Vec<(String, Vec<u8>)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
    let keep = [
        "md", "pdf", "docx", "xlsx", "pptx", "json", "jsonl", "log", "py", "html",
    ];
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let ext = path.extension().map(|e| e.to_string_lossy().into_owned());
            if path.is_dir() && !path.ends_with("_image_tools") {
                stack.push(path);
            } else if ext.is_some_and(|e| keep.contains(&e.as_str())) {
                let name = path
                    .strip_prefix(&root)
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

/// The file type a store holds: office formats share one.
fn family(name: &str) -> &'static str {
    match name.rsplit('.').next().unwrap() {
        "md" => "md",
        "pdf" => "pdf",
        "docx" | "xlsx" | "pptx" => "office",
        "json" | "jsonl" => "data",
        "py" => "code",
        "html" => "html",
        _ => "log",
    }
}

/// Bytes the kernel stores for `files` in one repository, after repack.
fn kernel(files: &[&(String, Vec<u8>)], read_weight: f64) -> usize {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = read_weight;
    let entries: Vec<TreeEntry> = files
        .iter()
        .map(|(name, bytes)| TreeEntry {
            name: name.replace('/', "|"),
            kind: Kind::File,
            id: repo.put_content(bytes).unwrap().id,
        })
        .collect();
    let tree = repo.put(&Object::tree(entries).unwrap()).unwrap();
    repo.commit("main", tree, "").unwrap();
    repo.repack().unwrap();
    assert!(
        files
            .iter()
            .all(|(_, b)| repo.read_content(&Id::of_content(b)).unwrap() == *b)
    );
    assert!(repo.fsck().unwrap().is_empty());
    // Stored objects by encoding tag, to stderr: where the bytes are.
    let store = repo.store();
    let mut kinds: BTreeMap<char, (usize, usize)> = BTreeMap::new();
    for id in store.ids().unwrap() {
        let e = store.read(&id).unwrap();
        let k = kinds.entry(e[0] as char).or_default();
        (k.0, k.1) = (k.0 + 1, k.1 + e.len());
    }
    eprintln!("  {} files: {kinds:?}", files.len());
    kinds.values().map(|k| k.1).sum()
}

/// zstd 3 with a dictionary trained on `files` (as the bench's baseline).
fn baseline(files: &[&(String, Vec<u8>)]) -> usize {
    let unique: BTreeMap<Id, &Vec<u8>> =
        files.iter().map(|(_, b)| (Id::of_content(b), b)).collect();
    let files: Vec<&Vec<u8>> = unique.into_values().collect();
    let total: usize = files.iter().map(|b| b.len()).sum();
    let dict =
        zstd::dict::from_samples(&files, (total / 10).clamp(4096, 112_640)).unwrap_or_default();
    let mut c = zstd::bulk::Compressor::with_dictionary(3, &dict).unwrap();
    dict.len()
        + files
            .iter()
            .map(|b| c.compress(b).unwrap().len())
            .sum::<usize>()
}

fn main() {
    let weight: f64 = std::env::args().nth(1).map_or(48.8, |w| w.parse().unwrap());
    for dir in ["../../tests/fixtures/cases", "fixtures/multiformat"] {
        println!("{dir}, read_weight {weight}");
        report(&files(dir), weight);
    }
}

fn report(all: &[(String, Vec<u8>)], weight: f64) {
    let mut by_family: BTreeMap<&str, Vec<&(String, Vec<u8>)>> = BTreeMap::new();
    all.iter()
        .for_each(|f| by_family.entry(family(&f.0)).or_default().push(f));
    println!(
        "{:8} {:>6} {:>10} {:>10} {:>10}",
        "family", "files", "input", "kernel", "baseline"
    );
    let (mut split_k, mut split_b) = (0, 0);
    for (fam, files) in &by_family {
        let input: usize = files.iter().map(|f| f.1.len()).sum();
        let (k, b) = (kernel(files, weight), baseline(files));
        (split_k, split_b) = (split_k + k, split_b + b);
        println!("{fam:8} {:>6} {input:>10} {k:>10} {b:>10}", files.len());
    }
    let refs: Vec<&(String, Vec<u8>)> = all.iter().collect();
    let input: usize = all.iter().map(|f| f.1.len()).sum();
    let (k, b) = (kernel(&refs, weight), baseline(&refs));
    println!(
        "{:8} {:>6} {input:>10} {split_k:>10} {split_b:>10}",
        "split",
        all.len()
    );
    println!("{:8} {:>6} {input:>10} {k:>10} {b:>10}", "one", all.len());
    println!(
        "cross-type saving: kernel {:.1}%, baseline {:.1}%",
        100.0 * (1.0 - k as f64 / split_k as f64),
        100.0 * (1.0 - b as f64 / split_b as f64)
    );
}
