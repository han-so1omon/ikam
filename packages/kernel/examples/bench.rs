//! Reproducible storage benchmark: one line per corpus, or JSON lines.
//!
//!   cargo run --release --example bench            # table
//!   cargo run --release --example bench -- --json  # one JSON object per corpus
//!   cargo run --release --example bench -- pdf md  # only corpora named so
//!
//! This is the fixed experiment command (docs/experiments/README.md): vary
//! code, never flags. The last line is the score: the geometric mean over
//! corpora of kernel bytes / trained-dictionary baseline bytes (lower is
//! better). Inexact or fsck-failing results abort the run.
//!
//! Every corpus is built deterministically from this repository. Each run
//! ingests the corpus snapshot by snapshot (one commit each), repacks, then
//! verifies every file of every snapshot byte-for-byte and runs fsck. The
//! baselines are file-level dedup + zstd per file, and file-level dedup +
//! zstd with a dictionary trained on the corpus (dictionary size included).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use ikam_kernel::{Id, Kind, MemStore, Object, Repo, Store, TreeEntry};

type Snapshot = Vec<(String, Vec<u8>)>;
type Corpus = (&'static str, fn() -> Vec<Snapshot>);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn walk(root: &Path, keep: &dyn Fn(&Path) -> bool) -> Snapshot {
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

fn fixtures(exts: &[&str]) -> Vec<Snapshot> {
    let root = repo_root().join("tests/fixtures/cases");
    vec![walk(&root, &|p| {
        !p.to_string_lossy().contains("_image_tools")
            && p.extension().is_some_and(|e| exts.iter().any(|x| e == *x))
    })]
}

/// 21 revisions of a spec: one appended phrase per revision, a new section
/// every fifth.
fn synthetic_history() -> Vec<Snapshot> {
    let base =
        std::fs::read_to_string(repo_root().join("docs/ikam/ikam-sheet-specification.md")).unwrap();
    let mut versions = vec![base];
    for i in 1..=20usize {
        let mut lines: Vec<String> = versions.last().unwrap().lines().map(String::from).collect();
        let k = (i * 37) % lines.len();
        lines[k].push_str(&format!(" (edit {i})"));
        let mut v = lines.join("\n") + "\n";
        if i % 5 == 0 {
            v.push_str(&format!(
                "\n## Added section {i}\nNew text for revision {i}.\n"
            ));
        }
        versions.push(v);
    }
    versions
        .into_iter()
        .map(|v| vec![("doc.md".to_string(), v.into_bytes())])
        .collect()
}

/// This repository's history before the kernel existed (fixed commits).
fn repo_history() -> Vec<Snapshot> {
    let root = repo_root();
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap()
    };
    let commits = String::from_utf8(git(&["rev-list", "--reverse", "6cd6921"]).stdout).unwrap();
    commits
        .lines()
        .map(|c| {
            let dir = tempfile::tempdir().unwrap();
            let cmd = format!(
                "git archive {c} -- docs packages/ikam packages/modelado packages/interacciones | tar -x -C {}",
                dir.path().display()
            );
            assert!(Command::new("sh").args(["-c", &cmd]).current_dir(&root).status().unwrap().success());
            walk(dir.path(), &|_| true)
        })
        .collect()
}

/// Invoices from one generator: long shared boilerplate, short fields.
fn invoices() -> Vec<Snapshot> {
    let one = |n: u32| {
        (0..20u32)
            .map(|k| {
                let v = (n.wrapping_mul(2654435761) ^ k.wrapping_mul(40503)) % 100_000;
                format!("<section id=\"{k}\"><label>Line item {k}: standard terms and conditions apply; see master agreement clause {k}.</label><value>{v:x}</value></section>\n")
            })
            .collect::<String>()
            .into_bytes()
    };
    vec![
        (0..30)
            .map(|n| (format!("inv{n:03}.xml"), one(n)))
            .collect(),
    ]
}

fn stored(repo: &Repo<MemStore>) -> usize {
    repo.store()
        .ids()
        .unwrap()
        .iter()
        .map(|id| repo.store().read(id).unwrap().len())
        .sum()
}

/// Stored objects by encoding tag ("B"/"Z"/"Y" content, "T" tree, "C"
/// commit, "D" derivation, "L" claim): `{"T":[count,bytes],...}`.
fn by_kind(repo: &Repo<MemStore>) -> String {
    let mut kinds: BTreeMap<char, (usize, usize)> = BTreeMap::new();
    for id in repo.store().ids().unwrap() {
        let encoded = repo.store().read(&id).unwrap();
        let k = kinds
            .entry(*encoded.first().unwrap_or(&b'?') as char)
            .or_default();
        (k.0, k.1) = (k.0 + 1, k.1 + encoded.len());
    }
    let parts: Vec<String> = kinds
        .iter()
        .map(|(k, (n, b))| format!("\"{k}\":[{n},{b}]"))
        .collect();
    format!("{{{}}}", parts.join(","))
}

/// Nested trees, one per directory, as a real checkout would be stored:
/// unchanged directories are shared across snapshots by id.
fn tree(repo: &mut Repo<MemStore>, files: &[(&str, &[u8])]) -> Id {
    let mut dirs: BTreeMap<&str, Vec<(&str, &[u8])>> = BTreeMap::new();
    let mut entries = Vec::new();
    for &(path, bytes) in files {
        match path.split_once('/') {
            Some((dir, rest)) => dirs.entry(dir).or_default().push((rest, bytes)),
            None => entries.push(TreeEntry {
                name: path.into(),
                kind: Kind::File,
                id: repo.put_content(bytes).unwrap().id,
            }),
        }
    }
    for (dir, files) in dirs {
        entries.push(TreeEntry {
            name: dir.into(),
            kind: Kind::Tree,
            id: tree(repo, &files),
        });
    }
    repo.put(&Object::tree(entries).unwrap()).unwrap()
}

fn commit(repo: &mut Repo<MemStore>, snapshot: &Snapshot) {
    let files: Vec<(&str, &[u8])> = snapshot
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    let tree = tree(repo, &files);
    repo.commit("main", tree, "").unwrap();
}

/// (file-level dedup + zstd per file, file-level dedup + trained dictionary).
fn baselines(snapshots: &[Snapshot]) -> (usize, usize) {
    let unique: BTreeMap<Id, &Vec<u8>> = snapshots
        .iter()
        .flatten()
        .map(|(_, b)| (Id::of_content(b), b))
        .collect();
    let files: Vec<&Vec<u8>> = unique.into_values().collect();
    let per_file = files
        .iter()
        .map(|b| zstd::bulk::compress(b, 3).unwrap().len())
        .sum();
    let total: usize = files.iter().map(|b| b.len()).sum();
    let dict =
        zstd::dict::from_samples(&files, (total / 10).clamp(4096, 112_640)).unwrap_or_default();
    let mut c = zstd::bulk::Compressor::with_dictionary(3, &dict).unwrap();
    let with_dict = dict.len()
        + files
            .iter()
            .map(|b| c.compress(b).unwrap().len())
            .sum::<usize>();
    (per_file, with_dict)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let only: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let corpora: Vec<Corpus> = vec![
        ("md", || fixtures(&["md"])),
        ("pdf", || fixtures(&["pdf"])),
        ("office", || fixtures(&["xlsx", "docx", "pptx"])),
        ("invoices", invoices),
        ("synthetic-history", synthetic_history),
        ("repo-history", repo_history),
    ];
    let (mut log_ratio, mut n, mut total_ms) = (0.0, 0, 0);
    for (name, build) in corpora {
        if !only.is_empty() && !only.iter().any(|o| *o == name) {
            continue;
        }
        let snapshots = build();
        let input: usize = snapshots.iter().flatten().map(|(_, b)| b.len()).sum();
        let mut repo = Repo::new(MemStore::default());
        let t = Instant::now();
        snapshots.iter().for_each(|s| commit(&mut repo, s));
        let (ingest, ingest_ms) = (stored(&repo), t.elapsed().as_millis());
        let t = Instant::now();
        repo.repack().unwrap();
        let (repack, repack_ms) = (stored(&repo), t.elapsed().as_millis());
        let exact = snapshots
            .iter()
            .flatten()
            .all(|(_, b)| repo.read_content(&Id::of_content(b)).unwrap() == *b);
        let fsck = repo.fsck().unwrap().is_empty();
        let (zstd_files, zstd_dict) = baselines(&snapshots);
        let ratio = repack as f64 / zstd_dict as f64;
        if json {
            println!(
                "{{\"corpus\":\"{name}\",\"input\":{input},\"ingest\":{ingest},\"repack\":{repack},\"ingest_ms\":{ingest_ms},\"repack_ms\":{repack_ms},\"zstd_files\":{zstd_files},\"zstd_dict\":{zstd_dict},\"vs_dict\":{ratio:.4},\"exact\":{exact},\"fsck\":{fsck},\"by_kind\":{}}}",
                by_kind(&repo)
            );
        } else {
            println!(
                "{name:<18} input={input:>9} ingest={ingest:>8} repack={repack:>8} ({:.3})  zstd/file={zstd_files:>8} zstd+dict={zstd_dict:>8}  vs_dict={ratio:.3}  {ingest_ms}+{repack_ms} ms  exact={exact} fsck={fsck}",
                repack as f64 / input as f64
            );
        }
        // Hard gates: an inexact or fsck-failing result is a failure, not a score.
        assert!(exact && fsck, "{name}: verification failed");
        (log_ratio, n, total_ms) = (
            log_ratio + ratio.ln(),
            n + 1,
            total_ms + ingest_ms + repack_ms,
        );
    }
    // The experiment score: geometric mean of kernel bytes / trained-dictionary
    // baseline bytes across corpora (every corpus weighs the same; lower is
    // better; below 1.0 beats the baseline).
    let score = (log_ratio / n.max(1) as f64).exp();
    if json {
        println!(
            "{{\"corpus\":\"_summary\",\"corpora\":{n},\"score\":{score:.4},\"total_ms\":{total_ms}}}"
        );
    } else {
        println!(
            "score (geometric mean of repack / zstd+dict) = {score:.4} over {n} corpora, {total_ms} ms"
        );
    }
}
