//! E241: a large append-only table as row events and as column blocks.
//! `cargo run --release --example column_blocks -- [N...]`
//!
//! Generates N rows (a time, an id, a two-place reading, a count, a status
//! word, as CSV), then stores them five ways in fresh in-memory stores and
//! prints one JSON line each: stored bytes per row, write time, the time to
//! sum one column over every row, and the time to read every row back.
//! Every way's rows are checked byte for byte against the originals.

mod db_common;

use std::time::Instant;

use db_common::{Counting, Lcg, row};
use ikam_kernel::{Event, Repo, colblock};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

/// Sum of the reading field (third), in hundredths.
fn reading(v: &[u8]) -> i64 {
    let s = std::str::from_utf8(v).unwrap();
    let (w, f) = s.split_once('.').unwrap();
    w.parse::<i64>().unwrap() * 100 + f.parse::<i64>().unwrap()
}

fn field(r: &[u8], j: usize) -> &[u8] {
    r.split(|&b| b == b',').nth(j).unwrap()
}

fn line(n: usize, way: &str, stored: usize, write: f64, scan: f64, read: f64) {
    println!(
        "{{\"rows\":{n},\"way\":\"{way}\",\"stored\":{stored},\"bytes_per_row\":{:.2},\"write_ms\":{write:.1},\"sum_column_ms\":{scan:.1},\"read_rows_ms\":{read:.1}}}",
        stored as f64 / n as f64
    );
}

fn events(rows: &[Vec<u8>], want: i64) {
    let mut repo = Repo::new(Counting::default());
    let mut db = repo.db_empty().unwrap();
    let t = Instant::now();
    for (b, chunk) in rows.chunks(10_000).enumerate() {
        // A row per event, keyed by its row number (as E240's row ids).
        let batch: Vec<Event> = chunk
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let n = b * 10_000 + i;
                Event::upsert(&format!("t/{n:020}"), r, &format!("r{n}"))
            })
            .collect();
        db = repo.db_append(&db, &batch).unwrap();
    }
    let write = ms(t);
    let t = Instant::now();
    let all = repo.db_scan(&db).unwrap();
    let sum: i64 = all.iter().map(|(_, v)| reading(field(v, 2))).sum();
    let scan = ms(t);
    assert_eq!(sum, want);
    let back: Vec<Vec<u8>> = all.into_iter().map(|p| p.1).collect();
    assert_eq!(back, rows);
    line(
        rows.len(),
        "row_events",
        repo.store().stored(),
        write,
        scan,
        scan,
    );
}

fn one_file(rows: &[Vec<u8>], want: i64) {
    let mut repo = Repo::new(Counting::default());
    let text = rows.join(&b'\n');
    let t = Instant::now();
    let id = repo.put_content(&text).unwrap().id;
    let write = ms(t);
    let t = Instant::now();
    let back = repo.read_content(&id).unwrap();
    let sum: i64 = back
        .split(|&b| b == b'\n')
        .map(|r| reading(field(r, 2)))
        .sum();
    let scan = ms(t);
    assert_eq!((sum, back), (want, text));
    line(
        rows.len(),
        "one_csv_file",
        repo.store().stored(),
        write,
        scan,
        scan,
    );
}

fn zstd_reference(rows: &[Vec<u8>]) {
    let text = rows.join(&b'\n');
    let t = Instant::now();
    let packed = zstd::bulk::compress(&text, 19).unwrap();
    let write = ms(t);
    let t = Instant::now();
    let back = zstd::bulk::decompress(&packed, text.len()).unwrap();
    let read = ms(t);
    assert_eq!(back, text);
    line(
        rows.len(),
        "zstd19_csv_reference",
        packed.len(),
        write,
        f64::NAN,
        read,
    );
}

fn blocks(rows: &[Vec<u8>], want: i64, size: usize) {
    let mut repo = Repo::new(Counting::default());
    let mut db = repo.db_empty().unwrap();
    let t = Instant::now();
    let mut keys = Vec::new();
    for (b, chunk) in rows.chunks(size).enumerate() {
        let refs: Vec<&[u8]> = chunk.iter().map(Vec::as_slice).collect();
        let key = format!("t/blocks/{:020}", b * size);
        db = repo.db_append_block(&db, &key, &refs, b',', &key).unwrap();
        keys.push(key);
    }
    let write = ms(t);
    let t = Instant::now();
    let mut sum = 0;
    for k in &keys {
        let block = repo.db_get(&db, k).unwrap().unwrap();
        sum += colblock::column(&block, 2)
            .unwrap()
            .iter()
            .map(|v| reading(v))
            .sum::<i64>();
    }
    let scan = ms(t);
    assert_eq!(sum, want);
    let t = Instant::now();
    let mut back = Vec::new();
    for k in &keys {
        back.extend(colblock::decode(&repo.db_get(&db, k).unwrap().unwrap()).unwrap());
    }
    let read = ms(t);
    assert_eq!(back, rows);
    line(
        rows.len(),
        &format!("column_blocks_{size}"),
        repo.store().stored(),
        write,
        scan,
        read,
    );
}

fn main() {
    let sizes: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let sizes = if sizes.is_empty() {
        vec![100_000]
    } else {
        sizes
    };
    for n in sizes {
        let mut rng = Lcg(42);
        let rows: Vec<Vec<u8>> = (0..n as u64)
            .map(|i| row(&mut rng, i).into_bytes())
            .collect();
        let want: i64 = rows.iter().map(|r| reading(field(r, 2))).sum();
        let text_bytes: usize = rows.iter().map(|r| r.len() + 1).sum();
        println!(
            "{{\"rows\":{n},\"way\":\"csv_text\",\"stored\":{text_bytes},\"bytes_per_row\":{:.2}}}",
            text_bytes as f64 / n as f64
        );
        zstd_reference(&rows);
        one_file(&rows, want);
        for size in [4096, 65536] {
            blocks(&rows, want, size);
        }
        events(&rows, want);
    }
}
