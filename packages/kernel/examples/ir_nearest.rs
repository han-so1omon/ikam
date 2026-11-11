//! E199: diff each file's cells against its nearest earlier file.
//!
//! `cargo run --release --example ir_nearest`
//!
//! `siblings` (E194) diffs a file's cells against the file with its stem (the
//! same table as csv, json and md). `nearest` looks wider: among the last
//! `nearest` files with the same extension (and the sibling), the one that
//! shares the most cells becomes the base, so a rerun that changed a few
//! numbers stores only those. For each corpus, the best IR without it and
//! with windows of 1, 4 and 16, with xz -9 over the pack, against xz -9
//! over the files; the encode time; and the same in the kernel in blocks of
//! 50 (E197), then at other block sizes, with and without a keyframe.
//! `drift-shuffled` is the drift reports in a random order, where
//! the nearest report is rarely the one just before.

mod ir_common;

use std::time::Instant;

use ikam_kernel::ir::{self, Options, Parts};
use ir_common::*;

fn ir_xz(files: &Files, o: Options) -> (usize, f64) {
    let t = Instant::now();
    let z = xz(&ir::pack(&ir::encode(files, o)));
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let back = ir::decode(&ir::unpack(&unxz(&z)).unwrap()).unwrap();
    assert!(back == *files, "{o:?}: not exact");
    (z.len(), ms)
}

/// Blocks of 50 in the kernel (E197), stored bytes.
fn blocks_kernel(files: &Files, o: Options) -> usize {
    blocks_kernel_at(files, o, 50, false)
}

fn blocks_kernel_at(files: &Files, o: Options, per: usize, keyframe: bool) -> usize {
    let b = ir::encode_blocks(files, o, per, keyframe);
    assert!(ir::decode_blocks(&b).unwrap() == *files);
    let mut entries = vec![(
        "ir/skeletons".to_string(),
        ir::pack(&Parts {
            skeletons: b.skeletons.clone(),
            ..Parts::default()
        }),
    )];
    entries.extend(
        b.blocks
            .iter()
            .enumerate()
            .map(|(i, p)| (format!("ir/block-{i:05}"), ir::pack(p))),
    );
    stored(&kernel_store(&entries))
}

fn shuffled(mut files: Files) -> Files {
    let mut rng = Rng(7);
    for i in (1..files.len()).rev() {
        files.swap(i, rng.below(i as u64 + 1) as usize);
    }
    files
}

fn main() {
    let variants = [TCN, TCNR, ALL, Options { rows: false, ..ALL }];
    let corpora = [
        ("reports-drift", corpus("reports-drift")),
        ("drift-shuffled", shuffled(corpus("reports-drift"))),
        ("reports-random", corpus("reports-random")),
        ("exports", corpus("exports")),
        ("tables", corpus("tables")),
    ];
    println!(
        "| corpus | xz -9 | best IR | nearest 1 | nearest 4 | nearest 16 | encode ms (off / 16) | blocks of 50 in the kernel (off / 16) |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---:|");
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        let mut row = Vec::new();
        let mut best_o = [TCN; 4];
        let mut times = [0.0; 4];
        for (j, w) in [0usize, 1, 4, 16].into_iter().enumerate() {
            let mut best = (usize::MAX, 0.0, TCN);
            for o in variants {
                let o = Options { nearest: w, ..o };
                let (b, ms) = ir_xz(files, o);
                if b < best.0 {
                    best = (b, ms, o);
                }
            }
            best_o[j] = best.2;
            times[j] = best.1;
            row.push(format!("{:.3}", best.0 as f64 / base));
        }
        let k0 = blocks_kernel(files, best_o[0]) as f64 / base;
        let k16 = blocks_kernel(files, best_o[3]) as f64 / base;
        println!(
            "| {name} | {base} B | {} | {:.0} / {:.0} | {k0:.3} / {k16:.3} |",
            row.join(" | "),
            times[0],
            times[3]
        );
    }
    println!(
        "\nBlocks in the kernel, best of the four variants, against xz -9 over the files (nearest off / 16; the variant that won):\n"
    );
    println!("| corpus | per block | independent | keyframe |");
    println!("|---|---:|---:|---:|");
    let label = |o: Options| match (o.rows, o.siblings) {
        (true, true) => "rows+siblings",
        (true, false) => "rows",
        (false, true) => "siblings",
        (false, false) => "numbers",
    };
    for (name, sizes) in [("reports-drift", [20, 50, 100]), ("exports", [15, 30, 60])] {
        let files = corpus(name);
        let base = xz(&solid(&files)).len() as f64;
        for per in sizes {
            let cell = |keyframe: bool| {
                let best = |w: usize| {
                    variants
                        .iter()
                        .map(|&o| {
                            let o = Options { nearest: w, ..o };
                            (blocks_kernel_at(&files, o, per, keyframe), label(o))
                        })
                        .min()
                        .unwrap()
                };
                let (a, la) = best(0);
                let (b, lb) = best(16);
                format!(
                    "{:.3} / {:.3} ({la}; {lb})",
                    a as f64 / base,
                    b as f64 / base
                )
            };
            println!("| {name} | {per} | {} | {} |", cell(false), cell(true));
        }
    }
    // the read time of the best small-block setting
    let files = corpus("reports-drift");
    let o = Options { nearest: 16, ..TCN };
    let b = ir::encode_blocks(&files, o, 20, false);
    let mid = files.len() / 2;
    let mut best = f64::MAX;
    for _ in 0..7 {
        let t = Instant::now();
        let got = ir::decode_block(&b, mid / 20).unwrap();
        assert!(got.iter().any(|f| *f == files[mid]));
        best = best.min(t.elapsed().as_secs_f64() * 1e3);
    }
    println!(
        "\nreports-drift, blocks of 20, numbers and nearest 16: one report read (block decode, in memory) {best:.2} ms"
    );
}
