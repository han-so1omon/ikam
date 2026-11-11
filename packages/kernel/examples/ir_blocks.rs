//! E197: IR blocks, so one report is read without decoding every report.
//!
//! `cargo run --release --example ir_blocks [corpus ...]`
//!
//! E192 to E196 left one catch: the IR is one solid pack, so reading one
//! report means decompressing and decoding all of them (12 ms for 300, where
//! the kernel reads a stored file in 0.03 ms). `ir::encode_blocks` cuts the
//! files into blocks of `per` files. One skeleton table is shared; each
//! block's streams are its own, so a read decodes one block. The cost is
//! that each block restarts its number deltas and string dictionaries, and a
//! small block compresses worse on its own.
//!
//! For each block size, three ways to store the blocks: xz -9 per block;
//! zstd -19 per block with one dictionary trained on the blocks (the
//! dictionary counted); and the kernel (each block one content, repacked at
//! the archive weight, which lets the kernel's own deltas and dictionaries
//! reach across blocks). Each row gives the stored bytes against xz -9 over
//! all files, and the time to read the middle file. Every file of every
//! block is checked byte-exact.
//!
//! Then the same with `keyframe`: each block after the first is coded
//! against the streams as the first block left them, so it starts from the
//! first block's strings and last numbers instead of from nothing. A read
//! then decodes the first block and its own (timed cold, both every time).

mod ir_common;

use std::time::Instant;

use ikam_kernel::Id;
use ikam_kernel::ir::{self, Blocks, Options, Parts};
use ir_common::*;

/// The skeleton table and each block as kernel entries.
fn block_entries(b: &Blocks) -> Files {
    let mut out = vec![(
        "ir/skeletons".to_string(),
        ir::pack(&Parts {
            skeletons: b.skeletons.clone(),
            ..Parts::default()
        }),
    )];
    out.extend(
        b.blocks
            .iter()
            .enumerate()
            .map(|(i, p)| (format!("ir/block-{i:05}"), ir::pack(p))),
    );
    out
}

/// Names in file order, one per line: which block holds a file is its
/// position over `per`.
fn name_index(files: &Files) -> Vec<u8> {
    files
        .iter()
        .flat_map(|(n, _)| n.bytes().chain(*b"\n"))
        .collect()
}

/// Decodes `want` from its block `k`, given the stored bytes of the
/// keyframe (block 0, when `b` has one and `k` is not it) and of block `k`,
/// unpacked by `open`. The keyframe is opened and decoded on every read (a
/// cold read; a reader that keeps it pays only for block `k`).
fn read_from(b: &Blocks, k: usize, open: &mut dyn FnMut(usize) -> Parts, want: &str) -> Vec<u8> {
    let mut blocks = vec![Parts::default(); k + 1];
    if b.keyframe && k > 0 {
        blocks[0] = open(0);
    }
    blocks[k] = open(k);
    let one = Blocks {
        per: b.per,
        keyframe: b.keyframe,
        skeletons: b.skeletons.clone(),
        blocks,
    };
    ir::decode_block(&one, k)
        .unwrap()
        .into_iter()
        .find(|(n, _)| n == want)
        .unwrap()
        .1
}

fn fastest<T>(mut f: impl FnMut() -> T) -> (f64, T) {
    let mut best = f64::MAX;
    let mut last = None;
    for _ in 0..7 {
        let t = Instant::now();
        let r = f();
        best = best.min(t.elapsed().as_secs_f64() * 1e3);
        last = Some(r);
    }
    (best, last.unwrap())
}

fn run(name: &str, files: &Files, o: Options, sizes: &[usize], keyframe: bool) {
    let base = xz(&solid(files)).len();
    let mid = files.len() / 2;
    let (want, want_bytes) = (&files[mid].0, &files[mid].1);
    let how = if keyframe {
        "blocks coded against the first block"
    } else {
        "independent blocks"
    };
    println!(
        "\n## {name}, {how}: {} files, xz -9 over all = {base} B\n",
        files.len()
    );
    println!(
        "| files per block | blocks | xz per block | zstd -19 + dict | kernel | read one: xz | zstd | kernel |"
    );
    println!("|---:|---:|---:|---:|---:|---:|---:|---:|");
    let index = xz(&name_index(files)).len();
    for &per in sizes {
        let b = ir::encode_blocks(files, o, per, keyframe);
        assert!(ir::decode_blocks(&b).unwrap() == *files, "{name} per {per}");
        let skel = xz(&ir::pack(&Parts {
            skeletons: b.skeletons.clone(),
            ..Parts::default()
        }));
        let packs: Vec<Vec<u8>> = b.blocks.iter().map(ir::pack).collect();
        let k = mid / b.per;

        // xz per block
        let xs: Vec<Vec<u8>> = packs.iter().map(|p| xz(p)).collect();
        let xz_bytes = skel.len() + index + xs.iter().map(Vec::len).sum::<usize>();
        let (xz_ms, got) =
            fastest(|| read_from(&b, k, &mut |i| ir::unpack(&unxz(&xs[i])).unwrap(), want));
        assert!(got == *want_bytes);

        // zstd -19 per block, one trained dictionary
        let total: usize = packs.iter().map(Vec::len).sum();
        let dict = if packs.len() >= 8 {
            zstd::dict::from_samples(&packs, (total / 10).clamp(1024, 112_640)).unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut c = zstd::bulk::Compressor::with_dictionary(19, &dict).unwrap();
        let zs: Vec<Vec<u8>> = packs.iter().map(|p| c.compress(p).unwrap()).collect();
        let zstd_bytes = skel.len() + index + dict.len() + zs.iter().map(Vec::len).sum::<usize>();
        let mut d = zstd::bulk::Decompressor::with_dictionary(&dict).unwrap();
        let (zstd_ms, got) = fastest(|| {
            read_from(
                &b,
                k,
                &mut |i| ir::unpack(&d.decompress(&zs[i], 1 << 28).unwrap()).unwrap(),
                want,
            )
        });
        assert!(got == *want_bytes);

        // the kernel
        let entries = block_entries(&b);
        let repo = kernel_store(&entries);
        let kernel_bytes = stored(&repo) + index;
        let ids: Vec<Id> = entries[1..].iter().map(|e| Id::of_content(&e.1)).collect();
        let (kernel_ms, got) = fastest(|| {
            read_from(
                &b,
                k,
                &mut |i| ir::unpack(&repo.read_content(&ids[i]).unwrap()).unwrap(),
                want,
            )
        });
        assert!(got == *want_bytes);

        let r = |x: usize| format!("{x} B ({:.3})", x as f64 / base as f64);
        println!(
            "| {per} | {} | {} | {} | {} | {xz_ms:.2} ms | {zstd_ms:.2} ms | {kernel_ms:.2} ms |",
            b.blocks.len(),
            r(xz_bytes),
            r(zstd_bytes),
            r(kernel_bytes)
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let want = |n: &str| args.is_empty() || args.iter().any(|a| a == n);
    let report_sizes = [1, 2, 5, 10, 20, 50, 100, 300];
    for keyframe in [false, true] {
        if want("reports-drift") {
            let c = corpus("reports-drift");
            run("reports-drift", &c, TCNR, &report_sizes, keyframe);
        }
        if want("reports-random") {
            let c = corpus("reports-random");
            run("reports-random", &c, TCNR, &report_sizes, keyframe);
        }
        if want("exports") {
            // each report's md, csv and json are next to each other, so a
            // multiple of 3 keeps siblings in one block
            let sizes = [3, 6, 15, 30, 60, 150, 300];
            run("exports", &corpus("exports"), ALL, &sizes, keyframe);
        }
    }
}
