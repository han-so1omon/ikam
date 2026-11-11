//! E206 check: the pack bytes of every corpus under every predict option set,
//! hashed, so a change that should not touch the format can be checked.
//!
//! `cargo run --release --example ir_parity`

mod ir_common;

use ikam_kernel::ir::{self, Options};
use ir_common::*;

fn main() {
    for name in [
        "reports-drift",
        "reports-random",
        "exports",
        "tables",
        "texts",
    ] {
        let files = corpus(name);
        for b in 0..32u32 {
            let o = Options {
                predict: true,
                step: b & 1 != 0,
                rows: b & 2 != 0,
                siblings: b & 4 != 0,
                nearest: if b & 8 != 0 { 16 } else { 0 },
                prose: b & 16 != 0,
                ..TCN
            };
            let p = ir::encode(&files, o);
            assert!(ir::decode(&p).unwrap() == files);
            let blocks = ir::encode_blocks(&files, o, 20, true);
            let mut all = ir::pack(&p);
            blocks.blocks.iter().for_each(|x| all.extend(ir::pack(x)));
            println!("{name} {b:2} {}", blake3::hash(&all).to_hex());
        }
    }
}
