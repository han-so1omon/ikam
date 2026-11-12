//! E248: Tantivy kept as a rebuildable view of a database, on E247's
//! generated documents and update batches. The view is built from the
//! rows read from ikam and caught up from the `kv` diff (delete a changed
//! key, add its new value); each commit's payload names the database's
//! log and `kv` roots it reflects, so an index reopened from disk knows
//! where to resume, and one that is lost or stale is rebuilt from ikam.
//! Answers are checked against the built-in index built at the same state.
//!
//! Run: cargo run --release --features search-tantivy --example search_tantivy_view -- DOCS BATCH...

#[path = "db_common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use common::{Counting, Lcg};
use ikam_kernel::{Arg, Db, Event, Id, Node, Repo, Target};
use tantivy::collector::DocSetCollector;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, STORED, STRING, Schema, TEXT, Value};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term, doc};

const VOCAB: usize = 20_000;
const PREFIX: &str = "doc/";

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

/// A row's value from its `kv` node.
fn value(repo: &Repo<Counting>, n: &Node) -> String {
    let bytes = match &n.target {
        Target::Content(Arg::Range { id, start, len }) => {
            let b = repo.read_content(id).unwrap();
            b[*start as usize..(*start + *len) as usize].to_vec()
        }
        Target::Content(Arg::Whole(id)) => repo.read_content(id).unwrap(),
        _ => panic!("db kv node"),
    };
    String::from_utf8(bytes).unwrap()
}

/// A Tantivy index of the rows under `PREFIX`, and the database state it
/// reflects (also in its last commit's payload: `log kv`).
struct View {
    index: Index,
    writer: IndexWriter,
    reader: IndexReader,
    key: Field,
    body: Field,
    at: (Id, Id),
}

fn schema() -> (Schema, Field, Field) {
    let mut sb = Schema::builder();
    let (k, b) = (
        sb.add_text_field("key", STRING | STORED),
        sb.add_text_field("body", TEXT),
    );
    (sb.build(), k, b)
}

impl View {
    fn open(index: Index) -> View {
        let (_, key, body) = schema();
        let writer = index.writer_with_num_threads(1, 200_000_000).unwrap();
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .unwrap();
        let payload = index.load_metas().unwrap().payload.unwrap_or_default();
        let mut parts = payload.split(' ').map(|s| s.parse::<Id>());
        let at = match (parts.next(), parts.next()) {
            (Some(Ok(log)), Some(Ok(kv))) => (log, kv),
            _ => (Id::of_content(b""), Id::of_content(b"")),
        };
        View {
            index,
            writer,
            reader,
            key,
            body,
            at,
        }
    }

    fn commit(&mut self, db: &Db) {
        let mut c = self.writer.prepare_commit().unwrap();
        c.set_payload(&format!("{} {}", db.log, db.kv));
        c.commit().unwrap();
        self.reader.reload().unwrap();
        self.at = (db.log, db.kv);
    }

    /// Every row of `db`, read from ikam.
    fn build(dir: Option<&Path>, repo: &Repo<Counting>, db: &Db) -> View {
        let index = match dir {
            Some(d) => Index::create_in_dir(d, schema().0).unwrap(),
            None => Index::create_in_ram(schema().0),
        };
        let mut v = View::open(index);
        for n in repo.graph_prefix(&db.kv, PREFIX).unwrap() {
            v.writer
                .add_document(doc!(v.key => n.label.clone(), v.body => value(repo, &n)))
                .unwrap();
        }
        v.commit(db);
        v
    }

    /// Bring the view from the state it reflects up to `db`.
    fn catch_up(&mut self, repo: &Repo<Counting>, db: &Db) -> usize {
        let changes = repo.graph_diff(&self.at.1, &db.kv).unwrap();
        let mut rows = 0;
        for (label, _, after) in changes.iter().filter(|c| c.0.starts_with(PREFIX)) {
            self.writer
                .delete_term(Term::from_field_text(self.key, label));
            if let Some(n) = after {
                self.writer
                    .add_document(doc!(self.key => label.clone(), self.body => value(repo, n)))
                    .unwrap();
            }
            rows += 1;
        }
        self.commit(db);
        rows
    }

    fn search(&self, q: &str) -> Vec<String> {
        let mut qp = QueryParser::for_index(&self.index, vec![self.body]);
        qp.set_conjunction_by_default();
        let s = self.reader.searcher();
        let mut out: Vec<String> = s
            .search(&qp.parse_query(q).unwrap(), &DocSetCollector)
            .unwrap()
            .into_iter()
            .map(|a| {
                let d: TantivyDocument = s.doc(a).unwrap();
                d.get_first(self.key)
                    .and_then(|v| v.as_str())
                    .unwrap()
                    .to_string()
            })
            .collect();
        out.sort();
        out
    }
}

fn main() {
    let args: Vec<usize> = std::env::args()
        .skip(1)
        .map(|a| a.parse().unwrap())
        .collect();
    let (n, batches) = (args[0], args[1..].to_vec());
    // E247's documents and rounds, from the same seed.
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
    let rounds: Vec<(usize, BTreeMap<usize, String>)> = batches
        .iter()
        .flat_map(|&size| std::iter::repeat_n(size, 5))
        .map(|size| {
            (
                size,
                (0..size)
                    .map(|_| (rng.next() as usize % n, text(&mut rng)))
                    .collect(),
            )
        })
        .collect();
    let (mut dbs, mut idem) = (vec![db], 0);
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
    let last = dbs.last().unwrap();
    let built = repo.text_index_build(last, PREFIX).unwrap();
    let want: Vec<Vec<String>> = queries
        .iter()
        .map(|q| repo.text_search(&built, q).unwrap())
        .collect();
    let dir = std::env::temp_dir().join(format!("e248-tantivy-{}", std::process::id()));
    let lag_dir = std::env::temp_dir().join(format!("e248-tantivy-lag-{}", std::process::id()));
    for way in ["disk", "ram", "disk_pruned", "ram_pruned"] {
        repo.prune_diff = way.ends_with("pruned"); // E249
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = way.starts_with("disk").then_some(dir.as_path());
        let t = Instant::now();
        let mut v = View::build(d, &repo, &dbs[0]);
        println!(
            "{{\"docs\":{n},\"way\":\"{way}\",\"build_from_ikam_ms\":{:.0}}}",
            ms(t)
        );
        let mut per: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
        for (k, (size, _)) in rounds.iter().enumerate() {
            let t = Instant::now();
            v.catch_up(&repo, &dbs[k + 1]);
            per.entry(*size).or_default().push(ms(t));
        }
        for (size, t) in &per {
            println!(
                "{{\"docs\":{n},\"way\":\"{way}\",\"batch\":{size},\"catch_up_ms\":{:.1}}}",
                median(t.clone())
            );
        }
        for (q, w) in queries.iter().zip(&want) {
            assert_eq!(&v.search(q), w, "{way}: {q}");
        }
        // Behind by the last five batches: one catch-up over them.
        let behind = dbs.len() - 6;
        let _ = std::fs::remove_dir_all(&lag_dir);
        std::fs::create_dir_all(&lag_dir).unwrap();
        let mut lag = View::build(d.map(|_| lag_dir.as_path()), &repo, &dbs[behind]);
        let t = Instant::now();
        let rows = lag.catch_up(&repo, last);
        println!(
            "{{\"docs\":{n},\"way\":\"{way}\",\"lag_batches\":5,\"rows\":{rows},\"catch_up_ms\":{:.1}}}",
            ms(t)
        );
        for (q, w) in queries.iter().zip(&want) {
            assert_eq!(&lag.search(q), w, "{way} lag: {q}");
        }
        if way == "disk" {
            // Reopen: the payload names the state; a fresh database state
            // that has moved on is caught up from there.
            drop(v);
            let t = Instant::now();
            let reopened = View::open(Index::open_in_dir(&dir).unwrap());
            let open_ms = ms(t);
            assert_eq!(
                reopened.at,
                (last.log, last.kv),
                "payload names the last state"
            );
            for (q, w) in queries.iter().zip(&want) {
                assert_eq!(&reopened.search(q), w, "reopened: {q}");
            }
            println!(
                "{{\"docs\":{n},\"way\":\"disk\",\"reopen_ms\":{open_ms:.1},\"payload_matches\":true}}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&lag_dir);
    println!("{{\"docs\":{n},\"answers_equal_builtin_index\":true}}");
}
