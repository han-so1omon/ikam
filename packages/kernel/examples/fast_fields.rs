//! E252: Tantivy's fast-field codecs in column blocks, against E241's
//! zigzag deltas in zstd 19, on E241's generated rows (`time,id,reading,
//! count,status`) in blocks of 65,536 rows.
//! `cargo run --release --example fast_fields -- [N...]`
//!
//! For each way: block bytes per row and per column, encode time, the time
//! to sum the `reading` column over every block (`colblock::ints`), and the
//! time of 10,000 single-value reads of `count` at random rows
//! (`colblock::value`). Every way's rows are checked against the originals.
//! Synthetic data only.

mod db_common;

use std::time::Instant;

use db_common::{Lcg, row};
use ikam_kernel::colblock::{self, ColOpts};

const BLOCK: usize = 65_536;

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// Bytes of column `j` in a block: the block less the block without it.
fn column_bytes(rows: &[&[u8]], opts: &ColOpts, j: usize) -> usize {
    let whole = colblock::encode_with(rows, b',', opts).unwrap().len();
    let fields: Vec<Vec<u8>> = rows
        .iter()
        .map(|r| {
            let mut f: Vec<&[u8]> = r.split(|&b| b == b',').collect();
            f[j] = b"";
            f.join(&b',')
        })
        .collect();
    let refs: Vec<&[u8]> = fields.iter().map(Vec::as_slice).collect();
    whole - colblock::encode_with(&refs, b',', opts).unwrap().len()
}

fn main() {
    let sizes: Vec<u64> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    for n in sizes {
        let mut rng = Lcg(42);
        let rows: Vec<Vec<u8>> = (0..n).map(|i| row(&mut rng, i).into_bytes()).collect();
        let want: i64 = rows
            .iter()
            .map(|r| {
                let f = std::str::from_utf8(r).unwrap().split(',').nth(2).unwrap();
                let (w, d) = f.split_once('.').unwrap();
                w.parse::<i64>().unwrap() * 100 + d.parse::<i64>().unwrap()
            })
            .sum();
        let ways = [
            ("e241", ColOpts::default()),
            (
                "fast_as_is",
                ColOpts {
                    fast_numbers: true,
                    compress_fast: false,
                },
            ),
            (
                "fast_zstd",
                ColOpts {
                    fast_numbers: true,
                    compress_fast: true,
                },
            ),
        ];
        for (way, opts) in ways {
            let t = Instant::now();
            let blocks: Vec<Vec<u8>> = rows
                .chunks(BLOCK)
                .map(|c| {
                    let refs: Vec<&[u8]> = c.iter().map(Vec::as_slice).collect();
                    colblock::encode_with(&refs, b',', &opts).unwrap()
                })
                .collect();
            let encode_ms = ms(t);
            let bytes: usize = blocks.iter().map(Vec::len).sum();
            let back: Vec<Vec<u8>> = blocks
                .iter()
                .flat_map(|b| colblock::decode(b).unwrap())
                .collect();
            assert_eq!(back, rows, "{way}: rows read back");
            let t = Instant::now();
            let sum: i64 = blocks
                .iter()
                .map(|b| colblock::ints(b, 2).unwrap().iter().sum::<i64>())
                .sum();
            let sum_ms = ms(t);
            assert_eq!(sum, want, "{way}: sum of readings");
            let mut r = Lcg(9);
            let t = Instant::now();
            let reads = 10_000;
            for _ in 0..reads {
                let i = r.next() as usize % rows.len();
                let v = colblock::value(&blocks[i / BLOCK], 3, i % BLOCK).unwrap();
                assert!(!v.is_empty());
            }
            let point_us = ms(t) * 1e3 / reads as f64;
            let first: Vec<&[u8]> = rows[..BLOCK.min(rows.len())]
                .iter()
                .map(Vec::as_slice)
                .collect();
            let per_col: Vec<String> = (0..5)
                .map(|j| {
                    format!(
                        "{:.3}",
                        column_bytes(&first, &opts, j) as f64 / first.len() as f64
                    )
                })
                .collect();
            println!(
                "{{\"rows\":{n},\"way\":\"{way}\",\"bytes_per_row\":{:.3},\"column_bytes_per_row\":[{}],\"encode_ms\":{encode_ms:.0},\"sum_reading_ms\":{sum_ms:.2},\"point_read_us\":{point_us:.1}}}",
                bytes as f64 / n as f64,
                per_col.join(",")
            );
        }
    }
}
