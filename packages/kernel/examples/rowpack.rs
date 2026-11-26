//! E402: rows stored one by one as the store stores contents (raw, zstd, or
//! zstd with a dictionary trained on a sample, the smallest), against the
//! same rows encoded with a rowpack context trained on the same sample
//! (`ikam::rowpack`), and the planner's choice of the smaller per row. Also
//! the time to decode a row each way.
//!
//!     cargo run --release --example rowpack -- FILE lines|events [LEVEL]
//!
//! `lines`: each line of FILE is a row (its newline included, as in a
//! file). `events`: each line is `key TAB idempotency-key TAB value`, and the
//! row is that upsert's event encoding, as `db_append` stores it. Prints one
//! JSON line.
use std::time::Instant;

use ikam_kernel::rowpack::Context;
use ikam_kernel::{Event, Op};

fn zstd(b: &[u8], dict: &[u8], level: i32) -> Vec<u8> {
    let mut c = zstd::bulk::Compressor::with_dictionary(level, dict).unwrap();
    c.include_checksum(false).unwrap();
    c.include_contentsize(true).unwrap();
    c.include_dictid(false).unwrap();
    c.compress(b).unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (file, kind) = (&args[1], &args[2]);
    let level: i32 = args.get(3).map_or(3, |l| l.parse().unwrap());
    let text = std::fs::read(file).unwrap();
    let rows: Vec<Vec<u8>> = text
        .split_inclusive(|b| *b == b'\n')
        .map(|l| match kind.as_str() {
            "lines" => l.to_vec(),
            _ => {
                let l = std::str::from_utf8(l).unwrap().trim_end_matches('\n');
                let mut f = l.splitn(3, '\t');
                let (key, idk, value) = (f.next().unwrap(), f.next().unwrap(), f.next().unwrap());
                Event {
                    key: key.into(),
                    op: Op::Upsert(value.as_bytes().to_vec()),
                    idempotency_key: idk.into(),
                }
                .encode()
            }
        })
        .collect();
    let raw: usize = rows.iter().map(|r| r.len()).sum();
    // The same sample for both: every k-th row, at most 10,000.
    let step = rows.len().div_ceil(10_000).max(1);
    let sample: Vec<&[u8]> = rows.iter().step_by(step).map(|r| &r[..]).collect();
    let sample_bytes: usize = sample.iter().map(|r| r.len()).sum();
    let t = Instant::now();
    let ctx = Context::train(&sample);
    let train_ms = t.elapsed().as_secs_f64() * 1e3;
    let ctx_bytes = ctx.to_bytes();
    let ctx_stored = zstd(&ctx_bytes, &[], 19).len().min(ctx_bytes.len()) + 1;
    // The dictionary: the best of zstd's trainer at the store's three sizes
    // (repack's `train_dictionaries`), counted with its own stored size.
    let mut best: Option<(usize, Vec<u8>, Vec<usize>)> = None;
    for size in [sample_bytes / 100, sample_bytes / 30, sample_bytes / 10] {
        let size = size.clamp(4096, 112_640);
        let Ok(dict) = zstd::dict::from_samples(&sample, size) else {
            continue;
        };
        let each: Vec<usize> = rows
            .iter()
            .map(|r| {
                let z = zstd(r, &[], level).len() + 1;
                let y = zstd(r, &dict, level).len() + 5;
                (r.len() + 1).min(z).min(y)
            })
            .collect();
        let total = each.iter().sum::<usize>() + zstd(&dict, &[], 19).len() + 1;
        if best.as_ref().is_none_or(|b| total < b.0) {
            best = Some((total, dict, each));
        }
    }
    let (dict_total, dict, each) = best.unwrap();
    let mut packed = 0;
    let mut chosen = ctx_stored + (dict_total - each.iter().sum::<usize>());
    let mut rowpack_only = ctx_stored;
    let mut encoded = Vec::new();
    for (r, z) in rows.iter().zip(&each) {
        match ctx.encode(r) {
            Some(e) => {
                packed += 1;
                rowpack_only += e.len() + 5;
                chosen += (e.len() + 5).min(*z);
                encoded.push(Some(e));
            }
            None => {
                rowpack_only += z;
                chosen += z;
                encoded.push(None);
            }
        }
    }
    // Decode time per row, over the rows rowpack encodes, each way.
    let n = encoded.iter().flatten().count().min(50_000);
    let picks: Vec<(usize, &Vec<u8>)> = encoded
        .iter()
        .enumerate()
        .filter_map(|(i, e)| Some((i, e.as_ref()?)))
        .take(n)
        .collect();
    let frames: Vec<Vec<u8>> = picks
        .iter()
        .map(|(i, _)| zstd(&rows[*i], &dict, level))
        .collect();
    let ddict = zstd::dict::DecoderDictionary::copy(&dict);
    let mut d = zstd::bulk::Decompressor::with_prepared_dictionary(&ddict).unwrap();
    // The fastest of five passes each way, alternating.
    let (mut sink, mut zstd_ns, mut pack_ns) = (0, f64::MAX, f64::MAX);
    for _ in 0..5 {
        let t = Instant::now();
        for (f, (i, _)) in frames.iter().zip(&picks) {
            sink += d.decompress(f, rows[*i].len()).unwrap().len();
        }
        zstd_ns = zstd_ns.min(t.elapsed().as_secs_f64() * 1e9 / n.max(1) as f64);
        let t = Instant::now();
        for (_, e) in &picks {
            sink += ctx.decode(e).unwrap().len();
        }
        pack_ns = pack_ns.min(t.elapsed().as_secs_f64() * 1e9 / n.max(1) as f64);
    }
    // Every encoded row decodes to its bytes.
    let exact = picks
        .iter()
        .all(|(i, e)| ctx.decode(e).unwrap() == rows[*i]);
    let whole = zstd(&text, &[], level).len();
    println!(
        "{{\"file\":\"{file}\",\"kind\":\"{kind}\",\"level\":{level},\"rows\":{},\"raw\":{raw},\"whole_file_zstd\":{whole},\"zstd_dict_per_row\":{dict_total},\"dict\":{},\"rowpack_only\":{rowpack_only},\"planner\":{chosen},\"context\":{ctx_stored},\"shapes\":{},\"encoded\":{packed},\"train_ms\":{train_ms:.1},\"decode_ns_zstd_dict\":{zstd_ns:.0},\"decode_ns_rowpack\":{pack_ns:.0},\"exact\":{exact},\"sink\":{sink}}}",
        rows.len(),
        dict.len(),
        ctx.shapes()
    );
}
