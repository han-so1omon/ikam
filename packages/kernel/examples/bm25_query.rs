//! E338: BM25 query time on E326's READMEs and queries, and every result
//! written out, so two builds can be compared for exactly the same answers.
//!
//! `cargo run --release --example bm25_query -- DIR OUT` (DIR from
//! `prepare.py`; OUT gets one line a query: key and score bits of each hit).

use std::io::Write;
use std::time::Instant;

use ikam_kernel::{Event, MemStore, Repo};

fn main() {
    let dir = std::env::args().nth(1).expect("DIR");
    let out = std::env::args().nth(2).expect("OUT");
    let docs: Vec<(String, String)> = std::fs::read_to_string(format!("{dir}/docs.tsv"))
        .unwrap()
        .lines()
        .map(|l| {
            let (k, t) = l.split_once('\t').unwrap();
            (
                k.to_string(),
                t.split('\u{1e}').collect::<Vec<_>>().join("\n"),
            )
        })
        .collect();
    let queries: Vec<String> = std::fs::read_to_string(format!("{dir}/queries.tsv"))
        .unwrap()
        .lines()
        .map(|l| l.splitn(5, '\t').nth(4).unwrap().to_string())
        .collect();
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let events: Vec<Event> = docs
        .iter()
        .map(|(k, t)| Event::upsert(&format!("t/{k}"), t.as_bytes(), &format!("t{k}")))
        .collect();
    let db = repo.db_append(&empty, &events).unwrap();
    let id = repo.bm25_build(&db, "t/").unwrap();
    let bm25 = repo.bm25_load(&id).unwrap();
    let mut file = std::io::BufWriter::new(std::fs::File::create(out).unwrap());
    for q in &queries {
        let line: Vec<String> = bm25
            .search(q, 100)
            .into_iter()
            .map(|(k, s)| format!("{k}:{:08x}", s.to_bits()))
            .collect();
        writeln!(file, "{}", line.join(" ")).unwrap();
    }
    for k in [10usize, 100] {
        let mut best = f64::MAX;
        for _ in 0..5 {
            let t = Instant::now();
            for q in &queries {
                std::hint::black_box(bm25.search(q, k));
            }
            best = best.min(t.elapsed().as_secs_f64() * 1e3 / queries.len() as f64);
        }
        println!(
            "{{\"docs\":{},\"queries\":{},\"k\":{k},\"query_ms\":{best:.3}}}",
            docs.len(),
            queries.len()
        );
    }
}
