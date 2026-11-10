//! E153: how content is cut into fragments, and how well the cuts survive
//! edits. A measurement only: nothing here changes how the kernel stores.
//!
//! `cargo run --release --example fragments`
//!
//! Corpus: every text file of `tests/fixtures/cases` (md, json, py, csv,
//! txt, log, html) and `fixtures/multiformat`'s md and html. For each
//! cutter, one JSON line with:
//! - fragments, distinct fragments and their bytes (dedup across files)
//! - a stored-size proxy: zstd 3 of the distinct fragments laid end to end,
//!   plus 32 bytes per distinct fragment (its id) and 4 per reference
//! - distinct fragment sizes (p50, p90, p99, max)
//! - sharing: distinct fragments used by two or more files, their bytes, and
//!   the largest fan-out (files using one fragment)
//! - edit survival over the md files: four deterministic edits per file,
//!   with the new fragment bytes each needs and the references it changes
//!
//! A last line, `kernel`, puts each md file and then each edit into a
//! kernel store and reports the bytes each edit writes, for reference.

#![allow(clippy::single_range_in_vec_init)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use ikam_kernel::{Id, MemStore, Repo, Store};

const TEXT: &[&str] = &["md", "json", "py", "csv", "txt", "log", "html"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn walk(root: &Path, keep: &dyn Fn(&Path) -> bool, out: &mut Vec<(String, Vec<u8>)>) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if keep(&path) {
                out.push((
                    path.to_string_lossy().into_owned(),
                    std::fs::read(&path).unwrap(),
                ));
            }
        }
    }
}

pub fn corpus() -> Vec<(String, Vec<u8>)> {
    let ext_ok = |p: &Path, exts: &[&str]| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.contains(&e))
    };
    let mut out = Vec::new();
    walk(
        &repo_root().join("tests/fixtures/cases"),
        &|p| !p.to_string_lossy().contains("_image_tools") && ext_ok(p, TEXT),
        &mut out,
    );
    walk(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/multiformat"),
        &|p| ext_ok(p, &["md", "html"]),
        &mut out,
    );
    out.sort();
    out
}

/// The gear table the kernel's matcher uses (splitmix64), so CDC here and
/// anchors there agree on what "content-defined" means.
fn gear() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    for slot in &mut table {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        *slot = z ^ (z >> 31);
    }
    table
}

#[derive(Clone, Copy, Debug)]
pub enum Cut {
    Whole,
    Lines,
    /// Paragraph blocks: a block ends after a run of blank lines; a block
    /// longer than 16 KiB is cut at the line boundary past it.
    Blocks,
    /// Gear CDC with average `avg`, min avg/4, max avg*4 (FastCDC's shape,
    /// without normalized chunking).
    Cdc(usize),
    /// Gear CDC whose cut moves to the next newline (bounded by max).
    CdcLine(usize),
}

impl Cut {
    pub fn name(&self) -> String {
        match self {
            Cut::Whole => "whole".into(),
            Cut::Lines => "lines".into(),
            Cut::Blocks => "blocks".into(),
            Cut::Cdc(a) => format!("cdc-{a}"),
            Cut::CdcLine(a) => format!("cdc-line-{a}"),
        }
    }
}

pub fn cut(c: Cut, b: &[u8]) -> Vec<Range<usize>> {
    match c {
        Cut::Whole => vec![0..b.len()],
        Cut::Lines => {
            let mut out = Vec::new();
            let mut s = 0;
            for (i, &x) in b.iter().enumerate() {
                if x == b'\n' {
                    out.push(s..i + 1);
                    s = i + 1;
                }
            }
            if s < b.len() {
                out.push(s..b.len());
            }
            out
        }
        Cut::Blocks => blocks(b),
        Cut::Cdc(avg) => cdc(b, avg, false),
        Cut::CdcLine(avg) => cdc(b, avg, true),
    }
}

fn blocks(b: &[u8]) -> Vec<Range<usize>> {
    const MAX: usize = 16 * 1024;
    let mut out = Vec::new();
    let (mut s, mut i) = (0, 0);
    while i < b.len() {
        if b[i] == b'\n' {
            // End the block after this newline and any blank lines after it,
            // if the next line is blank (a paragraph break) or the block is
            // too long.
            let mut j = i + 1;
            let blank = j < b.len() && (b[j] == b'\n' || b[j..].starts_with(b"\r\n"));
            if blank || j - s > MAX {
                while j < b.len() && (b[j] == b'\n' || b[j] == b'\r') {
                    j += 1;
                }
                out.push(s..j);
                s = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    if s < b.len() {
        out.push(s..b.len());
    }
    out
}

fn cdc(b: &[u8], avg: usize, line: bool) -> Vec<Range<usize>> {
    let g = gear();
    let bits = avg.trailing_zeros();
    let mask = (1u64 << bits) - 1;
    let (min, max) = (avg / 4, avg * 4);
    let mut out = Vec::new();
    let mut s = 0;
    while s < b.len() {
        let end = (s + max).min(b.len());
        let mut h = 0u64;
        let mut e = end;
        let mut i = s;
        while i < end {
            h = (h << 1).wrapping_add(g[b[i] as usize]);
            if i + 1 - s >= min && (h >> (64 - bits)) & mask == 0 {
                e = i + 1;
                break;
            }
            i += 1;
        }
        if line && e < b.len() {
            if let Some(nl) = b[e - 1..end].iter().position(|&x| x == b'\n') {
                e = e - 1 + nl + 1;
            } else {
                e = end;
            }
        }
        out.push(s..e);
        s = e;
    }
    out
}

fn pct(v: &[usize], p: f64) -> usize {
    if v.is_empty() {
        return 0;
    }
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// Four deterministic edits of a text, the kinds a person or an agent makes.
pub fn edits(b: &[u8]) -> Vec<(&'static str, Vec<u8>)> {
    let lines: Vec<&[u8]> = b.split_inclusive(|&x| x == b'\n').collect();
    let n = lines.len().max(1);
    let mid = lines
        .iter()
        .enumerate()
        .skip(n / 2)
        .find(|(_, l)| l.len() > 20)
        .map_or(n / 2, |(i, _)| i);
    let join = |ls: &[&[u8]]| ls.concat();
    // A word in a middle line replaced by a longer one.
    let mut word = lines.clone();
    let owned;
    if let Some(l) = word.get(mid) {
        let at = l.iter().position(|&x| x == b' ').unwrap_or(0);
        let mut v = l[..at].to_vec();
        v.extend_from_slice(b" revised");
        v.extend_from_slice(&l[at..]);
        owned = v;
        word[mid] = &owned;
    }
    let mut insert = lines.clone();
    let para: &[u8] = b"\nA new paragraph added during review, with a sentence or two of text that was not in the file before.\n\n";
    insert.insert(n / 2, para);
    let mut delete = lines.clone();
    if !delete.is_empty() {
        delete.remove((n / 3).min(delete.len() - 1));
    }
    let mut append = lines.clone();
    append.push(b"\nAppended note: checked again on review.\n");
    vec![
        ("word", join(&word)),
        ("insert", join(&insert)),
        ("delete", join(&delete)),
        ("append", join(&append)),
    ]
}

fn measure(c: Cut, files: &[(String, Vec<u8>)]) -> String {
    let mut distinct: HashMap<Id, (usize, HashSet<usize>)> = HashMap::new();
    let mut order: Vec<Id> = Vec::new();
    let mut total = 0usize;
    let mut refs = 0usize;
    for (fi, (_, b)) in files.iter().enumerate() {
        for r in cut(c, b) {
            let id = Id::of(&b[r.clone()]);
            refs += 1;
            total += r.len();
            let e = distinct.entry(id).or_insert_with(|| {
                order.push(id);
                (r.len(), HashSet::new())
            });
            e.1.insert(fi);
        }
    }
    // Lay distinct fragments end to end in first-seen order.
    let mut bytes_of: HashMap<Id, &[u8]> = HashMap::new();
    for (_, b) in files {
        for r in cut(c, b) {
            bytes_of.entry(Id::of(&b[r.clone()])).or_insert(&b[r]);
        }
    }
    let laid: Vec<u8> = order
        .iter()
        .flat_map(|id| bytes_of[id].iter().copied())
        .collect();
    let z = zstd::bulk::compress(&laid, 3).unwrap().len();
    let distinct_bytes = laid.len();
    let mut sizes: Vec<usize> = distinct.values().map(|v| v.0).collect();
    sizes.sort();
    let shared: Vec<&(usize, HashSet<usize>)> =
        distinct.values().filter(|v| v.1.len() > 1).collect();
    let shared_bytes: usize = shared.iter().map(|v| v.0).sum();
    let fanout = distinct.values().map(|v| v.1.len()).max().unwrap_or(0);
    let proxy = z + 32 * distinct.len() + 4 * refs;
    // Edit survival over md files.
    let mut ed: BTreeMap<&str, (usize, usize, usize, usize)> = BTreeMap::new();
    for (name, b) in files.iter().filter(|(n, _)| n.ends_with(".md")) {
        let _ = name;
        let old: HashSet<Id> = cut(c, b).into_iter().map(|r| Id::of(&b[r])).collect();
        for (kind, nb) in edits(b) {
            let rs = cut(c, &nb);
            let mut new_bytes = 0;
            let mut changed = 0;
            let mut seen = HashSet::new();
            for r in &rs {
                let id = Id::of(&nb[r.clone()]);
                if !old.contains(&id) && seen.insert(id) {
                    new_bytes += r.len();
                    changed += 1;
                }
            }
            let e = ed.entry(kind).or_default();
            e.0 += 1;
            e.1 += new_bytes;
            e.2 += changed;
            e.3 += rs.len();
        }
    }
    let edits: Vec<String> = ed
        .iter()
        .map(|(k, (n, nb, ch, rs))| {
            format!(
                "\"{k}\":{{\"new_bytes\":{:.0},\"new_frags\":{:.2},\"refs\":{:.1}}}",
                *nb as f64 / *n as f64,
                *ch as f64 / *n as f64,
                *rs as f64 / *n as f64
            )
        })
        .collect();
    format!(
        "{{\"cutter\":\"{}\",\"files\":{},\"input\":{},\"fragments\":{},\"distinct\":{},\"distinct_bytes\":{},\"dedup\":{:.4},\"zstd_distinct\":{},\"proxy\":{},\"size_p50\":{},\"size_p90\":{},\"size_p99\":{},\"size_max\":{},\"shared\":{},\"shared_bytes\":{},\"max_fanout\":{},\"edits_md\":{{{}}}}}",
        c.name(),
        files.len(),
        total,
        refs,
        distinct.len(),
        distinct_bytes,
        total as f64 / distinct_bytes as f64,
        z,
        proxy,
        pct(&sizes, 0.5),
        pct(&sizes, 0.9),
        pct(&sizes, 0.99),
        sizes.last().copied().unwrap_or(0),
        shared.len(),
        shared_bytes,
        fanout,
        edits.join(",")
    )
}

fn stored(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

fn kernel(files: &[(String, Vec<u8>)]) -> String {
    let md: Vec<&(String, Vec<u8>)> = files.iter().filter(|(n, _)| n.ends_with(".md")).collect();
    let mut repo = Repo::new(MemStore::default());
    for (_, b) in &md {
        repo.put_content(b).unwrap();
    }
    let base = stored(&repo);
    let mut ed: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (_, b) in &md {
        for (kind, nb) in edits(b) {
            let before = stored(&repo);
            repo.put_content(&nb).unwrap();
            let e = ed.entry(kind).or_default();
            e.0 += 1;
            e.1 += stored(&repo) - before;
            assert_eq!(repo.read_content(&Id::of_content(&nb)).unwrap(), nb);
        }
    }
    let edits: Vec<String> = ed
        .iter()
        .map(|(k, (n, b))| format!("\"{k}\":{{\"written\":{:.0}}}", *b as f64 / *n as f64))
        .collect();
    format!(
        "{{\"cutter\":\"kernel\",\"md_files\":{},\"stored\":{},\"edits_md\":{{{}}}}}",
        md.len(),
        base,
        edits.join(",")
    )
}

fn main() {
    let files = corpus();
    let cuts = [
        Cut::Whole,
        Cut::Lines,
        Cut::Blocks,
        Cut::Cdc(256),
        Cut::Cdc(1024),
        Cut::Cdc(4096),
        Cut::Cdc(16384),
        Cut::CdcLine(256),
        Cut::CdcLine(1024),
        Cut::CdcLine(4096),
    ];
    for c in cuts {
        // Every cut must cover its input exactly.
        for (_, b) in &files {
            let rs = cut(c, b);
            assert_eq!(
                rs.iter()
                    .map(|r| &b[r.clone()])
                    .collect::<Vec<_>>()
                    .concat(),
                *b
            );
        }
        println!("{}", measure(c, &files));
    }
    println!("{}", kernel(&files));
}
