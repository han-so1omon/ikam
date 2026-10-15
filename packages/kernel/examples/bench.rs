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

/// Structured data (E040): exports of this repository's history up to the
/// same pinned commit as `repo_graph`, at every fourth commit and the last.
/// Each snapshot has a full-table export (`inventory.csv`: every file's
/// path, mode, size and blob id), an append-only log (`changes.ndjson`: one
/// JSON object per file change so far, with commit, date, author name,
/// lines added and deleted) and a summary (`summary.json`: files and bytes
/// per top-level directory). Real metadata, not the output of a short
/// A coding agent's session log as it grows (since E055): the kernel's own
/// sources pinned at 0ca74e4, plus a JSONL transcript of an agent reading
/// them. Reads quote files as Read results do (line-numbered, JSON-escaped),
/// whole or as 40-line ranges; searches quote matching lines. Every event
/// carries a random id and its parent's, as real transcripts do. Snapshots
/// every 75 steps; deterministic.
fn transcripts() -> Vec<Snapshot> {
    let root = repo_root();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    let paths: Vec<String> = git(&[
        "ls-tree",
        "-r",
        "--name-only",
        "0ca74e4",
        "packages/kernel/src",
    ])
    .lines()
    .map(String::from)
    .collect();
    let sources: Vec<(String, String)> = paths
        .iter()
        .map(|p| (p.clone(), git(&["show", &format!("0ca74e4:{p}")])))
        .collect();
    let mut s = 0x0123_4567_89ab_cdefu64;
    let mut lcg = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as usize
    };
    let numbered = |text: &str, from: usize, n: usize| -> String {
        text.lines()
            .enumerate()
            .skip(from)
            .take(n)
            .map(|(i, l)| format!("{:>6}\t{l}\n", i + 1))
            .collect()
    };
    let (mut log, mut parent, mut out) = (String::new(), String::from("null"), Vec::new());
    for step in 1..=300 {
        let (path, text) = &sources[lcg() % sources.len()];
        let lines = text.lines().count().max(1);
        let (input, result) = match lcg() % 10 {
            0..=2 => (
                format!("{{\"file_path\":{}}}", json_str(path)),
                numbered(text, 0, lines),
            ),
            3..=7 => {
                let from = lcg() % lines;
                let input = format!(
                    "{{\"file_path\":{},\"offset\":{},\"limit\":40}}",
                    json_str(path),
                    from + 1
                );
                (input, numbered(text, from, 40))
            }
            _ => {
                let line = text.lines().nth(lcg() % lines).unwrap_or("");
                let word = line
                    .split(|c: char| !c.is_alphanumeric() && c != '_')
                    .find(|w| w.len() > 3)
                    .unwrap_or("fn");
                let hits: String = sources
                    .iter()
                    .flat_map(|(p, t)| {
                        t.lines()
                            .enumerate()
                            .filter(|(_, l)| l.contains(word))
                            .map(move |(i, l)| format!("{p}:{}:{l}\n", i + 1))
                    })
                    .take(30)
                    .collect();
                (format!("{{\"pattern\":{}}}", json_str(word)), hits)
            }
        };
        let tool = if input.contains("pattern") {
            "Grep"
        } else {
            "Read"
        };
        for (kind, body) in [
            ("tool_use", format!("\"name\":\"{tool}\",\"input\":{input}")),
            ("tool_result", format!("\"content\":{}", json_str(&result))),
        ] {
            let uuid = format!(
                "{:08x}-{:04x}-{:04x}-{:04x}-{:08x}{:04x}",
                lcg(),
                lcg() & 0xffff,
                lcg() & 0xffff,
                lcg() & 0xffff,
                lcg(),
                lcg() & 0xffff
            );
            log += &format!(
                "{{\"uuid\":\"{uuid}\",\"parentUuid\":{parent},\"type\":\"{kind}\",{body}}}\n"
            );
            parent = format!("\"{uuid}\"");
        }
        if step % 75 == 0 {
            let mut snap: Vec<(String, Vec<u8>)> = sources
                .iter()
                .map(|(p, t)| (p.clone(), t.clone().into_bytes()))
                .collect();
            snap.push(("session.jsonl".to_string(), log.clone().into_bytes()));
            out.push(snap);
        }
    }
    out
}

/// procedure; author emails are left out.
fn structured() -> Vec<Snapshot> {
    let root = repo_root();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        String::from_utf8(out.stdout).unwrap()
    };
    let commits: Vec<String> = git(&["rev-list", "--reverse", "0ca74e4"])
        .lines()
        .map(String::from)
        .collect();
    let (mut log, mut out) = (String::new(), Vec::new());
    for (i, c) in commits.iter().enumerate() {
        let meta = git(&["show", "-s", "--format=%aI%x09%an", c]);
        let (date, author) = meta.trim_end().split_once('\t').unwrap();
        for row in git(&["show", "--numstat", "--format=", c]).lines() {
            let mut f = row.splitn(3, '\t');
            let (added, deleted, path) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
            let num = |n: &str| {
                if n == "-" {
                    "null".to_string()
                } else {
                    n.to_string()
                }
            };
            log += &format!(
                "{{\"commit\":\"{c}\",\"date\":\"{date}\",\"author\":{},\"path\":{},\"added\":{},\"deleted\":{}}}\n",
                json_str(author),
                json_str(path),
                num(added),
                num(deleted)
            );
        }
        if (i + 1) % 4 != 0 && i + 1 != commits.len() {
            continue;
        }
        let mut inventory = String::from("path,mode,size,blob\n");
        let mut dirs: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for row in git(&["ls-tree", "-r", "-l", c]).lines() {
            let (meta, path) = row.split_once('\t').unwrap();
            let f: Vec<&str> = meta.split_whitespace().collect();
            let size: usize = f[3].parse().unwrap_or(0);
            inventory += &format!("{},{},{},{}\n", csv_str(path), f[0], f[3], f[2]);
            let d = dirs
                .entry(path.split('/').next().unwrap().to_string())
                .or_default();
            (d.0, d.1) = (d.0 + 1, d.1 + size);
        }
        let summary: Vec<String> = dirs
            .iter()
            .map(|(d, (n, b))| format!("  {}: {{\"files\": {n}, \"bytes\": {b}}}", json_str(d)))
            .collect();
        out.push(vec![
            ("inventory.csv".to_string(), inventory.into_bytes()),
            ("changes.ndjson".to_string(), log.clone().into_bytes()),
            (
                "summary.json".to_string(),
                format!("{{\n{}\n}}\n", summary.join(",\n")).into_bytes(),
            ),
        ]);
    }
    out
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out + "\""
}

fn csv_str(s: &str) -> String {
    match s.contains([',', '"', '\n']) {
        true => format!("\"{}\"", s.replace('"', "\"\"")),
        false => s.to_string(),
    }
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
/// The proposer prices a set of choices jointly, as repack stores them:
/// zstd of the distinct function and argument contents chosen (one group) +
/// read weight x decode work + zstd of each file left stored. Coordinate
/// descent picks, per file, the form (or none) that lowers that total.
/// Quoted-line runs (E056): text that quotes other files of its snapshot
/// line by line, numbered and JSON-escaped, as agent transcripts do, is
/// offered as `quote(ops, names, sources)` (`functions/quote/`). The kernel
/// verifies each derivation; repack keeps it only if it pays.
fn propose_quotes(repo: &mut Repo<MemStore>, snapshots: &[Snapshot]) {
    let module =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("functions/quote.wasm")).unwrap();
    let Some(last) = snapshots.last() else { return };
    let esc = |b: &[u8]| json_str(std::str::from_utf8(b).unwrap()).into_bytes();
    for (t, (_, target)) in last.iter().enumerate() {
        if target.len() < 64 * 1024 {
            continue;
        }
        let sources: Vec<(&str, Vec<Vec<u8>>)> = last
            .iter()
            .enumerate()
            .filter(|(i, (_, b))| *i != t && std::str::from_utf8(b).is_ok())
            .map(|(_, (name, b))| {
                let text = std::str::from_utf8(b).unwrap();
                let lines = text
                    .lines()
                    .map(|l| {
                        let e = esc(l.as_bytes());
                        e[1..e.len() - 1].to_vec()
                    })
                    .collect();
                (name.as_str(), lines)
            })
            .collect();
        let (ops, quoted) = encode_quotes(target, &sources, |b| {
            let e = esc(b);
            e[1..e.len() - 1].to_vec()
        });
        if quoted < target.len() / 2 {
            continue;
        }
        let names: String = sources.iter().map(|(n, _)| format!("{n}\n")).collect();
        let func = repo.put_content(&module).unwrap().id;
        let mut args = vec![
            Arg::Whole(repo.put_content(&ops).unwrap().id),
            Arg::Whole(repo.put_content(names.as_bytes()).unwrap().id),
        ];
        args.extend(
            last.iter()
                .enumerate()
                .filter(|(i, (_, b))| *i != t && std::str::from_utf8(b).is_ok())
                .map(|(_, (_, b))| Arg::Whole(Id::of_content(b))),
        );
        repo.put_derivation(target, func, args).unwrap();
    }
}

/// The op stream of `functions/quote` for `target`, and how many of its
/// bytes it quotes. Greedy: at each position the longest run of quoted lines.
fn encode_quotes(
    target: &[u8],
    sources: &[(&str, Vec<Vec<u8>>)],
    esc: impl Fn(&[u8]) -> Vec<u8>,
) -> (Vec<u8>, usize) {
    let put = |out: &mut Vec<u8>, mut n: usize| {
        while n >= 0x80 {
            out.push(n as u8 | 0x80);
            n >>= 7;
        }
        out.push(n as u8);
    };
    // A run of `lines` from line `a` in either style, matched at `p`: its end.
    let run = |p: usize, style: u8, s: usize, a: usize| -> (usize, usize) {
        let (mut at, mut n) = (p, 0);
        while let Some(line) = sources[s].1.get(a + n) {
            let head = match style {
                1 => format!("{:>6}\\t", a + n + 1).into_bytes(),
                _ => [
                    esc(sources[s].0.as_bytes()),
                    format!(":{}:", a + n + 1).into_bytes(),
                ]
                .concat(),
            };
            let piece = [head, line.clone(), b"\\n".to_vec()].concat();
            if !target[at..].starts_with(&piece) {
                break;
            }
            (at, n) = (at + piece.len(), n + 1);
        }
        (at, n)
    };
    let number = |b: &[u8]| {
        std::str::from_utf8(b)
            .ok()?
            .trim_start()
            .parse::<usize>()
            .ok()
    };
    let (mut ops, mut literal, mut quoted, mut p) = (Vec::new(), Vec::new(), 0, 0);
    while p < target.len() {
        let mut best: Option<(usize, u8, usize, usize, usize)> = None; // end, style, s, a, n
        // Style 1: `{n:>6}\t` then the line.
        if p + 8 <= target.len()
            && &target[p + 6..p + 8] == b"\\t"
            && let Some(n) = number(&target[p..p + 6]).filter(|&n| n > 0)
        {
            for s in 0..sources.len() {
                let (end, k) = run(p, 1, s, n - 1);
                if k > 0 && best.is_none_or(|b| end > b.0) {
                    best = Some((end, 1, s, n - 1, k));
                }
            }
        }
        // Style 2: `{name}:{n}:` then the line.
        for (s, (name, _)) in sources.iter().enumerate() {
            let name = esc(name.as_bytes());
            if target[p..].starts_with(&name) && target.get(p + name.len()) == Some(&b':') {
                let digits = target[p + name.len() + 1..]
                    .iter()
                    .take_while(|b| b.is_ascii_digit())
                    .count();
                if let Some(n) = number(&target[p + name.len() + 1..p + name.len() + 1 + digits])
                    .filter(|&n| n > 0)
                {
                    let (end, k) = run(p, 2, s, n - 1);
                    if k > 0 && best.is_none_or(|b| end > b.0) {
                        best = Some((end, 2, s, n - 1, k));
                    }
                }
            }
        }
        match best {
            Some((end, style, s, a, n)) => {
                if !literal.is_empty() {
                    ops.push(0);
                    put(&mut ops, literal.len());
                    ops.append(&mut literal);
                }
                ops.push(style);
                for v in [s, a, n] {
                    put(&mut ops, v);
                }
                quoted += end - p;
                p = end;
            }
            None => {
                literal.push(target[p]);
                p += 1;
            }
        }
    }
    if !literal.is_empty() {
        ops.push(0);
        put(&mut ops, literal.len());
        ops.append(&mut literal);
    }
    (ops, quoted)
}

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
    let zstd = |b: &[u8]| zstd::bulk::compress(b, 3).unwrap().len() as f64;
    let read = |path: &str| std::fs::read(dir.join(path)).unwrap();
    let outputs: BTreeSet<Id> = found.iter().map(|p| p.2).collect();
    let stored: BTreeMap<Id, f64> = outputs.iter().map(|o| (*o, zstd(files[o]))).collect();
    let total = |chosen: &BTreeMap<Id, &Candidate>| {
        let parts: BTreeSet<&str> = chosen
            .values()
            .flat_map(|p| [p.0, p.1])
            .filter(|f| *f != "-")
            .collect();
        let group: Vec<u8> = parts.iter().flat_map(|f| read(f)).collect();
        let work: f64 = chosen.values().map(|p| p.4).sum();
        let left: f64 = stored
            .iter()
            .filter(|(o, _)| !chosen.contains_key(o))
            .map(|(_, s)| s)
            .sum();
        zstd(&group) + ikam_kernel::DEFAULT_READ_WEIGHT * work + left
    };
    let mut best: BTreeMap<Id, &Candidate> = BTreeMap::new();
    let mut changed = true;
    while changed {
        changed = false;
        for out in &outputs {
            let mut options: Vec<Option<&Candidate>> = vec![None];
            options.extend(found.iter().filter(|p| p.2 == *out).map(Some));
            let price = |o: &Option<&Candidate>| {
                let mut c = best.clone();
                match o {
                    Some(p) => c.insert(*out, p),
                    None => c.remove(out),
                };
                total(&c)
            };
            let now = price(&best.get(out).copied());
            let pick = options
                .iter()
                .min_by(|a, b| price(a).total_cmp(&price(b)))
                .unwrap();
            if price(pick) < now {
                match pick {
                    Some(p) => best.insert(*out, p),
                    None => best.remove(out),
                };
                changed = true;
            }
        }
    }
    for (out, p) in best {
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

/// Read cost per corpus (reported, not scored): over every distinct file,
/// the decode cost of a whole read and of a 4 KiB range read from its
/// middle, each counted as bytes decompressed + bytes produced by
/// functions. Also whether every range read equals the slice it names.
fn read_costs(repo: &Repo<MemStore>, snapshots: &[Snapshot]) -> (usize, usize, bool) {
    let files: BTreeMap<Id, &Vec<u8>> = snapshots
        .iter()
        .flatten()
        .map(|(_, b)| (Id::of_content(b), b))
        .collect();
    let (mut whole, mut window, mut exact) = (0, 0, true);
    for (id, b) in files {
        let n = b.len() as u64;
        let (start, len) = (n / 2, (n - n / 2).min(4096));
        for (cost, start, len) in [(&mut whole, 0, n), (&mut window, start, len)] {
            let before = ikam_kernel::decode_cost();
            let (bytes, work) = repo.read_range(&id, start, len).unwrap();
            *cost += ikam_kernel::decode_cost() - before + work;
            exact &= bytes == b[start as usize..(start + len) as usize];
        }
    }
    (whole, window, exact)
}

/// Searches per full read pass of a file corpus in the score (E062): the
/// rate of E050's agent trace, about 100 grep scans per pass over the files
/// it read. A workload assumption, not a measurement of this corpus.
const SEARCH_RATE: f64 = 100.0;

/// The kernel's decode cost per search (E062): 100 deterministic grep-like
/// queries (an identifier from a random line of a random file, as the agent
/// trace of E050 searched) over every distinct file, through `Repo::search`
/// with whatever index the store holds. Without an index every query decodes
/// every file, so one query measures them all. Also whether every result
/// equals a scan of the original bytes.
fn search_cost(repo: &Repo<MemStore>, snapshots: &[Snapshot]) -> (usize, bool) {
    let files: BTreeMap<Id, &Vec<u8>> = snapshots
        .iter()
        .flatten()
        .map(|(_, b)| (Id::of_content(b), b))
        .collect();
    let ids: Vec<Id> = files.keys().copied().collect();
    let mut s = 0x1234_5678u64;
    let mut lcg = move || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (s >> 33) as usize
    };
    let mut queries = Vec::new();
    while queries.len() < 100 {
        let text = String::from_utf8_lossy(files[&ids[lcg() % ids.len()]]);
        let lines: Vec<&str> = text.lines().collect();
        let line = lines.get(lcg() % lines.len().max(1)).copied().unwrap_or("");
        let word = line
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .find(|w| w.len() > 3);
        queries.push(word.unwrap_or("data").as_bytes().to_vec());
    }
    let before = ikam_kernel::decode_cost();
    let index = repo.search_index().unwrap();
    if index.is_none() {
        queries.truncate(1);
    }
    let (mut work, mut exact) = (0, true);
    for q in &queries {
        let (hits, w) = repo.search(q, &ids, index.as_ref()).unwrap();
        let expected: Vec<Id> = (ids.iter())
            .filter(|id| files[id].windows(q.len()).any(|w| w == q.as_slice()))
            .copied()
            .collect();
        (work, exact) = (work + w, exact && hits == expected);
    }
    (
        (ikam_kernel::decode_cost() - before + work) / queries.len(),
        exact,
    )
}

/// Read cost of a graph corpus (since E039): reading every distinct
/// version's graph back once, as bytes decompressed (weighted per codec).
/// Graph reads go through `graph_nodes`, which reports no function work.
fn graph_read_cost(repo: &Repo<MemStore>) -> usize {
    // Each distinct graph once, as file corpora read each distinct file
    // once (E042: E039 read every commit's graph, duplicates included).
    let mut graphs = BTreeSet::new();
    for (_, c) in repo.log(repo.resolve("main").unwrap()).unwrap() {
        let Object::Tree(entries) = repo.get(&c.tree).unwrap() else {
            panic!("a graph commit's tree");
        };
        graphs.insert(entries[0].id);
    }
    // Only the graph reads count, as only content reads do for files.
    let before = ikam_kernel::decode_cost();
    graphs
        .iter()
        .for_each(|g| drop(repo.graph_nodes(g).unwrap()));
    ikam_kernel::decode_cost() - before
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
/// Bytes of the distinct files of a corpus: what the baseline decodes to
/// read each file once.
fn distinct_bytes(snapshots: &[Snapshot]) -> usize {
    let files: BTreeMap<Id, usize> = snapshots
        .iter()
        .flatten()
        .map(|(_, b)| (Id::of_content(b), b.len()))
        .collect();
    files.values().sum()
}

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
        ("structured", structured, false),
        ("transcripts", transcripts, false),
    ];
    let (mut log_ratio, mut log_read, mut log_bytes, mut n, mut total_ms) = (0.0, 0.0, 0.0, 0, 0);
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
        propose_quotes(&mut repo, &snapshots);
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
        let (read_whole, read_4k, range_exact) = match graph {
            true => (graph_read_cost(&repo), 0, true),
            false => read_costs(&repo, &snapshots),
        };
        let exact = exact && range_exact;
        let fsck = repo.fsck().unwrap().is_empty();
        let (zstd_files, zstd_dict) = baselines(&snapshots);
        let ratio = repack as f64 / zstd_dict as f64;
        // With read cost (E034): the baseline reads each distinct file by
        // decoding its zstd frame once; the kernel's read cost is
        // `read_whole`. For a graph corpus (E039) the files are each
        // version's edge list and the kernel reads each version's graph.
        let w = ikam_kernel::DEFAULT_READ_WEIGHT;
        let baseline_read = distinct_bytes(&snapshots);
        let read_ratio =
            (repack as f64 + w * read_whole as f64) / (zstd_dict as f64 + w * baseline_read as f64);
        // With search (E062): `SEARCH_RATE` searches per read pass. The
        // baseline searches by decoding every distinct file; the kernel
        // through `Repo::search` (an index it stores is in `repack`).
        let (search, search_exact) = match graph {
            true => (0, true),
            false => search_cost(&repo, &snapshots),
        };
        let rate = if graph { 0.0 } else { SEARCH_RATE };
        let search_ratio = (repack as f64 + w * (read_whole as f64 + rate * search as f64))
            / (zstd_dict as f64 + w * (1.0 + rate) * baseline_read as f64);
        let exact = exact && search_exact;
        let kinds = by_kind(&repo);
        if json {
            println!(
                "{{\"corpus\":\"{name}\",\"input\":{input},\"ingest\":{ingest},\"repack\":{repack},\"ingest_ms\":{ingest_ms},\"repack_ms\":{repack_ms},\"zstd_files\":{zstd_files},\"zstd_dict\":{zstd_dict},\"vs_dict\":{ratio:.4},\"vs_dict_read\":{read_ratio:.4},\"vs_dict_search\":{search_ratio:.4},\"search\":{search},\"read_baseline\":{baseline_read},\"exact\":{exact},\"fsck\":{fsck},\"read_whole\":{read_whole},\"read_4k\":{read_4k},\"by_kind\":{kinds}}}"
            );
        } else {
            println!(
                "{name:<18} input={input:>9} ingest={ingest:>8} repack={repack:>8} ({:.3})  zstd/file={zstd_files:>8} zstd+dict={zstd_dict:>8}  vs_dict={ratio:.3}  {ingest_ms}+{repack_ms} ms  exact={exact} fsck={fsck}",
                repack as f64 / input as f64
            );
        }
        // Hard gates: an inexact or fsck-failing result is a failure, not a score.
        assert!(exact && fsck, "{name}: verification failed");
        (log_ratio, log_read, log_bytes, n, total_ms) = (
            log_ratio + search_ratio.ln(),
            log_read + read_ratio.ln(),
            log_bytes + ratio.ln(),
            n + 1,
            total_ms + ingest_ms + repack_ms,
        );
    }
    // The experiment score (since E062): geometric mean across corpora of
    // (kernel bytes + read weight x (read cost + SEARCH_RATE x search cost))
    // / (trained-dictionary baseline bytes + read weight x (1 + SEARCH_RATE)
    // x its read cost); every corpus weighs the same; lower is better; below
    // 1.0 beats the baseline. `read_score` is the score of E034-E061 (no
    // search) and `bytes_score` the one before E034 (bytes only), reported
    // for continuity.
    let score = (log_ratio / n.max(1) as f64).exp();
    let read_score = (log_read / n.max(1) as f64).exp();
    let bytes_score = (log_bytes / n.max(1) as f64).exp();
    if json {
        println!(
            "{{\"corpus\":\"_summary\",\"corpora\":{n},\"score\":{score:.4},\"read_score\":{read_score:.4},\"bytes_score\":{bytes_score:.4},\"total_ms\":{total_ms}}}"
        );
    } else {
        println!(
            "score (geometric mean of (repack + w*(read + {SEARCH_RATE}*search)) / (zstd+dict + w*(1 + {SEARCH_RATE})*read)) = {score:.4}, without search = {read_score:.4}, bytes only = {bytes_score:.4}, over {n} corpora, {total_ms} ms"
        );
    }
}
