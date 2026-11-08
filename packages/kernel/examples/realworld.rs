//! Real-world corpora (see `realworld/README.md`): each corpus directory is
//! ingested into a fresh store as one commit, or one commit per version when
//! its top-level entries are all directories named like versions; then
//! repacked, verified byte for byte and fsck'd, and compared with common
//! archivers. Not a scoring command: the bench's two scores stay the gate.
//!
//! `cargo run --release --example realworld -- [--json] [--short-records] [--markup-layers] [--read-weight W] [--slow-codec-max BYTES] DIR...`

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ikam_kernel::{DEFAULT_READ_WEIGHT, Id, MemStore, Repo, Store, snapshot};

struct Corpus {
    name: String,
    versions: Vec<PathBuf>,
}

fn corpus(dir: &Path) -> Corpus {
    let mut subdirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    subdirs.sort_by_key(|p| version_key(p));
    let series = !subdirs.is_empty()
        && subdirs.iter().all(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(|c: char| c.is_ascii_digit()))
        });
    Corpus {
        name: dir.file_name().unwrap().to_string_lossy().into(),
        versions: if series {
            subdirs
        } else {
            vec![dir.to_path_buf()]
        },
    }
}

/// "2.10.0" after "2.9.1": numeric parts compared as numbers.
fn version_key(p: &Path) -> Vec<(u64, String)> {
    let name = p.file_name().unwrap().to_string_lossy();
    name.split(['.', '-'])
        .map(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            (
                digits.parse().unwrap_or(0),
                part[digits.len()..].to_string(),
            )
        })
        .collect()
}

/// Every regular file under `dir`, relative path and bytes.
fn read_all(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.is_file() {
                let rel = p.strip_prefix(dir).unwrap().to_string_lossy().into_owned();
                out.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

struct Run {
    /// Stored bytes after ingest, before the first repack.
    ingest_stored: usize,
    input: usize,
    distinct: usize,
    files: usize,
    stored: usize,
    ingest_ms: u128,
    repack_ms: u128,
    read_ms: u128,
    exact: bool,
    fsck: bool,
    kinds: BTreeMap<char, usize>,
    /// Peak resident memory of the store's run alone (ingest, repack, reads,
    /// fsck), before the baselines: xz 9 alone can take hundreds of MiB.
    peak_kib: usize,
}

fn run(c: &Corpus, short: bool, markup: bool, weight: f64, slow_max: usize) -> Run {
    reset_peak();
    let mut repo = Repo::new(MemStore::default());
    repo.short_records = short;
    repo.markup_layers = markup;
    repo.read_weight = weight;
    repo.slow_codec_max = slow_max;
    let t = Instant::now();
    for v in &c.versions {
        let tree = snapshot::snapshot(&mut repo, v, Path::new("/nonexistent")).unwrap();
        let msg = v.file_name().unwrap().to_string_lossy();
        repo.commit("main", tree, &msg).unwrap();
    }
    let ingest_ms = t.elapsed().as_millis();
    let ingest_stored = stored_bytes(&repo);
    let t = Instant::now();
    repo.repack().unwrap();
    let repack_ms = t.elapsed().as_millis();
    let (mut input, mut files) = (0, 0);
    let mut distinct: BTreeMap<Id, usize> = BTreeMap::new();
    let mut exact = true;
    let t = Instant::now();
    for v in &c.versions {
        for (_, bytes) in read_all(v) {
            input += bytes.len();
            files += 1;
            let id = Id::of_content(&bytes);
            distinct.insert(id, bytes.len());
            exact &= repo.read_content(&id).map(|b| b == bytes).unwrap_or(false);
        }
    }
    let read_ms = t.elapsed().as_millis();
    let fsck = repo.fsck().unwrap().is_empty();
    let peak = peak_kib();
    let mut kinds = BTreeMap::new();
    let mut stored = 0;
    for id in repo.store().ids().unwrap() {
        let e = repo.store().read(&id).unwrap();
        *kinds
            .entry(*e.first().unwrap_or(&b'?') as char)
            .or_default() += e.len();
        stored += e.len();
    }
    Run {
        ingest_stored,
        input,
        distinct: distinct.values().sum(),
        files,
        stored,
        ingest_ms,
        repack_ms,
        read_ms,
        exact,
        fsck,
        kinds,
        peak_kib: peak,
    }
}

fn stored_bytes(repo: &Repo<MemStore>) -> usize {
    let ids = repo.store().ids().unwrap();
    ids.iter()
        .map(|id| repo.store().read(id).unwrap().len())
        .sum()
}

/// Reset the peak resident memory to the current (Linux), so each corpus
/// reports its own peak rather than the process's so far.
fn reset_peak() {
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

/// Peak resident memory of this process so far (Linux), in KiB.
fn peak_kib() -> usize {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let line = status
        .lines()
        .find(|l| l.starts_with("VmHWM:"))
        .unwrap_or("");
    line.split_whitespace()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

/// Archivers on the distinct file contents (what a deduplicating store has
/// to hold): zstd 3 with a trained dictionary per file (the bench's
/// baseline), zstd 19 per file, and solid zstd 19 (long window) and xz 9
/// over all of them in path order, as `tar | zstd` or `tar | xz` would.
fn baselines(c: &Corpus) -> [(&'static str, usize); 4] {
    let mut seen = std::collections::HashSet::new();
    let files: Vec<Vec<u8>> = c
        .versions
        .iter()
        .flat_map(|v| read_all(v))
        .filter(|(_, b)| seen.insert(Id::of_content(b)))
        .map(|(_, b)| b)
        .collect();
    let refs: Vec<&Vec<u8>> = files.iter().collect();
    let total: usize = files.iter().map(Vec::len).sum();
    let dict =
        zstd::dict::from_samples(&refs, (total / 10).clamp(4096, 112_640)).unwrap_or_default();
    let mut c3 = zstd::bulk::Compressor::with_dictionary(3, &dict).unwrap();
    let dict3 = dict.len()
        + files
            .iter()
            .map(|b| c3.compress(b).unwrap().len())
            .sum::<usize>();
    let z19 = files
        .iter()
        .map(|b| zstd::bulk::compress(b, 19).unwrap().len())
        .sum();
    let solid = files.concat();
    let mut enc = zstd::stream::Encoder::new(Vec::new(), 19).unwrap();
    enc.long_distance_matching(true).unwrap();
    enc.window_log(27).unwrap();
    enc.write_all(&solid).unwrap();
    let zsolid = enc.finish().unwrap().len();
    let mut xz = xz2::write::XzEncoder::new(Vec::new(), 9);
    xz.write_all(&solid).unwrap();
    let xsolid = xz.finish().unwrap().len();
    [
        ("zstd3_dict", dict3),
        ("zstd19_files", z19),
        ("zstd19_solid", zsolid),
        ("xz9_solid", xsolid),
    ]
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let json = take(&mut args, "--json");
    let short = take(&mut args, "--short-records");
    let markup = take(&mut args, "--markup-layers");
    let weight = match args.iter().position(|a| a == "--read-weight") {
        Some(i) => {
            let w = args.remove(i + 1).parse().expect("--read-weight W");
            args.remove(i);
            w
        }
        None => DEFAULT_READ_WEIGHT,
    };
    let slow_max = match args.iter().position(|a| a == "--slow-codec-max") {
        Some(i) => {
            let n = args.remove(i + 1).parse().expect("--slow-codec-max BYTES");
            args.remove(i);
            n
        }
        None => usize::MAX,
    };
    for dir in &args {
        let c = corpus(Path::new(dir));
        let r = run(&c, short, markup, weight, slow_max);
        let base = baselines(&c);
        let best = base.iter().map(|b| b.1).min().unwrap();
        if json {
            let b: Vec<String> = base.iter().map(|(k, v)| format!("\"{k}\":{v}")).collect();
            let k: Vec<String> = r
                .kinds
                .iter()
                .map(|(k, v)| format!("\"{k}\":{v}"))
                .collect();
            println!(
                "{{\"corpus\":\"{}\",\"versions\":{},\"files\":{},\"input\":{},\"distinct\":{},\"stored\":{},\"ingest_stored\":{},\"peak_kib\":{},\"store_peak_kib\":{},\"vs_best\":{:.4},\"baselines\":{{{}}},\"ingest_ms\":{},\"repack_ms\":{},\"read_ms\":{},\"exact\":{},\"fsck\":{},\"by_kind\":{{{}}}}}",
                c.name,
                c.versions.len(),
                r.files,
                r.input,
                r.distinct,
                r.stored,
                r.ingest_stored,
                peak_kib(),
                r.peak_kib,
                r.stored as f64 / best as f64,
                b.join(","),
                r.ingest_ms,
                r.repack_ms,
                r.read_ms,
                r.exact,
                r.fsck,
                k.join(",")
            );
        } else {
            println!(
                "{:<16} v={:<2} files={:<4} input={:>10} distinct={:>10} kernel={:>9} zstd3+dict={:>9} zstd19={:>9} zstd19-solid={:>9} xz9-solid={:>9} vs_best={:.3} before_repack={} ingest={}ms repack={}ms read={}ms peak={}MiB store_peak={}MiB exact={} fsck={}",
                c.name,
                c.versions.len(),
                r.files,
                r.input,
                r.distinct,
                r.stored,
                base[0].1,
                base[1].1,
                base[2].1,
                base[3].1,
                r.stored as f64 / best as f64,
                r.ingest_stored,
                r.ingest_ms,
                r.repack_ms,
                r.read_ms,
                peak_kib() / 1024,
                r.peak_kib / 1024,
                r.exact,
                r.fsck
            );
        }
    }
}

fn take(args: &mut Vec<String>, flag: &str) -> bool {
    let found = args.iter().position(|a| a == flag);
    if let Some(i) = found {
        args.remove(i);
    }
    found.is_some()
}
