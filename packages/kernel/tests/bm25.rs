//! E324: BM25 scores equal the formula computed row by row, survive the
//! stored index, and rank as expected on a small case.

use ikam_kernel::bm25::{B, K1, terms};
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

/// E329: the BM25 view, built and then kept up by edits, scores as the
/// formula does over the rows it is at, with rows added, changed and
/// removed, and rows outside the prefix left out.
#[test]
fn view_kept_by_updates_equals_the_formula() {
    let words = [
        "vector", "index", "graph", "Store", "row", "query", "fusion", "the", "a", "café", "rare",
    ];
    let mut rng = Lcg(11);
    let text = |rng: &mut Lcg| {
        let n = 1 + rng.next() % 30;
        (0..n)
            .map(|_| words[(rng.next() % 11) as usize])
            .collect::<Vec<_>>()
            .join(" ")
    };
    let mut rows: std::collections::BTreeMap<String, String> = (0..200)
        .map(|i| (format!("doc/{i:04}"), text(&mut rng)))
        .collect();
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let mut events: Vec<Event> = rows
        .iter()
        .map(|(k, v)| Event::upsert(k, v.as_bytes(), k))
        .collect();
    events.push(Event::upsert("other/x", b"rare rare rare", "o"));
    let mut db = repo.db_append(&empty, &events).unwrap();
    let blank = repo.bm25_view_empty().unwrap();
    let mut view = repo.bm25_view_update(&blank, &empty, &db, "doc/").unwrap();
    // Live views: one that merges every few updates, one that never does.
    let mut lives = [
        repo.bm25_live_build(&db, "doc/", 25).unwrap(),
        repo.bm25_live_build(&db, "doc/", usize::MAX).unwrap(),
    ];
    let mut merges = 0;
    for round in 0..6 {
        let check =
            |repo: &Repo<MemStore>, view, rows: &std::collections::BTreeMap<String, String>| {
                let list: Vec<(String, String)> =
                    rows.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                for query in [
                    "vector",
                    "graph store",
                    "rare",
                    "CAFÉ query",
                    "missing",
                    "the a row",
                ] {
                    let want = naive(&list, query);
                    let got = repo.bm25_view_search(&view, query, 1000).unwrap();
                    assert_eq!(got.len(), want.len(), "{query}");
                    let w: std::collections::HashMap<&String, f32> =
                        want.iter().map(|(k, s)| (k, *s)).collect();
                    for (k, s) in &got {
                        assert!((w[k] - s).abs() < 1e-4, "{query} {k} {s} {}", w[k]);
                    }
                }
            };
        check(&repo, view, &rows);
        for live in &lives {
            for query in [
                "vector",
                "graph store",
                "rare",
                "CAFÉ query",
                "missing",
                "the a row",
            ] {
                let want = repo.bm25_view_search(&view, query, 1000).unwrap();
                let got = repo.bm25_live_search(live, query, 1000).unwrap();
                assert_eq!(got.len(), want.len(), "{query}");
                for (g, w) in got.iter().zip(&want) {
                    assert!(
                        g.0 == w.0 || (g.1 - w.1).abs() < 1e-5,
                        "{query} {g:?} {w:?}"
                    );
                    assert!((g.1 - w.1).abs() < 1e-4, "{query} {g:?} {w:?}");
                }
            }
        }
        // Edit: change some rows, add some, remove some.
        let mut edits = Vec::new();
        for j in 0..15 {
            let i = rng.next() % 260;
            let key = format!("doc/{i:04}");
            if j % 4 == 0 {
                rows.remove(&key);
                edits.push(Event::remove(&key, false, &format!("r{round}{j}")));
            } else {
                let t = text(&mut rng);
                rows.insert(key.clone(), t.clone());
                edits.push(Event::upsert(&key, t.as_bytes(), &format!("u{round}{j}")));
            }
        }
        edits.push(Event::upsert("other/y", b"vector", &format!("o{round}")));
        let next = repo.db_append(&db, &edits).unwrap();
        view = repo.bm25_view_update(&view, &db, &next, "doc/").unwrap();
        for live in lives.iter_mut() {
            let was = live.base_db;
            *live = repo.bm25_live_update(live, &db, &next, "doc/").unwrap();
            merges += usize::from(live.base_db != was);
        }
        db = next;
    }
    // The kept view is the same graph as one built at the end.
    let rebuilt = repo.bm25_view_update(&blank, &empty, &db, "doc/").unwrap();
    assert_eq!(view, rebuilt);
    // E382: and the same graph as the one-pass build.
    assert_eq!(repo.bm25_view_build(&db, "doc/").unwrap(), view);
    // The merging view merged along the way.
    assert!(merges >= 2, "{merges}");
}

/// E344: a stemmed index finds a word's other forms, remembers its
/// analyzer when stored, and a plain index's bytes are what they were.
#[test]
fn stemmed_index_matches_other_forms_and_plain_bytes_stay() {
    use ikam_kernel::bm25::Analyzer;
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let rows = [
        ("r/1", "Indexes for semantic queries"),
        ("r/2", "a graph store"),
        ("r/3", "indexing tool"),
    ];
    let events: Vec<Event> = rows
        .iter()
        .map(|(k, v)| Event::upsert(k, v.as_bytes(), k))
        .collect();
    let db = repo.db_append(&empty, &events).unwrap();
    let plain = repo.bm25_build(&db, "r/").unwrap();
    let again = repo.bm25_build_with(&db, "r/", Analyzer::Plain).unwrap();
    assert_eq!(plain, again);
    assert!(repo.read_content(&plain).unwrap()[..2] != [0, 1]);
    let porter = repo.bm25_build_with(&db, "r/", Analyzer::Porter).unwrap();
    let plural = repo.bm25_build_with(&db, "r/", Analyzer::Plural).unwrap();
    let keys = |id, q: &str| -> Vec<String> {
        let ix = repo.bm25_load(id).unwrap();
        ix.search(q, 3).into_iter().map(|x| x.0).collect()
    };
    assert_eq!(keys(&plain, "index"), Vec::<String>::new());
    assert_eq!(keys(&plain, "query"), Vec::<String>::new());
    let mut got = keys(&porter, "index");
    got.sort();
    assert_eq!(got, ["r/1", "r/3"]);
    assert_eq!(keys(&porter, "query"), ["r/1"]);
    assert_eq!(keys(&plural, "query"), ["r/1"]);
    assert_eq!(keys(&plural, "indexing"), ["r/3"]);
}

#[test]
fn weights_change_scores_only_when_set() {
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let long = "alpha ".to_string() + &"filler ".repeat(200);
    let events = vec![
        Event::upsert("t/short", b"alpha beta", "a"),
        Event::upsert("t/long", long.as_bytes(), "b"),
    ];
    let db = repo.db_append(&empty, &events).unwrap();
    let id = repo.bm25_build(&db, "t/").unwrap();
    let plain = repo.bm25_load(&id).unwrap();
    assert_eq!(plain.weights(), (K1, B));
    let same = repo.bm25_load(&id).unwrap().with_weights(K1, B);
    assert_eq!(plain.search("alpha", 10), same.search("alpha", 10));
    // No length normalization: both rows hold "alpha" once, so they tie.
    let flat = repo.bm25_load(&id).unwrap().with_weights(K1, 0.0);
    let hits = flat.search("alpha", 10);
    assert_eq!(hits[0].1, hits[1].1);
    // Full normalization pushes the long row further down than B does.
    let full = repo.bm25_load(&id).unwrap().with_weights(K1, 1.0);
    let gap = |h: &[(String, f32)]| h[0].1 - h[1].1;
    assert_eq!(full.search("alpha", 10)[0].0, "t/short");
    assert!(gap(&full.search("alpha", 10)) > gap(&plain.search("alpha", 10)));
}

/// E382: the one-pass build is the same graph, and writes the same
/// contents, as an update from an empty view and an empty database, in a
/// store that holds none of them yet: rows with no terms, repeated terms,
/// non-ASCII text, rows that share their postings, rows outside the
/// prefix and an empty prefix.
#[test]
fn one_pass_build_equals_an_update_from_nothing() {
    let mut rng = Lcg(29);
    let mut events = Vec::new();
    for i in 0..3000 {
        let v = match i % 7 {
            0 => String::new(),
            1 => "-- ... !!".to_string(),
            2 => "Ünïcode ünïcode ÜNÏCODE".to_string(),
            _ => format!(
                "[{i}, \"w{}\", \"w{}\", {}.{}]",
                rng.next() % 50,
                rng.next() % 50,
                rng.next() % 1000,
                rng.next()
            ),
        };
        events.push(Event::upsert(
            &format!("t/a/{i:06}"),
            v.as_bytes(),
            &format!("e{i}"),
        ));
        if i % 10 == 0 {
            events.push(Event::upsert(
                &format!("t/b/{i:06}"),
                b"w1 elsewhere",
                &format!("b{i}"),
            ));
        }
    }
    let build = |prefix: &str, one_pass: bool| {
        let mut repo = Repo::new(MemStore::default());
        let empty = repo.db_empty().unwrap();
        let db = repo.db_append(&empty, &events).unwrap();
        let before: std::collections::BTreeSet<_> =
            repo.store().ids().unwrap().into_iter().collect();
        let view = match one_pass {
            true => repo.bm25_view_build(&db, prefix).unwrap(),
            false => {
                let blank = repo.bm25_view_empty().unwrap();
                repo.bm25_view_update(&blank, &empty, &db, prefix).unwrap()
            }
        };
        let written: std::collections::BTreeMap<_, _> = repo
            .store()
            .ids()
            .unwrap()
            .into_iter()
            .filter(|id| !before.contains(id))
            .map(|id| (id, repo.store().read(&id).unwrap()))
            .collect();
        (view, written)
    };
    for prefix in ["t/a/", "t/", "", "t/none/"] {
        let (got, got_written) = build(prefix, true);
        let (want, want_written) = build(prefix, false);
        assert_eq!(got, want, "{prefix}");
        // The update path also writes the empty view's graph.
        let extra: Vec<_> = want_written
            .keys()
            .filter(|k| !got_written.contains_key(*k))
            .collect();
        assert!(extra.len() <= 1, "{prefix}: {} more", extra.len());
        for (id, bytes) in &got_written {
            assert_eq!(want_written.get(id), Some(bytes), "{prefix} {id}");
        }
    }
}

/// E388: `Bm25Mem`, kept up by puts and removes in memory, gives the same
/// keys and the same score bits as `bm25_live_search` over the same rows,
/// through rounds of edits, at every k.
#[test]
fn in_memory_index_scores_as_the_live_view() {
    use ikam_kernel::bm25::Bm25Mem;
    let words = [
        "vector",
        "index",
        "graph",
        "Store",
        "row",
        "query",
        "fusion",
        "the",
        "a",
        "café",
        "rare",
        "x1",
        "Ünïcode",
    ];
    let mut rng = Lcg(388);
    let text = |rng: &mut Lcg| {
        let n = rng.next() % 40;
        (0..n)
            .map(|_| words[(rng.next() % 13) as usize])
            .collect::<Vec<_>>()
            .join(if rng.next().is_multiple_of(2) {
                " "
            } else {
                ", "
            })
    };
    let mut repo = Repo::new(MemStore::default());
    let empty = repo.db_empty().unwrap();
    let mut mem = Bm25Mem::new();
    let mut events = Vec::new();
    for i in 0..150 {
        let (k, v) = (format!("doc/{i:04}"), text(&mut rng));
        mem.put(&k, &v);
        events.push(Event::upsert(&k, v.as_bytes(), &k));
    }
    let mut db = repo.db_append(&empty, &events).unwrap();
    let mut live = repo.bm25_live_build(&db, "doc/", 20).unwrap();
    let bits = |v: &[(String, f32)]| -> Vec<(String, u32)> {
        v.iter().map(|(s, f)| (s.clone(), f.to_bits())).collect()
    };
    for round in 0..8 {
        assert_eq!(mem.len(), repo.db_scan(&db).unwrap().len());
        // Slots are reused: never more than the 190 keys ever used.
        assert!(mem.slots() <= 190, "{} slots", mem.slots());
        for query in [
            "vector",
            "graph store GRAPH",
            "rare x1",
            "CAFÉ query ünïcode",
            "missing",
            "the a row index fusion",
            "",
        ] {
            for k in [1, 3, 10, 1000] {
                let want = repo.bm25_live_search(&live, query, k).unwrap();
                assert_eq!(
                    bits(&mem.search(query, k)),
                    bits(&want),
                    "{round} {query} {k}"
                );
            }
        }
        let mut edits = Vec::new();
        for j in 0..20 {
            let key = format!("doc/{:04}", rng.next() % 190);
            if j % 3 == 0 {
                mem.remove(&key);
                edits.push(Event::remove(&key, false, &format!("r{round}{j}")));
            } else {
                let t = text(&mut rng);
                mem.put(&key, &t);
                edits.push(Event::upsert(&key, t.as_bytes(), &format!("u{round}{j}")));
            }
        }
        let next = repo.db_append(&db, &edits).unwrap();
        live = repo.bm25_live_update(&live, &db, &next, "doc/").unwrap();
        db = next;
    }
    assert!(!mem.remove("doc/none"));
}
