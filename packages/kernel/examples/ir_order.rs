//! E201: lay the streams out by key.
//!
//! `cargo run --release --example ir_order`
//!
//! The IR's streams sit in the pack in the order they first appear: in a
//! report, every column of the first row, then every column of the second.
//! With `group`, the streams an encode adds are sorted by key (line
//! skeleton, slot, row key), so one column of every row sits together and
//! xz finds the matches between them. For each corpus and each variant,
//! xz -9 over the pack without and with `group` (and with `predict`, E200),
//! against xz -9 over the files; then blocks of 20 in the kernel with
//! nearest 16 (E199).

mod ir_common;

use ikam_kernel::ir::{self, Options, Parts};
use ir_common::*;

fn ir_xz(files: &Files, o: Options) -> usize {
    let z = xz(&ir::pack(&ir::encode(files, o)));
    let back = ir::decode(&ir::unpack(&unxz(&z)).unwrap()).unwrap();
    assert!(back == *files, "{o:?}: not exact");
    z.len()
}

fn blocks_kernel(files: &Files, o: Options, per: usize) -> usize {
    let b = ir::encode_blocks(files, o, per, false);
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
        ("+numbers", TCN),
        ("+row keys", TCNR),
        ("+siblings", ALL),
        (
            "numbers+siblings, no row keys",
            Options { rows: false, ..ALL },
        ),
        (
            "+numbers, predict",
            Options {
                predict: true,
                ..TCN
            },
        ),
        (
            "+row keys, predict",
            Options {
                predict: true,
                ..TCNR
            },
        ),
        (
            "+siblings, predict",
            Options {
                predict: true,
                ..ALL
            },
        ),
        (
            "numbers+siblings, predict",
            Options {
                predict: true,
                rows: false,
                ..ALL
            },
        ),
    ];
    let corpora = [
        ("reports-drift", corpus("reports-drift")),
        ("reports-random", corpus("reports-random")),
        ("exports", corpus("exports")),
        ("tables", corpus("tables")),
    ];
    println!("| corpus | xz -9 | best without group | best with group | variant: without / with |");
    println!("|---|---:|---:|---:|---|");
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        let mut cells = Vec::new();
        let (mut b0, mut b1) = (f64::MAX, f64::MAX);
        for (vn, o) in variants {
            let a = ir_xz(files, o) as f64 / base;
            let b = ir_xz(files, Options { group: true, ..o }) as f64 / base;
            b0 = b0.min(a);
            b1 = b1.min(b);
            cells.push(format!("{vn}: {a:.3} / {b:.3}"));
        }
        println!(
            "| {name} | {base} B | {b0:.3} | {b1:.3} | {} |",
            cells.join("; ")
        );
    }
    println!(
        "\nBlocks of 20 in the kernel with nearest 16, best variant (without / with group):\n"
    );
    println!("| corpus | blocks of 20 |");
    println!("|---|---:|");
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        let best = |group: bool| {
            variants
                .iter()
                .map(|(_, o)| {
                    let o = Options {
                        nearest: 16,
                        group,
                        ..*o
                    };
                    blocks_kernel(files, o, 20)
                })
                .min()
                .unwrap() as f64
                / base
        };
        println!("| {name} | {:.3} / {:.3} |", best(false), best(true));
    }
}
