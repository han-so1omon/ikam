//! E198: store prose as it is.
//!
//! `cargo run --release --example ir_prose`
//!
//! E192 found the IR loses on prose: each prose line is a skeleton of its
//! own, so the templates buy nothing, and pulling its numbers out into
//! streams cuts them from the words around them (texts: 1.249 of xz). With
//! `prose`, a file under a tenth of whose bytes are cells is stored as it is,
//! in one raw part with the other such files, and the rest go through the IR
//! as before. For every corpus and every IR variant of E192 to E194, the
//! size with xz -9 over the pack is printed without and with `prose`, against
//! xz -9 over the files. `mixed` is the drift reports and the prose texts in
//! one folder, as a real project folder would hold them.

mod ir_common;

use ikam_kernel::ir::{self, Options};
use ir_common::*;

fn ir_xz(files: &Files, o: Options) -> usize {
    let z = xz(&ir::pack(&ir::encode(files, o)));
    let back = ir::decode(&ir::unpack(&unxz(&z)).unwrap()).unwrap();
    assert!(back == *files, "{o:?}: not exact");
    z.len()
}

fn main() {
    let variants = [
        ("templates", T),
        ("+columns", TC),
        ("+numbers", TCN),
        ("+row keys", TCNR),
        ("+siblings", ALL),
        (
            "numbers+siblings, no row keys",
            Options { rows: false, ..ALL },
        ),
    ];
    let mut mixed = corpus("reports-drift");
    mixed.extend(corpus("texts"));
    let corpora = [
        ("reports-drift", corpus("reports-drift")),
        ("reports-random", corpus("reports-random")),
        ("exports", corpus("exports")),
        ("tables", corpus("tables")),
        ("texts", corpus("texts")),
        ("mixed", mixed),
    ];
    println!("| corpus | xz -9 | best IR | best IR with prose | variant: without / with |");
    println!("|---|---:|---:|---:|---|");
    for (name, files) in &corpora {
        let base = xz(&solid(files)).len() as f64;
        let mut cells = Vec::new();
        let (mut best0, mut best1) = (f64::MAX, f64::MAX);
        for (vn, o) in variants {
            let a = ir_xz(files, o) as f64 / base;
            let b = ir_xz(files, Options { prose: true, ..o }) as f64 / base;
            best0 = best0.min(a);
            best1 = best1.min(b);
            cells.push(format!("{vn}: {a:.3} / {b:.3}"));
        }
        println!(
            "| {name} | {base} B | {best0:.3} | {best1:.3} | {} |",
            cells.join("; ")
        );
    }
}
