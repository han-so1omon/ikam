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

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use ikam_kernel::{
    Arg, Edge, Id, Kind, MemStore, Node, Object, Repo, Store, Target, TreeEntry, node_key,
};

type Snapshot = Vec<(String, Vec<u8>)>;
/// (name, snapshots, whether each snapshot is one graph's edge list).
type Corpus = (&'static str, fn() -> Vec<Snapshot>, bool);

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

/// This repository's dependency graph at each commit up to the graph layer
/// (a fixed range): one node per file or module referenced, with edges
/// "imports" (Python), "links" (markdown, resolved to a path) and "uses"
/// (Rust `mod` and `use crate::`). Imports may form cycles. Each version is
/// one file, the sorted edge list `source\tlabel\ttarget\n`: the baseline
/// compresses it; the kernel stores it as a graph (`graph_of`).
fn repo_graph() -> Vec<Snapshot> {
    let root = repo_root();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    let pattern = r"^\s*(from\s+[A-Za-z_][\w.]*\s+import|import\s+[A-Za-z_][\w.]*)|\]\([^)#[:space:]]+|^\s*(pub\s+)?mod\s+\w+;|use crate::\w+";
    let mut out = Vec::new();
    for c in git(&["rev-list", "--reverse", "0ca74e4"]).lines() {
        let found = git(&[
            "grep",
            "-I",
            "-o",
            "-E",
            pattern,
            c,
            "--",
            "packages",
            "docs",
            ":!packages/narraciones",
        ]);
        let edges: BTreeSet<String> = found
            .lines()
            .filter_map(|l| edge(l.split_once(':')?.1))
            .collect();
        out.push(vec![(
            "graph.tsv".to_string(),
            edges.into_iter().collect::<String>().into_bytes(),
        )]);
    }
    out
}

/// One edge line from a `path:match` grep result.
fn edge(line: &str) -> Option<String> {
    let (path, m) = line.split_once(':')?;
    let m = m.trim();
    let (label, target) = if let Some(link) = m.strip_prefix("](") {
        if link.contains("://") || link.starts_with("mailto:") {
            return None;
        }
        let mut parts: Vec<&str> = path.split('/').collect();
        parts.pop();
        for seg in link.split('/') {
            match seg {
                "." | "" => {}
                ".." => drop(parts.pop()),
                s => parts.push(s),
            }
        }
        ("links", parts.join("/"))
    } else if let Some(module) = m.strip_prefix("from ") {
        (
            "imports",
            format!("py:{}", module.split_whitespace().next()?),
        )
    } else if let Some(module) = m.strip_prefix("import ") {
        ("imports", format!("py:{}", module.trim()))
    } else {
        let name = m.trim_end_matches(';').rsplit([' ', ':']).next()?;
        ("uses", format!("rs:{name}"))
    };
    Some(format!("{path}\t{label}\t{target}\n"))
}

/// The graph an edge list describes: every source and target is a node.
fn graph_of(text: &[u8]) -> Vec<Node> {
    let mut nodes: BTreeMap<String, BTreeSet<Edge>> = BTreeMap::new();
    for line in std::str::from_utf8(text).unwrap().lines() {
        let mut f = line.split('\t');
        let (src, label, dst) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
        nodes.entry(dst.to_string()).or_default();
        nodes.entry(src.to_string()).or_default().insert(Edge {
            to: node_key(dst),
            label: label.into(),
        });
    }
    nodes
        .into_iter()
        .map(|(label, edges)| Node {
            label,
            target: Target::None,
            edges: edges.into_iter().collect(),
        })
        .collect()
}

/// A priced proposal: (function file, argument file or "-", output id,
/// output bytes, decode work).
type Candidate<'a> = (&'a str, &'a str, Id, f64, f64);

/// Offer the kernel the reconstruction functions priced in
/// `functions/proposals.txt` (written by `examples/programs.rs`) whose output
/// is a file of this corpus. A file may have several forms (a program for
/// the shared interpreter, or the same program compiled to its own module).
/// The proposer offers the form with the lowest estimated cost: function
/// bytes (the interpreter's shared by the files using it) + argument bytes +
/// read weight x decode work, and only if that is below the output's size.
/// The kernel verifies what is offered; repack keeps it only if it pays.
fn propose_programs(repo: &mut Repo<MemStore>, snapshots: &[Snapshot]) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("functions");
    let files: BTreeMap<Id, &Vec<u8>> = snapshots
        .iter()
        .flatten()
        .map(|(_, b)| (Id::of_content(b), b))
        .collect();
    let proposals = std::fs::read_to_string(dir.join("proposals.txt")).unwrap_or_default();
    let found: Vec<Candidate> = proposals
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .map(|f| {
            (
                f[0],
                f[1],
                f[2].parse().unwrap(),
                f[3].parse().unwrap(),
                f[4].parse().unwrap(),
            )
        })
        .filter(|p: &Candidate| files.contains_key(&p.2))
        .collect();
    let size = |path: &str| std::fs::metadata(dir.join(path)).map_or(0.0, |m| m.len() as f64);
    let users = |func: &str| found.iter().filter(|p| p.0 == func).count() as f64;
    let cost = |p: &Candidate| {
        size(p.0) / users(p.0)
            + if p.1 == "-" { 0.0 } else { size(p.1) }
            + ikam_kernel::DEFAULT_READ_WEIGHT * p.4
    };
    let mut best: BTreeMap<Id, (f64, &Candidate)> = BTreeMap::new();
    for p in &found {
        let c = cost(p);
        if c < p.3 && best.get(&p.2).is_none_or(|b| c < b.0) {
            best.insert(p.2, (c, p));
        }
    }
    for (out, (_, p)) in best {
        let func = repo
            .put_content(&std::fs::read(dir.join(p.0)).unwrap())
            .unwrap()
            .id;
        let mut args = Vec::new();
        if p.1 != "-" {
            args.push(Arg::Whole(
                repo.put_content(&std::fs::read(dir.join(p.1)).unwrap())
                    .unwrap()
                    .id,
            ));
        }
        assert!(
            repo.put_derivation(files[&out], func, args)
                .unwrap()
                .is_some()
        );
    }
}

/// The committed embedding proposal (`semantic/order.txt`, written by
/// `semantic/embed.py`): content ids in a semantic order. Repack uses it as
/// one grouping proposal among several, so no model runs here.
fn proposed_order() -> Vec<Id> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("semantic/order.txt");
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.parse().unwrap())
        .collect()
}

/// Data that is the output of a small procedure: where a reconstruction
/// function (a program) can be far smaller than any compression of its
/// output. Deterministic, integer-only.
fn generated() -> Vec<Snapshot> {
    let mut s = 0x2545_f491_4f6c_dd1du64;
    let mut lcg = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as usize
    };
    let mut metrics = String::from("minute,requests,errors,p50_ms,total\n");
    let mut total = 0usize;
    for i in 0..4000usize {
        let (req, err, p50) = (1000 + (i * 37) % 251, (i * i) % 7, 20 + (i % 60));
        total += req;
        metrics.push_str(&format!(
            "{},{req},{err},{p50},{total}\n",
            1_700_000_000 + 60 * i
        ));
    }
    let table: String = (1..=99usize)
        .map(|a| {
            (1..=99usize)
                .map(|b| format!("{:>5}", a * b))
                .collect::<String>()
                + "\n"
        })
        .collect();
    let paths = [
        "/api/users",
        "/api/orders",
        "/health",
        "/static/app.js",
        "/api/search",
    ];
    let log: String = (0..6000usize)
        .map(|i| {
            let status = [200, 200, 200, 304, 404, 500][lcg() % 6];
            format!(
                "2026-10-06T12:{:02}:{:02}Z req={:06} {} {status} {}ms\n",
                (i / 60) % 60,
                i % 60,
                100_000 + i,
                paths[lcg() % paths.len()],
                5 + lcg() % 200
            )
        })
        .collect();
    let (w, h) = (320usize, 240usize);
    let mut pgm = format!("P5\n{w} {h}\n255\n").into_bytes();
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            // Fixed point, 12 fractional bits: c = (-2.2 + 3.2 x/w, -1.2 + 2.4 y/h).
            let (cr, ci) = (-9011 + x * 13107 / w as i64, -4915 + y * 9830 / h as i64);
            let (mut zr, mut zi, mut n) = (0i64, 0i64, 0u8);
            while n < 255 && zr * zr + zi * zi <= 4 << 24 {
                (zr, zi) = (((zr * zr - zi * zi) >> 12) + cr, ((2 * zr * zi) >> 12) + ci);
                n += 1;
            }
            pgm.push(n);
        }
    }
    let mut primes = Vec::new();
    let mut n = 2usize;
    while primes.len() < 20_000 {
        if primes
            .iter()
            .take_while(|&&p| p * p <= n)
            .all(|p| !n.is_multiple_of(*p))
        {
            primes.push(n);
        }
        n += 1;
    }
    let primes: String = primes.iter().map(|p| format!("{p}\n")).collect();
    vec![vec![
        ("metrics.csv".to_string(), metrics.into_bytes()),
        ("multiplication.txt".to_string(), table.into_bytes()),
        ("server.log".to_string(), log.into_bytes()),
        ("mandelbrot.pgm".to_string(), pgm),
        ("primes.txt".to_string(), primes.into_bytes()),
    ]]
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

fn commit(repo: &mut Repo<MemStore>, snapshot: &Snapshot, graph: bool) {
    if graph {
        let root = repo.put_graph(graph_of(&snapshot[0].1)).unwrap();
        let entry = TreeEntry {
            name: "graph".into(),
            kind: Kind::Graph,
            id: root,
        };
        let tree = repo.put(&Object::tree(vec![entry]).unwrap()).unwrap();
        repo.commit("main", tree, "").unwrap();
        return;
    }
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
    // Not a scoring option: writes the generated corpus out as input for an
    // offline program writer (functions/write_programs.py), then exits.
    if let Some(i) = args.iter().position(|a| a == "--dump-generated") {
        let dir = Path::new(&args[i + 1]);
        std::fs::create_dir_all(dir).unwrap();
        for (name, bytes) in generated().concat() {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        return;
    }
    let only: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| !a.starts_with("--") && (*i == 0 || args[i - 1] != "--dump-generated"))
        .map(|(_, a)| a)
        .collect();
    let corpora: Vec<Corpus> = vec![
        ("md", || fixtures(&["md"]), false),
        ("pdf", || fixtures(&["pdf"]), false),
        ("office", || fixtures(&["xlsx", "docx", "pptx"]), false),
        ("invoices", invoices, false),
        ("synthetic-history", synthetic_history, false),
        ("repo-history", repo_history, false),
        ("repo-graph", repo_graph, true),
        ("generated", generated, false),
    ];
    let (mut log_ratio, mut n, mut total_ms) = (0.0, 0, 0);
    for (name, build, graph) in corpora {
        if !only.is_empty() && !only.iter().any(|o| *o == name) {
            continue;
        }
        let snapshots = build();
        let input: usize = snapshots.iter().flatten().map(|(_, b)| b.len()).sum();
        let mut repo = Repo::new(MemStore::default());
        let t = Instant::now();
        snapshots.iter().for_each(|s| commit(&mut repo, s, graph));
        let (ingest, ingest_ms) = (stored(&repo), t.elapsed().as_millis());
        propose_programs(&mut repo, &snapshots);
        let t = Instant::now();
        repo.proposed_order = proposed_order();
        repo.repack().unwrap();
        let (repack, repack_ms) = (stored(&repo), t.elapsed().as_millis());
        let exact = match graph {
            // Every version's graph, read back from its commit, is the one stored.
            true => repo
                .log(repo.resolve("main").unwrap())
                .unwrap()
                .iter()
                .rev()
                .zip(&snapshots)
                .all(|((_, c), s)| {
                    let Object::Tree(entries) = repo.get(&c.tree).unwrap() else {
                        return false;
                    };
                    repo.graph_nodes(&entries[0].id).unwrap() == graph_of(&s[0].1)
                }),
            false => snapshots
                .iter()
                .flatten()
                .all(|(_, b)| repo.read_content(&Id::of_content(b)).unwrap() == *b),
        };
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
