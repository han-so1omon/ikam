//! E324: BM25 scores equal the formula computed row by row, survive the
//! stored index, and rank as expected on a small case.

use ikam_kernel::bm25::{B, K1, terms};
use ikam_kernel::{Event, MemStore, Repo};

struct Lcg(u64);
impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }
}

fn naive(rows: &[(String, String)], query: &str) -> Vec<(String, f32)> {
    let docs: Vec<Vec<String>> = rows.iter().map(|r| terms(&r.1).collect()).collect();
    let n = docs.len() as f32;
    let avg = docs.iter().map(|d| d.len() as f32).sum::<f32>() / n;
    let mut q: Vec<String> = terms(query).collect();
    q.sort();
    q.dedup();
    let mut out: Vec<(String, f32)> = Vec::new();
    for (r, d) in rows.iter().zip(&docs) {
        let mut s = 0.0;
        let mut any = false;
        for t in &q {
            let df = docs.iter().filter(|d| d.contains(t)).count() as f32;
            let tf = d.iter().filter(|x| *x == t).count() as f32;
            if tf == 0.0 {
                continue;
            }
            any = true;
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            s += idf * tf * (K1 + 1.0) / (tf + K1 * (1.0 - B + B * d.len() as f32 / avg));
        }
        if any {
            out.push((r.0.clone(), s));
        }
    }
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

#[test]
fn scores_equal_the_formula_after_storing_the_index() {
    let words = [
        "vector", "index", "graph", "Store", "row", "query", "fusion", "the", "a", "café",
    ];
    let mut rng = Lcg(9);
    let rows: Vec<(String, String)> = (0..300)
        .map(|i| {
            let n = 1 + rng.next() % 40;
            let text: Vec<&str> = (0..n).map(|_| words[(rng.next() % 10) as usize]).collect();
            (
                format!("doc/{i:04}"),
                text.join(if i % 2 == 0 { " " } else { ", " }),
            )
        })
        .collect();
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let mut events: Vec<Event> = rows
        .iter()
        .map(|(k, v)| Event::upsert(k, v.as_bytes(), k))
        .collect();
    events.push(Event::upsert("other/x", b"vector vector vector", "o"));
    let db = repo.db_append(&empty, &events).unwrap();
    let id = repo.bm25_build(&db, "doc/").unwrap();
    let index = repo.bm25_load(&id).unwrap();
    assert_eq!(index.len(), 300);
    for query in [
        "vector",
        "graph store",
        "the a",
        "CAFÉ query query",
        "missing",
        "fusion, row!",
    ] {
        let want = naive(&rows, query);
        let got = index.search(query, 1000);
        assert_eq!(got.len(), want.len(), "{query}");
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(g.0, w.0, "{query}");
            assert!((g.1 - w.1).abs() < 1e-4, "{query}: {} vs {}", g.1, w.1);
        }
    }
}

#[test]
fn rare_terms_and_short_rows_rank_first() {
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let rows = [
        ("r/1", "a vector index for semantic search"),
        (
            "r/2",
            "a graph store with a long description of many unrelated things and words",
        ),
        ("r/3", "semantic search over a graph store"),
    ];
    let events: Vec<Event> = rows
        .iter()
        .map(|(k, v)| Event::upsert(k, v.as_bytes(), k))
        .collect();
    let db = repo.db_append(&empty, &events).unwrap();
    let id = repo.bm25_build(&db, "r/").unwrap();
    let index = repo.bm25_load(&id).unwrap();
    let got: Vec<String> = index
        .search("semantic graph", 3)
        .into_iter()
        .map(|x| x.0)
        .collect();
    assert_eq!(got, ["r/3", "r/1", "r/2"]);
    assert!(index.search("", 3).is_empty());
}
