//! E219: the E216 setting as a set of reports grows.
//!
//! `cargo run --release --example ir_scale`
//!
//! `sample` (E214) cuts only large files, so a set of many small files is
//! judged whole by `route` and `tune`. For drift and random reports at 300,
//! 1,000 and 3,000 files, xz -9 over the pack against xz -9 over the files,
//! and the encode and decode time, untuned and tuned.

mod ir_common;

use std::time::Instant;

use ikam_kernel::ir::{self, Options};
use ir_common::*;

fn main() {
    let setting = Options {
        numbers: true,
        siblings: true,
        nearest: 16,
        predict: true,
        step: true,
        prose: true,
        route: true,
        sort: true,
        transpose: true,
        bases: true,
        sample: 1 << 20,
        ..TC
    };
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let sizes = if args.is_empty() {
        vec![300, 1000, 3000]
    } else {
        args
    };
    println!(
        "| reports | files | bytes | xz -9 | untuned (encode s, decode ms) | tuned (encode s, decode ms) |"
    );
    println!("|---|---:|---:|---:|---|---|");
    for (name, random) in [("drift", false), ("random", true)] {
        for &n in &sizes {
            let files = reports(n, random, 1);
            let bytes: usize = files.iter().map(|(_, b)| b.len()).sum();
            let base = xz(&solid(&files)).len() as f64;
            let cells: Vec<String> = [
                setting,
                Options {
                    tune: true,
                    ..setting
                },
            ]
            .iter()
            .map(|&o| {
                let t = Instant::now();
                let p = ir::encode(&files, o);
                let enc = t.elapsed().as_secs_f64();
                let z = xz(&ir::pack(&p));
                let t = Instant::now();
                let back = ir::decode(&ir::unpack(&unxz(&z)).unwrap()).unwrap();
                let dec = t.elapsed().as_secs_f64() * 1e3;
                assert!(back == files);
                format!("{:.3} ({enc:.1} s, {dec:.0} ms)", z.len() as f64 / base)
            })
            .collect();
            println!(
                "| {name} | {n} | {} KB | {base} B | {} |",
                bytes / 1000,
                cells.join(" | ")
            );
        }
    }
}
