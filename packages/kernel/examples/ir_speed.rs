//! E206: encode, decode and column-query times of the predicting IR.
//!
//! `cargo run --release --example ir_speed`
//!
//! In memory, best of 25, for 300 reports: encode, decode, and the E203
//! "every status" column query, for columns and numbers, with `predict`,
//! and with `predict` and `step`. Run on a branch before and after a change
//! that leaves the format alone (`ir_parity` checks that) to see what it
//! bought.

mod ir_common;

use std::time::Instant;

use ikam_kernel::ir::{self, Options, query};
use ir_common::*;

fn best<T>(mut f: impl FnMut() -> T) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..25 {
        let t = Instant::now();
        std::hint::black_box(f());
        b = b.min(t.elapsed().as_secs_f64() * 1e3);
    }
    b
}

fn main() {
    println!("| corpus | options | encode ms | decode ms | column query ms |");
    println!("|---|---|---:|---:|---:|");
    for name in ["reports-drift", "reports-random", "exports"] {
        let files = corpus(name);
        let p = Options {
            predict: true,
            ..TCN
        };
        for (oname, o) in [
            ("columns, numbers", TCN),
            ("+ predict", p),
            ("+ predict + step", Options { step: true, ..p }),
        ] {
            let enc = best(|| ir::encode(&files, o));
            let parts = ir::encode(&files, o);
            let dec = best(|| ir::decode(&parts).unwrap());
            let ix = query::index(&parts, &parts.skeletons).unwrap();
            let counts = ix.counts();
            let id = (0..parts.skeletons.len())
                .filter(|&i| ir::segments(&parts.skeletons[i]).unwrap().len() > 10)
                .max_by_key(|&i| counts[i])
                .unwrap();
            let q = best(|| {
                let ix = query::index(&parts, &parts.skeletons).unwrap();
                let mut f = |i: usize| Ok(parts.streams[i].clone());
                query::column(&ix, id, 9, &mut f).unwrap()
            });
            println!("| {name} | {oname} | {enc:.1} | {dec:.1} | {q:.2} |");
        }
    }
}
