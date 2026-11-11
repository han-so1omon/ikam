//! Shared by the IR examples (`ir_compress`, `ir_blocks`): the corpora,
//! the general compressors and the kernel and IR methods they are measured
//! with. Moved out of `ir_compress.rs` unchanged (E197).
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use ikam_kernel::ir::{self, Options, Parts};
use ikam_kernel::{Id, Kind, MemStore, Object, Repo, Store, TreeEntry};

pub type Files = Vec<(String, Vec<u8>)>;

/// `examples/bench.rs`' `ARCHIVE_READ_WEIGHT` (E099): 2 reads a month.
pub const ARCHIVE_READ_WEIGHT: f64 = 2.0 * 1e-9 * (0.04 / 3600.0) / (0.023 / 1e9);

// ---------------------------------------------------------------- corpora

pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs[self.below(xs.len() as u64) as usize]
    }
}

pub const SENSORS: &[&str] = &[
    "claims", "permits", "listings", "searches", "openings", "rents", "loads", "fares", "bookings",
    "filings", "hires", "reviews",
];
pub const TARGETS: &[&str] = &[
    "rents",
    "starts",
    "prices",
    "revenue",
    "occupancy",
    "payrolls",
    "sales",
    "vacancy",
];
pub const REGIONS: &[&str] = &["sd", "la", "sf", "phx", "den", "aus", "sea", "us"];
pub const WORDS: &[&str] = &[
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
pub const STATUSES: &[&str] = &["passed", "failed dev", "failed holdout", "insufficient n"];

pub struct Link {
    pub id: String,
    pub kind: &'static str,
    pub who: &'static str,
    pub sensor: String,
    pub target: String,
    pub mechanism: String,
    pub lag: u32,
    pub lead: u32,
    pub from_run: usize,
    pub n_dev: u32,
    pub n_hold: u32,
    pub rho: f64,
    pub rho_h: f64,
    pub q: f64,
    pub p: f64,
    pub status: &'static str,
    pub prior: u32,
}

pub fn links(rng: &mut Rng) -> Vec<Link> {
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

pub fn f3(x: f64) -> String {
    format!("{x:.3}")
}

/// `n` lead-lag reports. `random`: every number redrawn each run; otherwise
/// numbers move only when a month of data arrives.
pub fn reports(n: usize, random: bool, seed: u64) -> Files {
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

pub fn stamp(minutes: u64) -> String {
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

pub fn row_cells(l: &Link) -> [String; 11] {
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

pub const HEADER: [&str; 11] = [
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

pub fn render(run: usize, minutes: u64, active: &[usize], ls: &[Link]) -> String {
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
pub fn exports(n: usize) -> Files {
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

pub fn fixture(dir: &str, exts: &[&str]) -> Files {
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
pub fn folder(dir: &Path) -> Files {
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

pub fn corpus(name: &str) -> Files {
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

pub fn solid(files: &Files) -> Vec<u8> {
    let mut out = Vec::new();
    for (n, b) in files {
        out.extend_from_slice(n.as_bytes());
        out.push(0);
        out.extend_from_slice(&(b.len() as u64).to_le_bytes());
        out.extend_from_slice(b);
    }
    out
}

pub fn unsolid(mut b: &[u8]) -> Files {
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

pub fn gz(b: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(b).unwrap();
    e.finish().unwrap()
}

pub fn ungz(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(b)
        .read_to_end(&mut out)
        .unwrap();
    out
}

pub fn xz(b: &[u8]) -> Vec<u8> {
    let mut e = xz2::write::XzEncoder::new(Vec::new(), 9);
    e.write_all(b).unwrap();
    e.finish().unwrap()
}

pub fn unxz(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    xz2::read::XzDecoder::new(b).read_to_end(&mut out).unwrap();
    out
}

pub fn zst(b: &[u8]) -> Vec<u8> {
    zstd::bulk::compress(b, 19).unwrap()
}

pub fn unzst(b: &[u8]) -> Vec<u8> {
    zstd::decode_all(b).unwrap()
}

pub fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// Fastest of 3 runs of `f`, in ms, and its last result.
pub fn timed<T>(mut f: impl FnMut() -> T) -> (f64, T) {
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

pub struct Row {
    pub method: String,
    pub bytes: usize,
    pub enc_ms: f64,
    pub dec_ms: f64,
}

pub fn solid_method(
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

pub fn dict_method(files: &Files) -> Row {
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

pub fn stored(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.read(id).unwrap().len())
        .sum()
}

/// One repository at the archive weight, every entry put, repacked; checked
/// byte-exact and fsck clean.
pub fn kernel_store(entries: &Files) -> Repo<MemStore> {
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

pub fn kernel_method(name: &str, entries: &Files) -> Row {
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
pub fn part_entries_split(p: &Parts, split: bool) -> Files {
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

pub fn part_entries(p: &Parts) -> Files {
    part_entries_split(p, false)
}

pub fn ir_method(name: &str, files: &Files, o: Options) -> Row {
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

pub fn ir_zstd_parts(files: &Files, o: Options) -> Row {
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
            // `ir_compress` never sets `prose`, so there is no raw part
            ..Parts::default()
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

pub fn ir_kernel(files: &Files, o: Options) -> Row {
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

pub const T: Options = Options {
    columns: false,
    numbers: false,
    siblings: false,
    rows: false,
    prose: false,
    nearest: 0,
};
pub const TC: Options = Options { columns: true, ..T };
pub const TCN: Options = Options {
    numbers: true,
    ..TC
};
pub const TCNR: Options = Options { rows: true, ..TCN };
pub const ALL: Options = Options {
    siblings: true,
    ..TCNR
};
