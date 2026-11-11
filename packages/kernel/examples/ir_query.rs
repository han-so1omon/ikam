//! E203: read one column of every report without rebuilding the reports.
//!
//! `cargo run --release --example ir_query`
//!
//! Three questions over a folder of reports: (1) every row's status, (2)
//! one link's status in every report, (3) how many rows have a dev q under
//! 0.05. Each is answered four ways, timed (best of 7) and checked equal:
//!
//! - **files in the kernel**: read every file, split it into lines and
//!   cells, take the column;
//! - **solid IR**: xz -9 over the whole pack (E192, the smallest the query
//!   can use), unpacked and decoded in full, then the same;
//! - **solid IR, query**: the same bytes, unpacked, then `ir::query` on the
//!   streams in memory (decompression still covers everything);
//! - **IR parts in the kernel**: the skeletons, the files part, the
//!   directory and each stream stored as its own content, repacked at the
//!   archive weight; `ir::query` reads the files part and the streams the
//!   question needs.
//!
//! The query needs every cell in its stream, so `siblings`, `nearest` and
//! `predict` are off; the size each layout costs is given against xz -9
//! over the files, and against the smallest IR without that limit (E200).

mod ir_common;

use std::time::Instant;

use ikam_kernel::Id;
use ikam_kernel::ir::{self, Options, Parts, query};
use ir_common::*;

type Answer = Vec<((usize, usize), Vec<u8>)>;

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

/// The three answers from decoded files.
fn from_files(files: &Files, sk: &[u8], key: &[u8]) -> [Answer; 3] {
    let (mut a, mut b, mut c) = (Vec::new(), Vec::new(), Vec::new());
    for (fi, (n, bytes)) in files.iter().enumerate() {
        for (li, (s, cells)) in ir::lex_lines(n, bytes).into_iter().enumerate() {
            if s != sk {
                continue;
            }
            a.push(((fi, li), cells[9].clone()));
            if cells[0] == key {
                b.push(((fi, li), cells[9].clone()));
            }
            if low_q(&cells[4]) {
                c.push(((fi, li), cells[4].clone()));
            }
        }
    }
    [a, b, c]
}

fn low_q(v: &[u8]) -> bool {
    std::str::from_utf8(v)
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .is_some_and(|q| q < 0.05)
}

/// Question `q` (0, 1 or 2) through `ir::query`.
fn ask(q: usize, ix: &query::Index, id: usize, key: &[u8], fetch: &mut query::Fetch) -> Answer {
    match q {
        0 => query::column(ix, id, 9, fetch).unwrap(),
        1 => query::row(ix, id, 9, key, fetch).unwrap(),
        _ => query::column(ix, id, 4, fetch)
            .unwrap()
            .into_iter()
            .filter(|(_, v)| low_q(v))
            .collect(),
    }
}

fn main() {
    println!(
        "| corpus, layout | stored (of xz -9 over the files) | every status | one link's status | dev q under 0.05 | streams read |"
    );
    println!("|---|---:|---:|---:|---:|---:|");
    for name in ["reports-drift", "reports-random"] {
        let files = corpus(name);
        let base = xz(&solid(&files)).len() as f64;
        let best_any = xz(&ir::pack(&ir::encode(
            &files,
            Options {
                predict: true,
                rows: false,
                nearest: 16,
                ..ALL
            },
        )))
        .len() as f64;
        for (oname, o) in [("columns+numbers", TCN), ("+row keys", TCNR)] {
            let parts = ir::encode(&files, o);
            // the table: the skeleton with 10+ slots and the most lines
            let ix = query::index(&parts, &parts.skeletons).unwrap();
            let counts = ix.counts();
            let id = (0..parts.skeletons.len())
                .filter(|&i| ir::segments(&parts.skeletons[i]).unwrap().len() > 10)
                .max_by_key(|&i| counts[i])
                .unwrap();
            let sk = parts.skeletons[id].clone();
            let mut mem = |i: usize| Ok(parts.streams[i].clone());
            let firsts = query::column(&ix, id, 0, &mut mem).unwrap();
            let key = firsts[firsts.len() / 2].1.clone();
            let truth = from_files(&files, &sk, &key);
            let r = |x: f64| {
                format!(
                    "{x:.0} B ({:.3}; {:.2} of the smallest IR)",
                    x / base,
                    x / best_any
                )
            };

            if oname == "columns+numbers" {
                // files in the kernel
                let repo = kernel_store(&files);
                let ids: Vec<Id> = files.iter().map(|f| Id::of_content(&f.1)).collect();
                let (ms, got) = fastest(|| {
                    let back: Files = files
                        .iter()
                        .zip(&ids)
                        .map(|(f, id)| (f.0.clone(), repo.read_content(id).unwrap()))
                        .collect();
                    from_files(&back, &sk, &key)
                });
                assert!(got == truth);
                println!(
                    "| {name}, files in the kernel | {} | {ms:.1} ms | {ms:.1} ms | {ms:.1} ms | all files |",
                    r(stored(&repo) as f64)
                );
            }

            // solid IR
            let z = xz(&ir::pack(&parts));
            let (ms, got) = fastest(|| {
                let back = ir::decode(&ir::unpack(&unxz(&z)).unwrap()).unwrap();
                from_files(&back, &sk, &key)
            });
            assert!(got == truth);
            println!(
                "| {name}, solid IR {oname}, decoded | {} | {ms:.1} ms | {ms:.1} ms | {ms:.1} ms | all |",
                r(z.len() as f64)
            );
            let mut t = [0.0; 3];
            for (q, tq) in t.iter_mut().enumerate() {
                let (ms, got) = fastest(|| {
                    let p = ir::unpack(&unxz(&z)).unwrap();
                    let ix = query::index(&p, &p.skeletons).unwrap();
                    let mut f = |i: usize| Ok(p.streams[i].clone());
                    ask(q, &ix, id, &key, &mut f)
                });
                assert!(got == truth[q], "{name} {oname} q{q}");
                *tq = ms;
            }
            println!(
                "| {name}, solid IR {oname}, query | same | {:.1} ms | {:.1} ms | {:.1} ms | all decompressed |",
                t[0], t[1], t[2]
            );

            // parts in the kernel
            let mut entries = vec![
                (
                    "ir/skeletons".to_string(),
                    ir::pack(&Parts {
                        skeletons: parts.skeletons.clone(),
                        ..Parts::default()
                    }),
                ),
                ("ir/files".to_string(), parts.files.clone()),
                ("ir/directory".to_string(), parts.directory.clone()),
            ];
            entries.extend(
                parts
                    .streams
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (format!("ir/stream-{i:05}"), s.clone())),
            );
            let repo = kernel_store(&entries);
            let id_of = |b: &[u8]| Id::of_content(b);
            let (skel_id, files_id, dir_id) = (
                id_of(&entries[0].1),
                id_of(&entries[1].1),
                id_of(&entries[2].1),
            );
            let stream_ids: Vec<Id> = parts.streams.iter().map(|s| id_of(s)).collect();
            let mut t = [0.0; 3];
            let mut reads = [0usize; 3];
            for q in 0..3 {
                let (ms, (got, n)) = fastest(|| {
                    let p = Parts {
                        skeletons: ir::unpack(&repo.read_content(&skel_id).unwrap())
                            .unwrap()
                            .skeletons,
                        files: repo.read_content(&files_id).unwrap(),
                        directory: repo.read_content(&dir_id).unwrap(),
                        ..Parts::default()
                    };
                    let ix = query::index(&p, &p.skeletons).unwrap();
                    let mut n = 0;
                    let mut f = |i: usize| {
                        n += 1;
                        Ok(repo.read_content(&stream_ids[i]).unwrap())
                    };
                    let a = ask(q, &ix, id, &key, &mut f);
                    (a, n)
                });
                assert!(got == truth[q], "{name} {oname} parts q{q}");
                t[q] = ms;
                reads[q] = n;
            }
            println!(
                "| {name}, IR {oname}, parts in the kernel | {} | {:.1} ms | {:.1} ms | {:.1} ms | {} / {} / {} of {} |",
                r(stored(&repo) as f64),
                t[0],
                t[1],
                t[2],
                reads[0],
                reads[1],
                reads[2],
                parts.streams.len()
            );
        }
    }
}
