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

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use ikam_kernel::ir::{self, Options, Parts};
use ikam_kernel::{Id, Kind, MemStore, Object, Repo, Store, TreeEntry};

type Files = Vec<(String, Vec<u8>)>;

/// `examples/bench.rs`' `ARCHIVE_READ_WEIGHT` (E099): 2 reads a month.
const ARCHIVE_READ_WEIGHT: f64 = 2.0 * 1e-9 * (0.04 / 3600.0) / (0.023 / 1e9);

// ---------------------------------------------------------------- corpora

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len() as u64) as usize]
    }
}

const SENSORS: &[&str] = &[
    "claims", "permits", "listings", "searches", "openings", "rents", "loads", "fares", "bookings",
    "filings", "hires", "reviews",
];
const TARGETS: &[&str] = &[
    "rents",
    "starts",
    "prices",
    "revenue",
    "occupancy",
    "payrolls",
    "sales",
    "vacancy",
];
const REGIONS: &[&str] = &["sd", "la", "sf", "phx", "den", "aus", "sea", "us"];
const WORDS: &[&str] = &[
    "demand",
    "leads",
    "supply",
    "with",
    "a",
    "lag",
    "as",
    "new",
    "households",
    "form",
    "and",
    "landlords",
    "reprice",
    "renewals",
    "when",
    "the",
    "pipeline",
    "fills",
    "slowly",
    "after",
    "rates",
    "move",
    "first",
    "local",
    "spending",
    "follows",
    "hiring",
    "into",
    "season",
];
const STATUSES: &[&str] = &["passed", "failed dev", "failed holdout", "insufficient n"];

struct Link {
    id: String,
    kind: &'static str,
    who: &'static str,
    sensor: String,
    target: String,
    mechanism: String,
    lag: u32,
    lead: u32,
    from_run: usize,
    n_dev: u32,
    n_hold: u32,
    rho: f64,
    rho_h: f64,
    q: f64,
    p: f64,
    status: &'static str,
    prior: u32,
}

fn links(rng: &mut Rng) -> Vec<Link> {
    (0..24)
        .map(|i| {
            let s = rng.pick(SENSORS);
            let t = rng.pick(TARGETS);
            let r = rng.pick(REGIONS);
            let words = 8 + rng.below(10) as usize;
            let mut m: Vec<&str> = (0..words).map(|_| rng.pick(WORDS)).collect();
            m[0] = rng.pick(SENSORS);
            Link {
                id: format!("{s}-to-{t}-{r}"),
                kind: ["leading", "coincident", "contrarian"][i % 3],
                who: ["eric", "agent", "literature"][rng.below(3) as usize],
                sensor: format!("{}:{}_{}", ["fred", "google", "zillow", "bls"][i % 4], s, r),
                target: format!("{}:{}_{}", ["zillow", "census", "bls"][i % 3], t, r),
                mechanism: format!("{}.", m.join(" ")),
                lag: [1, 2, 3, 6][rng.below(4) as usize],
                lead: 20 + rng.below(160) as u32,
                from_run: if i < 8 { 0 } else { (i - 7) * 18 },
                n_dev: 30 + rng.below(40) as u32,
                n_hold: 8 + rng.below(12) as u32,
                rho: rng.unit() * 1.2 - 0.6,
                rho_h: rng.unit() * 1.2 - 0.6,
                q: rng.unit(),
                p: rng.unit(),
                status: rng.pick(STATUSES),
                prior: 0,
            }
        })
        .collect()
}

fn f3(x: f64) -> String {
    format!("{x:.3}")
}

/// `n` lead-lag reports. `random`: every number redrawn each run; otherwise
/// numbers move only when a month of data arrives.
fn reports(n: usize, random: bool, seed: u64) -> Files {
    let mut rng = Rng(seed);
    let mut ls = links(&mut rng);
    let mut minutes: u64 = 29_000_000; // about 2025-02
    let mut out = Vec::new();
    for run in 0..n {
        minutes += 300 + rng.below(120);
        let month = run > 0 && rng.below(20) == 0;
        let only = rng.below(10) < 3;
        let pick = rng.below(24) as usize;
        let active: Vec<usize> = (0..ls.len())
            .filter(|&i| {
                ls[i].from_run <= run && (!only || ls[i].id.starts_with(&ls[pick].id[..3]))
            })
            .collect();
        let active = if active.is_empty() { vec![0] } else { active };
        for &i in &active {
            let l = &mut ls[i];
            if random {
                l.n_dev = 30 + rng.below(60) as u32;
                l.n_hold = 8 + rng.below(20) as u32;
                (l.rho, l.rho_h, l.q, l.p) = (
                    rng.unit() * 1.2 - 0.6,
                    rng.unit() * 1.2 - 0.6,
                    rng.unit(),
                    rng.unit(),
                );
                l.status = rng.pick(STATUSES);
                l.lead = 20 + rng.below(160) as u32;
            } else if month {
                l.n_dev += 1;
                l.n_hold += u32::from(rng.below(3) == 0);
                l.rho += rng.unit() * 0.04 - 0.02;
                l.rho_h += rng.unit() * 0.06 - 0.03;
                l.q = (l.q + rng.unit() * 0.1 - 0.05).clamp(0.0, 1.0);
                l.p = (l.p + rng.unit() * 0.1 - 0.05).clamp(0.0, 1.0);
                if rng.below(8) == 0 {
                    l.status = rng.pick(STATUSES);
                }
            }
        }
        out.push((
            format!("reports/leadlag-{run:04}.md"),
            render(run, minutes, &active, &ls).into_bytes(),
        ));
        active.iter().for_each(|&i| ls[i].prior += 1);
    }
    out
}

fn stamp(minutes: u64) -> String {
    // days since 1970 to a civil date (Howard Hinnant's algorithm).
    let (days, rest) = ((minutes / 1440) as i64, minutes % 1440);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}", rest / 60, rest % 60)
}

fn row_cells(l: &Link) -> [String; 11] {
    [
        l.id.clone(),
        l.lag.to_string(),
        l.n_dev.to_string(),
        f3(l.rho),
        f3(l.q),
        l.n_hold.to_string(),
        f3(l.rho_h),
        f3(l.p),
        l.lead.to_string(),
        l.status.to_string(),
        l.prior.to_string(),
    ]
}

const HEADER: [&str; 11] = [
    "link",
    "lag",
    "dev n",
    "dev rho",
    "dev q",
    "hold n",
    "hold rho",
    "hold p",
    "lead (days)",
    "status",
    "earlier runs",
];

fn render(run: usize, minutes: u64, active: &[usize], ls: &[Link]) -> String {
    let mut s = format!(
        "# Lead-lag run {} \u{2014} {} UTC\n\n{} trials (links x lags). Dev/holdout split 70%/30% by date; \
         Benjamini-Hochberg FDR 0.1 on dev; holdout one-sided alpha 0.05. p-values use \
         autocorrelation-adjusted n. {} earlier draft hypotheses on these sensor/target pairs were \
         added to the correction.\n\n| {} |\n|{}\n",
        1000 + run,
        stamp(minutes),
        active.len() * 4,
        active.len() / 3,
        HEADER.join(" | "),
        "---|".repeat(11)
    );
    for &i in active {
        s += &format!("| {} |\n", row_cells(&ls[i]).join(" | "));
    }
    s += "\n## Paths\n\nA path is only as strong as its weakest link.\n\n";
    for w in active.windows(2).step_by(3) {
        let (a, b) = (&ls[w[0]], &ls[w[1]]);
        let ok = a.status == "passed" && b.status == "passed";
        s += &format!(
            "- **{}-via-{}** ({}) \u{2014} {}: {} \u{2192} {}: {}\n",
            a.id,
            b.id,
            if ok { "passed" } else { "not established" },
            a.id,
            a.status,
            b.id,
            b.status
        );
    }
    s += "\n## Links\n\n";
    for &i in active {
        let l = &ls[i];
        s += &format!(
            "- **{}** ({}, proposed by {}): `{}` \u{2192} `{}`. {}\n",
            l.id, l.kind, l.who, l.sensor, l.target, l.mechanism
        );
    }
    s
}

/// Each report with its table also as csv and json (same stem).
fn exports(n: usize) -> Files {
    let mut out = Vec::new();
    for (name, md) in reports(n, false, 7) {
        let text = String::from_utf8(md.clone()).unwrap();
        let rows: Vec<Vec<&str>> = text
            .lines()
            .filter(|l| l.starts_with("| ") && !l.starts_with("| link"))
            .map(|l| {
                l.trim_matches(|c| c == '|' || c == ' ')
                    .split(" | ")
                    .collect()
            })
            .collect();
        let numeric = |c: &str| c.parse::<f64>().is_ok();
        let mut csv = HEADER.map(|h| format!("\"{h}\"")).join(",") + "\n";
        let mut json = String::from("[\n");
        for (r, row) in rows.iter().enumerate() {
            let cells: Vec<String> = row
                .iter()
                .map(|c| {
                    if numeric(c) {
                        c.to_string()
                    } else {
                        format!("\"{c}\"")
                    }
                })
                .collect();
            csv += &(cells.join(",") + "\n");
            let fields: Vec<String> = HEADER
                .iter()
                .zip(&cells)
                .map(|(h, c)| format!("    \"{h}\": {c}"))
                .collect();
            json += &format!(
                "  {{\n{}\n  }}{}\n",
                fields.join(",\n"),
                if r + 1 < rows.len() { "," } else { "" }
            );
        }
        json += "]\n";
        let stem = name.trim_end_matches(".md");
        out.push((format!("{stem}.csv"), csv.into_bytes()));
        out.push((format!("{stem}.json"), json.into_bytes()));
        out.push((name, md));
    }
    out
}

fn fixture(dir: &str, exts: &[&str]) -> Files {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(dir);
    let mut names: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| exts.iter().any(|e| n.ends_with(&format!(".{e}"))))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|n| (format!("{dir}/{n}"), std::fs::read(root.join(&n)).unwrap()))
        .collect()
}

/// Every file directly in `dir`, by name (read only; for real reports, such
/// as Mercante's `research/reports`). Only sizes and times are printed.
fn folder(dir: &Path) -> Files {
    let mut out: Files = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_file())
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

fn corpus(name: &str) -> Files {
    match name {
        "reports-drift" => reports(300, false, 1),
        "reports-random" => reports(300, true, 1),
        "reports-30" => reports(30, true, 3),
        "exports" => exports(100),
        "tables" => fixture("tables", &["csv", "json", "md"]),
        "texts" => fixture("multiformat", &["md"]),
        _ => match name.strip_prefix("dir:") {
            Some(dir) => folder(Path::new(dir)),
            None => panic!("unknown corpus {name}"),
        },
    }
}

// ---------------------------------------------------------------- codecs

fn solid(files: &Files) -> Vec<u8> {
    let mut out = Vec::new();
    for (n, b) in files {
        out.extend_from_slice(n.as_bytes());
        out.push(0);
        out.extend_from_slice(&(b.len() as u64).to_le_bytes());
        out.extend_from_slice(b);
    }
    out
}

fn unsolid(mut b: &[u8]) -> Files {
    let mut out = Vec::new();
    while !b.is_empty() {
        let z = b.iter().position(|&c| c == 0).unwrap();
        let name = String::from_utf8(b[..z].to_vec()).unwrap();
        let len = u64::from_le_bytes(b[z + 1..z + 9].try_into().unwrap()) as usize;
        out.push((name, b[z + 9..z + 9 + len].to_vec()));
        b = &b[z + 9 + len..];
    }
    out
}

fn gz(b: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(b).unwrap();
    e.finish().unwrap()
}

fn ungz(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(b)
        .read_to_end(&mut out)
        .unwrap();
    out
}

fn xz(b: &[u8]) -> Vec<u8> {
    let mut e = xz2::write::XzEncoder::new(Vec::new(), 9);
    e.write_all(b).unwrap();
    e.finish().unwrap()
}

fn unxz(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    xz2::read::XzDecoder::new(b).read_to_end(&mut out).unwrap();
    out
}

fn zst(b: &[u8]) -> Vec<u8> {
    zstd::bulk::compress(b, 19).unwrap()
}

fn unzst(b: &[u8]) -> Vec<u8> {
    zstd::decode_all(b).unwrap()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// Fastest of 3 runs of `f`, in ms, and its last result.
fn timed<T>(mut f: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::MAX;
    let mut last = None;
    for _ in 0..3 {
        let t = Instant::now();
        let r = f();
        best = best.min(ms(t));
        last = Some(r);
    }
    (best, last.unwrap())
}

struct Row {
    method: String,
    bytes: usize,
    enc_ms: f64,
    dec_ms: f64,
}

fn solid_method(
    name: &str,
    files: &Files,
    c: fn(&[u8]) -> Vec<u8>,
    d: fn(&[u8]) -> Vec<u8>,
) -> Row {
    let (enc_ms, z) = timed(|| c(&solid(files)));
    let (dec_ms, back) = timed(|| unsolid(&d(&z)));
    assert!(back == *files, "{name}: not exact");
    Row {
        method: name.into(),
        bytes: z.len(),
        enc_ms,
        dec_ms,
    }
}

fn dict_method(files: &Files) -> Row {
    let distinct: BTreeMap<Id, &Vec<u8>> =
        files.iter().map(|(_, b)| (Id::of_content(b), b)).collect();
    let samples: Vec<&Vec<u8>> = distinct.values().copied().collect();
    let total: usize = samples.iter().map(|b| b.len()).sum();
    let (enc_ms, (dict, zs)) = timed(|| {
        let dict = zstd::dict::from_samples(&samples, (total / 10).clamp(4096, 112_640))
            .unwrap_or_default();
        let mut c = zstd::bulk::Compressor::with_dictionary(3, &dict).unwrap();
        let zs: Vec<Vec<u8>> = samples.iter().map(|b| c.compress(b).unwrap()).collect();
        (dict, zs)
    });
    let (dec_ms, back) = timed(|| {
        let mut d = zstd::bulk::Decompressor::with_dictionary(&dict).unwrap();
        zs.iter()
            .map(|z| d.decompress(z, 1 << 26).unwrap())
            .collect::<Vec<_>>()
    });
    assert!(
        back.iter().zip(&samples).all(|(a, b)| a == *b),
        "dict: not exact"
    );
    let bytes = dict.len() + zs.iter().map(Vec::len).sum::<usize>();
    Row {
        method: "zstd3+dict per file".into(),
        bytes,
        enc_ms,
        dec_ms,
    }
}

fn stored(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

/// One repository at the archive weight, every entry put, repacked; checked
/// byte-exact and fsck clean.
fn kernel_store(entries: &Files) -> Repo<MemStore> {
    let mut repo = Repo::new(MemStore::default());
    repo.read_weight = ARCHIVE_READ_WEIGHT;
    let tree: Vec<TreeEntry> = entries
        .iter()
        .map(|(n, b)| TreeEntry {
            name: n.replace('/', "|"),
            kind: Kind::File,
            id: repo.put_content(b).unwrap().id,
        })
        .collect();
    let tree = repo.put(&Object::tree(tree).unwrap()).unwrap();
    repo.commit("main", tree, "").unwrap();
    repo.repack().unwrap();
    assert!(
        entries
            .iter()
            .all(|(_, b)| repo.read_content(&Id::of_content(b)).unwrap() == *b)
    );
    assert!(repo.fsck().unwrap().is_empty());
    repo
}

fn kernel_method(name: &str, entries: &Files) -> Row {
    let (enc_ms, repo) = timed(|| kernel_store(entries));
    let (dec_ms, _) = timed(|| {
        entries
            .iter()
            .map(|(_, b)| repo.read_content(&Id::of_content(b)).unwrap().len())
            .sum::<usize>()
    });
    Row {
        method: name.into(),
        bytes: stored(&repo),
        enc_ms,
        dec_ms,
    }
}

/// IR parts as named entries: skeletons, files, directory, and either every
/// stream on its own (each column readable alone) or all streams as one.
fn part_entries_split(p: &Parts, split: bool) -> Files {
    let mut out = vec![
        (
            "ir/skeletons".to_string(),
            ir::pack(&Parts {
                skeletons: p.skeletons.clone(),
                ..Parts::default()
            }),
        ),
        ("ir/files".to_string(), p.files.clone()),
        ("ir/directory".to_string(), p.directory.clone()),
    ];
    if split {
        out.extend(
            p.streams
                .iter()
                .enumerate()
                .map(|(i, s)| (format!("ir/stream-{i:05}"), s.clone())),
        );
    } else {
        out.push((
            "ir/streams".to_string(),
            ir::pack(&Parts {
                streams: p.streams.clone(),
                ..Parts::default()
            }),
        ));
    }
    out
}

fn part_entries(p: &Parts) -> Files {
    part_entries_split(p, false)
}

fn ir_method(name: &str, files: &Files, o: Options) -> Row {
    let (enc_ms, z) = timed(|| xz(&ir::pack(&ir::encode(files, o))));
    let (dec_ms, back) = timed(|| ir::decode(&ir::unpack(&unxz(&z)).unwrap()).unwrap());
    assert!(back == *files, "{name}: not exact");
    Row {
        method: name.into(),
        bytes: z.len(),
        enc_ms,
        dec_ms,
    }
}

fn ir_zstd_parts(files: &Files, o: Options) -> Row {
    let (enc_ms, zs) = timed(|| {
        part_entries_split(&ir::encode(files, o), true)
            .iter()
            .map(|(_, b)| zst(b))
            .collect::<Vec<_>>()
    });
    let (dec_ms, back) = timed(|| {
        let mut parts: Vec<Vec<u8>> = zs.iter().map(|z| unzst(z)).collect();
        let streams = parts.split_off(3);
        let skeletons = ir::unpack(&parts[0]).unwrap().skeletons;
        let p = Parts {
            skeletons,
            files: parts[1].clone(),
            directory: parts[2].clone(),
            streams,
        };
        ir::decode(&p).unwrap()
    });
    assert!(back == *files, "ir zstd parts: not exact");
    Row {
        method: "IR best, zstd -19 per part".into(),
        bytes: zs.iter().map(Vec::len).sum(),
        enc_ms,
        dec_ms,
    }
}

fn ir_kernel(files: &Files, o: Options) -> Row {
    let parts = ir::encode(files, o);
    let entries = part_entries(&parts);
    let mut row = kernel_method("IR best, in the kernel", &entries);
    let (enc_ms, _) = timed(|| ir::encode(files, o));
    row.enc_ms += enc_ms;
    let (dec_ms, back) = timed(|| ir::decode(&parts).unwrap());
    assert!(back == *files);
    row.dec_ms += dec_ms;
    row
}

const T: Options = Options {
    columns: false,
    numbers: false,
    siblings: false,
    rows: false,
};
const TC: Options = Options { columns: true, ..T };
const TCN: Options = Options {
    numbers: true,
    ..TC
};
const TCNR: Options = Options { rows: true, ..TCN };
const ALL: Options = Options {
    siblings: true,
    ..TCNR
};

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
