//! E204: number columns that step (a counter, a steady trend).
//!
//! `cargo run --release --example ir_step`
//!
//! `predict` (E200) gets a column right when it repeats the value it last
//! had beside some earlier cell of its line. A counter (a report's "earlier
//! runs", one more each time the link runs) never repeats, and it was the
//! largest stream left in the drift reports (1,036 of 9,152 bytes). With
//! `step`, a number column also remembers, beside each value of each
//! earlier cell, the step it took last time, and a value that is the last
//! one plus that step is one byte. For each corpus, xz -9 over the pack of
//! each predict variant without and with `step`, against xz -9 over the
//! files, the decode time, and blocks of 20 in the kernel with a keyframe.

mod ir_common;

use std::time::Instant;

use ikam_kernel::ir::{self, Options, Parts};
use ir_common::*;

fn ir_xz(files: &Files, o: Options) -> (usize, f64) {
    let z = xz(&ir::pack(&ir::encode(files, o)));
    let t = Instant::now();
    let back = ir::decode(&ir::unpack(&unxz(&z)).unwrap()).unwrap();
    let dec = t.elapsed().as_secs_f64() * 1e3;
    assert!(back == *files, "{o:?}: not exact");
    (z.len(), dec)
}

fn blocks_kernel(files: &Files, o: Options, per: usize, keyframe: bool) -> usize {
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

fn main() {
    let p = |o: Options| Options { predict: true, ..o };
    let variants = [
        ("numbers", p(TCN)),
        ("numbers, row keys", p(TCNR)),
        ("numbers, siblings", p(Options { rows: false, ..ALL })),
        (
            "numbers, siblings, nearest 16",
            p(Options {
                rows: false,
                nearest: 16,
                ..ALL
            }),
        ),
    ];
    let corpora = [
        ("reports-drift", corpus("reports-drift")),
        ("reports-random", corpus("reports-random")),
        ("exports", corpus("exports")),
        ("tables", corpus("tables")),
    ];
    println!(
        "| corpus | xz -9 | best predict | best with step | variant: without / with | decode ms of the best (without / with) |"
    );
    println!("|---|---:|---:|---:|---|---|");
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        let mut cells = Vec::new();
        let mut best = [(f64::MAX, 0.0); 2];
        for (vn, o) in variants {
            let r = [o, Options { step: true, ..o }].map(|o| ir_xz(files, o));
            for (b, x) in best.iter_mut().zip(&r) {
                let v = x.0 as f64 / base;
                if v < b.0 {
                    *b = (v, x.1);
                }
            }
            cells.push(format!(
                "{vn}: {:.3} / {:.3}",
                r[0].0 as f64 / base,
                r[1].0 as f64 / base
            ));
        }
        println!(
            "| {name} | {base} B | {:.3} | {:.3} | {} | {:.0} / {:.0} |",
            best[0].0,
            best[1].0,
            cells.join("; "),
            best[0].1,
            best[1].1
        );
    }
    println!(
        "\nBlocks of 20 in the kernel, keyframe, nearest 16, best variant (without / with step):\n"
    );
    println!("| corpus | blocks of 20 |");
    println!("|---|---:|");
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        let best = |step: bool| {
            variants
                .iter()
                .map(|(_, o)| {
                    let o = Options {
                        nearest: 16,
                        step,
                        ..*o
                    };
                    blocks_kernel(files, o, 20, true)
                })
                .min()
                .unwrap() as f64
                / base
        };
        println!("| {name} | {:.3} / {:.3} |", best(false), best(true));
    }
}
