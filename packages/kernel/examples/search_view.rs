//! E244: a word index kept as a view (src/textindex.rs) against Tantivy
//! on the same generated documents. Both split words the same way, and
//! every query's rows are checked to be equal.
//!
//! Run: cargo run --release --features search-tantivy --example search_view -- [docs] [batch...]

#[path = "db_common/mod.rs"]
mod common;

use std::collections::HashSet;
use std::time::Instant;

use common::{Counting, Lcg};
use ikam_kernel::{Db, Event, Repo};
use tantivy::collector::DocSetCollector;
use tantivy::query::QueryParser;
use tantivy::schema::{STORED, STRING, Schema, TEXT, Value};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term, doc};

const VOCAB: usize = 20_000;

/// Word `r` of a made-up vocabulary.
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

/// A document of 10 to 40 words, ranked roughly as in natural text (word
/// `r` about as common as 1/r).
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

struct Tv {
    index: Index,
    writer: IndexWriter,
    reader: IndexReader,
    key: tantivy::schema::Field,
    body: tantivy::schema::Field,
    dir: std::path::PathBuf,
}

impl Tv {
    fn search(&self, q: &str) -> Vec<String> {
        let mut qp = QueryParser::for_index(&self.index, vec![self.body]);
        qp.set_conjunction_by_default();
        let searcher = self.reader.searcher();
        let mut out: Vec<String> = searcher
            .search(&qp.parse_query(q).unwrap(), &DocSetCollector)
            .unwrap()
            .into_iter()
            .map(|a| {
                let d: TantivyDocument = searcher.doc(a).unwrap();
                d.get_first(self.key)
                    .and_then(|v| v.as_str())
                    .unwrap()
                    .to_string()
            })
            .collect();
        out.sort();
        out
    }

    fn bytes(&self) -> u64 {
        std::fs::read_dir(&self.dir)
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().len())
            .sum()
    }
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let n = args.first().copied().unwrap_or(50_000);
    let batches = if args.len() > 1 {
        args[1..].to_vec()
    } else {
        vec![10, 100, 1000]
    };
    let mut rng = Lcg(7);
    let mut docs: Vec<String> = (0..n).map(|_| text(&mut rng)).collect();
    let key = |i: usize| format!("doc/{i:010}");

    // The table, as database events.
    let mut repo = Repo::new(Counting::default());
    let mut db = repo.db_empty().unwrap();
    for (c, chunk) in docs.chunks(10_000).enumerate() {
        let events: Vec<Event> = chunk
            .iter()
            .enumerate()
            .map(|(j, t)| {
                let i = c * 10_000 + j;
                Event::upsert(&key(i), t.as_bytes(), &format!("w{i}"))
            })
            .collect();
        db = repo.db_append(&db, &events).unwrap();
    }
    repo.object_templates = false; // derived data (E228's option)
    let table_bytes = repo.store().stored();

    // Built-in: build.
    let t = Instant::now();
    let mut index = repo.text_index_build(&db, "doc/").unwrap();
    let ik_build = ms(t);
    let ik_bytes = repo.store().stored() - table_bytes;
    repo.store().take();

    // Tantivy: build, one indexing thread as the built-in index has.
    let dir = std::env::temp_dir().join(format!("e244-tantivy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut sb = Schema::builder();
    let kf = sb.add_text_field("key", STRING | STORED);
    let bf = sb.add_text_field("body", TEXT);
    let tindex = Index::create_in_dir(&dir, sb.build()).unwrap();
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
    let tv_build = ms(t);
    let mut tv = Tv {
        index: tindex,
        writer,
        reader,
        key: kf,
        body: bf,
        dir: dir.clone(),
    };
    let tv_bytes = tv.bytes();
    println!(
        "{{\"docs\":{n},\"table_bytes\":{table_bytes},\"builtin_build_ms\":{ik_build:.0},\"builtin_index_bytes\":{ik_bytes},\"tantivy_build_ms\":{tv_build:.0},\"tantivy_index_bytes\":{tv_bytes}}}"
    );

    // Queries: one common word, a middling one, a rare one, and two ANDs.
    let queries = [
        word(0),
        word(50),
        word(5000),
        format!("{} {}", word(3), word(40)),
        format!("{} {} {}", word(1), word(10), word(100)),
    ];
    let check = |repo: &Repo<Counting>, index: &ikam_kernel::Id, tv: &Tv| {
        for q in &queries {
            assert_eq!(repo.text_search(index, q).unwrap(), tv.search(q), "{q}");
        }
    };
    check(&repo, &index, &tv);
    for q in &queries {
        let (mut a, mut b) = (vec![], vec![]);
        let hits = repo.text_search(&index, q).unwrap().len();
        repo.store().take();
        for _ in 0..9 {
            let t = Instant::now();
            repo.text_search(&index, q).unwrap();
            a.push(ms(t));
            let t = Instant::now();
            tv.search(q);
            b.push(ms(t));
        }
        let reads = repo.store().take().reads / 9;
        println!(
            "{{\"query\":\"{q}\",\"hits\":{hits},\"builtin_ms\":{:.2},\"builtin_reads\":{reads},\"tantivy_ms\":{:.2}}}",
            median(a),
            median(b)
        );
    }

    // Updates: a batch of edits, then each index is brought up to date
    // (Tantivy: delete and re-add, commit, reload).
    let mut idem = 0;
    for &size in &batches {
        let (mut a, mut b, mut reads) = (vec![], vec![], vec![]);
        for _ in 0..5 {
            let mut events = Vec::new();
            let mut touched = HashSet::new();
            for _ in 0..size {
                idem += 1;
                let i = rng.next() as usize % docs.len();
                if !touched.insert(i) {
                    continue;
                }
                docs[i] = text(&mut rng);
                events.push(Event::upsert(
                    &key(i),
                    docs[i].as_bytes(),
                    &format!("u{idem}"),
                ));
            }
            let next: Db = repo.db_append(&db, &events).unwrap();
            repo.store().take();
            let t = Instant::now();
            index = repo.text_index_update(&index, &db, &next, "doc/").unwrap();
            a.push(ms(t));
            reads.push(repo.store().take().reads as f64);
            db = next;
            let t = Instant::now();
            for i in &touched {
                tv.writer.delete_term(Term::from_field_text(kf, &key(*i)));
                tv.writer
                    .add_document(doc!(kf => key(*i), bf => docs[*i].as_str()))
                    .unwrap();
            }
            tv.writer.commit().unwrap();
            tv.reader.reload().unwrap();
            b.push(ms(t));
        }
        check(&repo, &index, &tv);
        println!(
            "{{\"batch\":{size},\"builtin_update_ms\":{:.1},\"builtin_reads\":{:.0},\"tantivy_update_ms\":{:.1}}}",
            median(a),
            median(reads),
            median(b)
        );
    }
    // The kept index equals one built now.
    assert_eq!(index, repo.text_index_build(&db, "doc/").unwrap());
    println!(
        "{{\"final_check\":\"updated index equals a fresh build; all queries equal Tantivy's\",\"tantivy_index_bytes_after\":{}}}",
        tv.bytes()
    );
    let _ = std::fs::remove_dir_all(&dir);
}
