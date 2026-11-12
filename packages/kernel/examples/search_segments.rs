//! E247: the word index kept as segments (base, delta, dead postings;
//! src/textindex.rs) against E244's single graph and Tantivy, on E244's
//! generated documents and update batches. Every way is checked to answer
//! every query as Tantivy does after the updates.
//!
//! Run: cargo run --release --features search-tantivy --example search_segments -- DOCS MERGE_ABOVE BATCH...

#[path = "db_common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::time::Instant;

use common::{Counting, Lcg};
use ikam_kernel::textindex::{SegmentOpts, Segments};
use ikam_kernel::{Event, Id, Repo};
use tantivy::collector::DocSetCollector;
use tantivy::query::QueryParser;
use tantivy::schema::{STORED, STRING, Schema, TEXT, Value};
use tantivy::{Index, ReloadPolicy, TantivyDocument, Term, doc};

const VOCAB: usize = 20_000;

/// Word `r` of E244's made-up vocabulary.
fn word(mut r: usize) -> String {
    const SYL: [&str; 16] = [
        "ka", "lo", "mi", "ne", "su", "ta", "ri", "po", "ve", "du", "zi", "ho", "ba", "fe", "gu",
        "ny",
    ];
    let mut w = String::new();
    loop {
        w.push_str(SYL[r % 16]);
        r /= 16;
        if r == 0 {
            return w;
        }
    }
}

/// E244's document: 10 to 40 words, word `r` about as common as 1/r.
fn text(rng: &mut Lcg) -> String {
    let n = 10 + rng.next() % 31;
    let words: Vec<String> = (0..n)
        .map(|_| {
            let u = (rng.next() % 1_000_000) as f64 / 1e6;
            word(((VOCAB as f64).powf(u) as usize).saturating_sub(1))
        })
        .collect();
    words.join(" ")
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1e3
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn key(i: usize) -> String {
    format!("doc/{i:010}")
}

/// One way's index: E244's single graph or E247's segments.
#[allow(clippy::large_enum_variant)] // a benchmark's one value
enum Kept {
    View(Id),
    Segments(Segments),
}

/// Tantivy on the same edits, as E244 ran it (one indexing thread), on
/// disk (each commit synced) or in RAM. Returns each query's rows.
fn tantivy(
    way: &str,
    ram: bool,
    docs: &[String],
    rounds: &[(usize, BTreeMap<usize, String>)],
    queries: &[String],
) -> Vec<Vec<String>> {
    let (n, mut docs) = (docs.len(), docs.to_vec());
    let dir = std::env::temp_dir().join(format!("e247-tantivy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut sb = Schema::builder();
    let (kf, bf) = (
        sb.add_text_field("key", STRING | STORED),
        sb.add_text_field("body", TEXT),
    );
    let tindex = match ram {
        true => Index::create_in_ram(sb.build()),
        false => Index::create_in_dir(&dir, sb.build()).unwrap(),
    };
    let t = Instant::now();
    let mut writer = tindex.writer_with_num_threads(1, 200_000_000).unwrap();
    for (i, d) in docs.iter().enumerate() {
        writer
            .add_document(doc!(kf => key(i), bf => d.as_str()))
            .unwrap();
    }
    writer.commit().unwrap();
    let reader = tindex
        .reader_builder()
        .reload_policy(ReloadPolicy::Manual)
        .try_into()
        .unwrap();
    println!(
        "{{\"docs\":{n},\"way\":\"{way}\",\"build_ms\":{:.0}}}",
        ms(t)
    );
    let mut per: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
    for (size, edits) in rounds {
        let t = Instant::now();
        for (i, d) in edits {
            docs[*i] = d.clone();
            writer.delete_term(Term::from_field_text(kf, &key(*i)));
            writer
                .add_document(doc!(kf => key(*i), bf => d.as_str()))
                .unwrap();
        }
        writer.commit().unwrap();
        reader.reload().unwrap();
        per.entry(*size).or_default().push(ms(t));
    }
    for (size, v) in &per {
        println!(
            "{{\"docs\":{n},\"way\":\"{way}\",\"batch\":{size},\"update_ms\":{:.1}}}",
            median(v.clone())
        );
    }
    let tsearch = |q: &str| -> Vec<String> {
        let mut qp = QueryParser::for_index(&tindex, vec![bf]);
        qp.set_conjunction_by_default();
        let s = reader.searcher();
        let mut out: Vec<String> = s
            .search(&qp.parse_query(q).unwrap(), &DocSetCollector)
            .unwrap()
            .into_iter()
            .map(|a| {
                let d: TantivyDocument = s.doc(a).unwrap();
                d.get_first(kf)
                    .and_then(|v| v.as_str())
                    .unwrap()
                    .to_string()
            })
            .collect();
        out.sort();
        out
    };
    let want: Vec<Vec<String>> = queries.iter().map(|q| tsearch(q)).collect();
    for (q, w) in queries.iter().zip(&want) {
        let mut times = vec![];
        for _ in 0..9 {
            let t = Instant::now();
            tsearch(q);
            times.push(ms(t));
        }
        println!(
            "{{\"way\":\"{way}\",\"query\":\"{q}\",\"hits\":{},\"query_ms\":{:.2}}}",
            w.len(),
            median(times)
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
    want
}

/// E251: the vocabulary as an FST (tantivy-fst, Tantivy's term
/// dictionary) against a graph of word labels in the store.
fn term_dictionary(repo: &Repo<Counting>, docs: &[String]) {
    let vocab: std::collections::BTreeSet<String> = docs
        .iter()
        .flat_map(|d| ikam_kernel::textindex::words(d.as_bytes()))
        .collect();
    let raw: usize = vocab.iter().map(String::len).sum();
    let mut b = tantivy_fst::MapBuilder::memory();
    for (i, w) in vocab.iter().enumerate() {
        b.insert(w, i as u64).unwrap();
    }
    let bytes = b.into_inner().unwrap();
    let fst_bytes = bytes.len();
    let fst = tantivy_fst::Map::from_bytes(bytes).unwrap();
    let mut r = Repo::new(repo.store().copy());
    let before = r.store().stored();
    let nodes = vocab
        .iter()
        .map(|w| ikam_kernel::Node {
            label: w.clone(),
            target: ikam_kernel::Target::None,
            edges: Vec::new(),
        })
        .collect();
    let root = r.put_graph_with(nodes, ikam_kernel::Keys::Any).unwrap();
    let graph = r.store().stored() - before;
    let t = Instant::now();
    let mut hits = 0;
    for w in vocab.iter().step_by(97) {
        hits += usize::from(fst.get(w).is_some());
    }
    let fst_ns = t.elapsed().as_nanos() as f64 / hits as f64;
    let t = Instant::now();
    for w in vocab.iter().step_by(97) {
        r.graph_node(&root, w).unwrap().unwrap();
    }
    let graph_ns = t.elapsed().as_nanos() as f64 / hits as f64;
    println!(
        "{{\"words\":{},\"raw_bytes\":{raw},\"fst_bytes\":{},\"graph_bytes\":{graph},\"fst_lookup_ns\":{fst_ns:.0},\"graph_lookup_ns\":{graph_ns:.0}}}",
        vocab.len(),
        fst_bytes
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let n: usize = args[0].parse().unwrap();
    let merge_above: f64 = args[1].parse().unwrap();
    let batches: Vec<usize> = args[2..].iter().map(|a| a.parse().unwrap()).collect();
    let mut rng = Lcg(7);
    let docs: Vec<String> = (0..n).map(|_| text(&mut rng)).collect();
    let mut repo = Repo::new(Counting::default());
    let mut db = repo.db_empty().unwrap();
    for (c, chunk) in docs.chunks(10_000).enumerate() {
        let events: Vec<Event> = chunk
            .iter()
            .enumerate()
            .map(|(j, t)| {
                Event::upsert(
                    &key(c * 10_000 + j),
                    t.as_bytes(),
                    &format!("w{}", c * 10_000 + j),
                )
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
    }
    // Every round's edits, made up front so every way applies the same.
    let mut idem = 0;
    let rounds: Vec<(usize, BTreeMap<usize, String>)> = batches
        .iter()
        .flat_map(|&size| std::iter::repeat_n(size, 5))
        .map(|size| {
            let edits = (0..size)
                .map(|_| (rng.next() as usize % n, text(&mut rng)))
                .collect();
            (size, edits)
        })
        .collect();
    let mut dbs = vec![db];
    for (_, edits) in &rounds {
        let events: Vec<Event> = edits
            .iter()
            .map(|(i, t)| {
                idem += 1;
                Event::upsert(&key(*i), t.as_bytes(), &format!("u{idem}"))
            })
            .collect();
        dbs.push(repo.db_append(dbs.last().unwrap(), &events).unwrap());
    }
    let queries = [
        word(0),
        word(50),
        word(5000),
        format!("{} {}", word(3), word(40)),
        format!("{} {} {}", word(1), word(10), word(100)),
    ];

    term_dictionary(&repo, &docs);
    let want = tantivy("tantivy_disk", false, &docs, &rounds, &queries);
    assert_eq!(want, tantivy("tantivy_ram", true, &docs, &rounds, &queries));

    for way in [
        "view",
        "view_plain",
        "view_pruned",
        "segments",
        "segments_plain",
        "segments_pruned",
        "segments_block",
        "segments_block_pruned",
    ] {
        let mut r = Repo::new(repo.store().copy());
        r.object_templates = false; // derived data (E228's option), as E244
        r.prune_diff = way.ends_with("pruned"); // E249
        let opts = SegmentOpts {
            merge_above,
            plain_writes: way.ends_with("plain"),
            block_base: way.contains("block"),
        };
        let stored0 = r.store().stored();
        let t = Instant::now();
        let mut kept = match way {
            w if w.starts_with("view") => {
                if opts.plain_writes {
                    r.read_weight = f64::INFINITY;
                }
                Kept::View(r.text_index_build(&dbs[0], "doc/").unwrap())
            }
            _ => Kept::Segments(r.text_segments_build(&dbs[0], "doc/", &opts).unwrap()),
        };
        let (build_ms, built_bytes) = (ms(t), r.store().stored() - stored0);
        let (mut per, mut merges) = (BTreeMap::<usize, (Vec<f64>, Vec<f64>)>::new(), 0);
        for (k, (size, _)) in rounds.iter().enumerate() {
            r.store().take();
            let t = Instant::now();
            kept = match kept {
                Kept::View(id) => Kept::View(
                    r.text_index_update(&id, &dbs[k], &dbs[k + 1], "doc/")
                        .unwrap(),
                ),
                Kept::Segments(s) => {
                    let next = r
                        .text_segments_update(&s, &dbs[k], &dbs[k + 1], "doc/", &opts)
                        .unwrap();
                    merges += usize::from(next.base != s.base);
                    Kept::Segments(next)
                }
            };
            let e = per.entry(*size).or_default();
            e.0.push(ms(t));
            e.1.push(r.store().take().reads as f64);
        }
        let grown = r.store().stored() - stored0;
        println!(
            "{{\"docs\":{n},\"way\":\"{way}\",\"merge_above\":{merge_above},\"build_ms\":{build_ms:.0},\"built_bytes\":{built_bytes},\"bytes_after_updates\":{grown},\"merges\":{merges}}}"
        );
        for (size, (t, reads)) in per {
            println!(
                "{{\"docs\":{n},\"way\":\"{way}\",\"batch\":{size},\"update_ms\":{:.1},\"update_reads\":{:.0}}}",
                median(t),
                median(reads)
            );
        }
        let search = |q: &str| match &kept {
            Kept::View(id) => r.text_search(id, q).unwrap(),
            Kept::Segments(s) => r.text_segments_search(s, q).unwrap(),
        };
        for (q, w) in queries.iter().zip(&want) {
            assert_eq!(&search(q), w, "{way}: {q}");
            let mut times = vec![];
            for _ in 0..9 {
                let t = Instant::now();
                search(q);
                times.push(ms(t));
            }
            println!(
                "{{\"way\":\"{way}\",\"query\":\"{q}\",\"hits\":{},\"query_ms\":{:.2}}}",
                w.len(),
                median(times)
            );
        }
        if let Kept::Segments(s) = &kept {
            let t = Instant::now();
            let m = r
                .text_segments_merge_at(s, dbs.last().unwrap(), "doc/", &opts)
                .unwrap();
            let merge_ms = ms(t);
            match m.block {
                Some(b) => assert_eq!(
                    b,
                    r.block_index_build(dbs.last().unwrap(), "doc/").unwrap(),
                    "{way}: merged blocks equal a build"
                ),
                None => assert_eq!(
                    m.base,
                    r.text_index_build(dbs.last().unwrap(), "doc/").unwrap(),
                    "{way}: merged base equals a build"
                ),
            }
            println!(
                "{{\"way\":\"{way}\",\"final_merge_ms\":{merge_ms:.0},\"merged_equals_build\":true}}"
            );
        }
    }
}
