//! E200: columns that predict each other (generator plus residual).
//!
//! `cargo run --release --example ir_predict`
//!
//! A table often has columns that follow from another column: a link's
//! sensor and target from its id, a client's region from its name, a status
//! from its row's label. With `predict`, each column learns, per earlier
//! cell of its line, which value it last had beside each value of that cell,
//! and how often that held; a cell the best such map gets right is stored as
//! one byte, and the rest as before (the residual). For each corpus and each
//! columns variant, xz -9 over the pack without and with `predict`, against
//! xz -9 over the files; then the same for blocks of 20 in the kernel with
//! `nearest: 16` (E197, E199), where each block restarts its streams, and
//! with a keyframe, whose streams (and what their predictors learned) every
//! later block goes on from.

mod ir_common;

use std::time::Instant;

use ikam_kernel::ir::{self, Options, Parts};
use ir_common::*;

fn ir_xz(files: &Files, o: Options) -> (usize, f64, f64) {
    let t = Instant::now();
    let z = xz(&ir::pack(&ir::encode(files, o)));
    let enc = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let back = ir::decode(&ir::unpack(&unxz(&z)).unwrap()).unwrap();
    let dec = t.elapsed().as_secs_f64() * 1e3;
    assert!(back == *files, "{o:?}: not exact");
    (z.len(), enc, dec)
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
    let variants = [
        ("columns", TC),
        ("+numbers", TCN),
        ("+row keys", TCNR),
        ("+siblings", ALL),
        (
            "numbers+siblings, no row keys",
            Options { rows: false, ..ALL },
        ),
    ];
    let corpora = [
        ("reports-drift", corpus("reports-drift")),
        ("reports-random", corpus("reports-random")),
        ("exports", corpus("exports")),
        ("tables", corpus("tables")),
    ];
    println!(
        "| corpus | xz -9 | best without | best with predict | variant: without / with | encode, decode ms of the best (without / with) |"
    );
    println!("|---|---:|---:|---:|---|---|");
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        let mut cells = Vec::new();
        let mut best = [(f64::MAX, 0.0, 0.0); 2];
        for (vn, o) in variants {
            let r = [o, Options { predict: true, ..o }].map(|o| ir_xz(files, o));
            for (b, x) in best.iter_mut().zip(&r) {
                let v = x.0 as f64 / base;
                if v < b.0 {
                    *b = (v, x.1, x.2);
                }
            }
            cells.push(format!(
                "{vn}: {:.3} / {:.3}",
                r[0].0 as f64 / base,
                r[1].0 as f64 / base
            ));
        }
        println!(
            "| {name} | {base} B | {:.3} | {:.3} | {} | {:.0}, {:.0} / {:.0}, {:.0} |",
            best[0].0,
            best[1].0,
            cells.join("; "),
            best[0].1,
            best[0].2,
            best[1].1,
            best[1].2
        );
    }
    println!(
        "\nBlocks of 20 in the kernel with nearest 16, best variant, against xz -9 over the files:\n"
    );
    println!("| corpus | independent: without / with predict | keyframe: without / with predict |");
    println!("|---|---:|---:|");
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        let best = |predict: bool, keyframe: bool| {
            variants
                .iter()
                .map(|(_, o)| {
                    let o = Options {
                        nearest: 16,
                        predict,
                        ..*o
                    };
                    blocks_kernel(files, o, 20, keyframe)
                })
                .min()
                .unwrap() as f64
                / base
        };
        println!(
            "| {name} | {:.3} / {:.3} | {:.3} / {:.3} |",
            best(false, false),
            best(true, false),
            best(false, true),
            best(true, true)
        );
    }
}
