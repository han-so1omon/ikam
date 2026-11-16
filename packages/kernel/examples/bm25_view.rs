//! E329: BM25 as a view of the database, kept up by edits, against the
//! stored index (E324) that is rebuilt and loaded whole.
//!
//! `cargo run --release --example bm25_view -- [DIR]`
//!
//! With DIR (the `docs.tsv` and `queries.tsv` that `hybrid_kernel` reads)
//! it uses that text; without, Zipf-distributed synthetic text. Reports
//! build, update of 1% of rows, rebuild, store growth and query times.

use std::collections::BTreeSet;
use std::time::Instant;

use ikam_kernel::bm25::Bm25Live;
use ikam_kernel::{Event, MemStore, Repo, Store};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
    /// A word rank from a Zipf-like law over `n` words.
    fn zipf(&mut self, n: usize) -> usize {
        let u = (self.next() % 1_000_000) as f64 / 1e6;
        ((n as f64).powf(u) as usize).saturating_sub(1).min(n - 1)
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn stored(repo: &Repo<MemStore>) -> usize {
    let s = repo.store();
    s.ids()
        .unwrap()
        .iter()
        .map(|id| s.stored_len(id).unwrap())
        .sum()
}

fn main() {
    let dir = std::env::args().nth(1);
    let mut rng = Lcg(5);
    let synth = |rng: &mut Lcg| {
        let n = 50 + rng.next() % 250;
        (0..n)
            .map(|_| format!("w{}", rng.zipf(30_000)))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let (docs, queries): (Vec<(String, String)>, Vec<String>) = match &dir {
        Some(d) => (
            std::fs::read_to_string(format!("{d}/docs.tsv"))
                .unwrap()
                .lines()
                .map(|l| {
                    let (k, t) = l.split_once('\t').unwrap();
                    (k.to_string(), t.replace('\u{1e}', "\n"))
                })
                .collect(),
            std::fs::read_to_string(format!("{d}/queries.tsv"))
                .unwrap()
                .lines()
                .map(|l| l.splitn(5, '\t').nth(4).unwrap().to_string())
                .collect(),
        ),
        None => (
            (0..20_000)
                .map(|i| (format!("{i:06}"), synth(&mut rng)))
                .collect(),
            (0..200)
                .map(|_| {
                    (0..3)
                        .map(|_| format!("w{}", rng.zipf(30_000)))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect(),
        ),
    };
    let mut repo = Repo::new(MemStore::default());
    repo.needed_index = true;
    let empty = repo.db_empty().unwrap();
    let events: Vec<Event> = docs
        .iter()
        .map(|(k, t)| Event::upsert(&format!("t/{k}"), t.as_bytes(), k))
        .collect();
    let db = repo.db_append(&empty, &events).unwrap();
    // 1% of rows changed: each gets another row's text.
    let n = docs.len();
    let edits: Vec<Event> = (0..(n / 100).max(1))
        .map(|j| {
            let i = rng.next() as usize % n;
            let other = &docs[rng.next() as usize % n].1;
            Event::upsert(
                &format!("t/{}", docs[i].0),
                other.as_bytes(),
                &format!("e{j}"),
            )
        })
        .collect();
    let next = repo.db_append(&db, &edits).unwrap();
    // The stored index (E324): rebuilt for the new rows, then loaded.
    let before = stored(&repo);
    let t = Instant::now();
    let id = repo.bm25_build(&next, "t/").unwrap();
    let index_build_ms = ms(t);
    let index_bytes = stored(&repo) - before;
    let t = Instant::now();
    let index = repo.bm25_load(&id).unwrap();
    let index_load_ms = ms(t);
    let t = Instant::now();
    let want: Vec<Vec<(String, f32)>> = queries.iter().map(|q| index.search(q, 10)).collect();
    let index_query_ms = ms(t) / queries.len() as f64;
    println!(
        "{{\"text\":\"{}\",\"rows\":{n},\"updated_rows\":{},\"queries\":{},\"index_build_ms\":{index_build_ms:.0},\"index_mb\":{:.2},\"index_load_ms\":{index_load_ms:.0},\"index_query_ms\":{index_query_ms:.3}}}",
        if dir.is_some() { "real" } else { "zipf" },
        edits.len(),
        queries.len(),
        index_bytes as f64 / 1e6
    );
    let same_as = |got: &[(String, f32)], w: &[(String, f32)]| {
        let a: BTreeSet<&String> = got.iter().map(|x| &x.0).collect();
        let b: BTreeSet<&String> = w.iter().map(|x| &x.0).collect();
        usize::from(a == b)
    };
    // One graph, edited in place.
    let base = stored(&repo);
    let t = Instant::now();
    let blank = repo.bm25_view_empty().unwrap();
    let view = repo.bm25_view_update(&blank, &empty, &db, "t/").unwrap();
    let build_ms = ms(t);
    let view_bytes = stored(&repo) - base;
    let before = stored(&repo);
    let t = Instant::now();
    let view2 = repo.bm25_view_update(&view, &db, &next, "t/").unwrap();
    let update_ms = ms(t);
    let update_bytes = stored(&repo) - before;
    let t = Instant::now();
    let rebuilt = repo.bm25_view_update(&blank, &empty, &next, "t/").unwrap();
    let rebuild_ms = ms(t);
    assert_eq!(view2, rebuilt);
    let (mut same, t) = (0, Instant::now());
    for (q, w) in queries.iter().zip(&want) {
        same += same_as(&repo.bm25_view_search(&view2, q, 10).unwrap(), w);
    }
    let query_ms = ms(t) / queries.len() as f64;
    println!(
        "{{\"form\":\"view\",\"build_ms\":{build_ms:.0},\"mb\":{:.2},\"update_ms\":{update_ms:.1},\"update_mb\":{:.2},\"rebuild_ms\":{rebuild_ms:.0},\"query_ms\":{query_ms:.3},\"same_top10_as_index\":{same}}}",
        view_bytes as f64 / 1e6,
        update_bytes as f64 / 1e6
    );
    // Base plus delta, merging past 5% of rows; then a forced merge.
    let merge_rows = n / 20;
    let live = repo.bm25_live_build(&db, "t/", merge_rows).unwrap();
    let before = stored(&repo);
    let t = Instant::now();
    let live2 = repo.bm25_live_update(&live, &db, &next, "t/").unwrap();
    let update_ms = ms(t);
    let update_bytes = stored(&repo) - before;
    assert_eq!(live2.base_db, db);
    let (mut same, t) = (0, Instant::now());
    for (q, w) in queries.iter().zip(&want) {
        same += same_as(&repo.bm25_live_search(&live2, q, 10).unwrap(), w);
    }
    let query_ms = ms(t) / queries.len() as f64;
    // A merge, on rows no list was written for yet: another 1% changed,
    // folded in with the first.
    let edits2: Vec<Event> = (0..(n / 100).max(1))
        .map(|j| {
            let i = rng.next() as usize % n;
            let other = &docs[rng.next() as usize % n].1;
            Event::upsert(
                &format!("t/{}", docs[i].0),
                other.as_bytes(),
                &format!("f{j}"),
            )
        })
        .collect();
    let next2 = repo.db_append(&next, &edits2).unwrap();
    let forced = Bm25Live {
        merge_rows: 0,
        ..live2
    };
    let t = Instant::now();
    let merged = repo.bm25_live_update(&forced, &next, &next2, "t/").unwrap();
    let merge_ms = ms(t);
    assert_eq!(merged.base_db, next2);
    println!(
        "{{\"form\":\"live\",\"merge_rows\":{merge_rows},\"update_ms\":{update_ms:.1},\"update_mb\":{:.3},\"query_ms\":{query_ms:.3},\"same_top10_as_index\":{same},\"merge_ms\":{merge_ms:.0}}}",
        update_bytes as f64 / 1e6
    );
    // From the merged view, 1% updates in a row until the delta passes 5%
    // and merges.
    let (mut live, mut at, mut series) = (
        Bm25Live {
            merge_rows,
            ..merged
        },
        next2,
        Vec::new(),
    );
    for round in 0..7 {
        let edits: Vec<Event> = (0..(n / 100).max(1))
            .map(|j| {
                let i = rng.next() as usize % n;
                let other = &docs[rng.next() as usize % n].1;
                Event::upsert(
                    &format!("t/{}", docs[i].0),
                    other.as_bytes(),
                    &format!("g{round}-{j}"),
                )
            })
            .collect();
        let to = repo.db_append(&at, &edits).unwrap();
        let t = Instant::now();
        let after = repo.bm25_live_update(&live, &at, &to, "t/").unwrap();
        series.push(format!(
            "{{\"ms\":{:.0},\"merged\":{}}}",
            ms(t),
            after.base_db == to
        ));
        (live, at) = (after, to);
    }
    println!(
        "{{\"form\":\"live\",\"updates_in_a_row\":[{}]}}",
        series.join(",")
    );
    // The live view after them ranks as a view built at the same rows.
    let blank = repo.bm25_view_empty().unwrap();
    let fresh = repo.bm25_view_update(&blank, &empty, &at, "t/").unwrap();
    for q in queries.iter().take(50) {
        let a = repo.bm25_live_search(&live, q, 10).unwrap();
        let b = repo.bm25_view_search(&fresh, q, 10).unwrap();
        assert_eq!(same_as(&a, &b), 1, "{q}");
    }
}
