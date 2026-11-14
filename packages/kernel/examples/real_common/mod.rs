//! Shared by the E256 to E259 harnesses: loading the real-text benchmark
//! that `examples/real_text/prepare.py` writes (READMEs and summaries of
//! OSI-licensed PyPI projects, WordLlama vectors), BM25 over it with
//! Tantivy, and the quality measures. The data stays outside the repository.

#![allow(dead_code)]

use std::collections::{BTreeMap, HashSet};
use std::fs;

use ikam_kernel::vectors::dot;

pub const RS: char = '\u{1e}';

pub struct Query {
    pub task: String,
    pub qid: String,
    pub split: String,
    pub relevant: HashSet<String>,
    pub text: String,
}

pub fn dim(dir: &str) -> usize {
    let m = fs::read_to_string(format!("{dir}/model.json")).unwrap();
    model_num(&m, "dim") as usize
}

/// A number field of model.json (flat keys only).
pub fn model_num(m: &str, field: &str) -> f64 {
    let at = m.find(&format!("\"{field}\":")).unwrap() + field.len() + 3;
    m[at..]
        .trim_start()
        .split([',', '\n', '}'])
        .next()
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// Rows of `docs.tsv`: key, and paragraphs joined by newlines.
pub fn docs(dir: &str) -> Vec<(String, String)> {
    fs::read_to_string(format!("{dir}/docs.tsv"))
        .unwrap()
        .lines()
        .map(|l| {
            let (k, t) = l.split_once('\t').unwrap();
            (k.to_string(), t.replace(RS, "\n"))
        })
        .collect()
}

pub fn vectors(path: &str, dim: usize) -> Vec<Vec<f32>> {
    fs::read(path)
        .unwrap()
        .chunks_exact(4 * dim)
        .map(|c| {
            c.chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect()
        })
        .collect()
}

pub fn queries(dir: &str) -> Vec<Query> {
    fs::read_to_string(format!("{dir}/queries.tsv"))
        .unwrap()
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.splitn(5, '\t').collect();
            Query {
                task: f[0].into(),
                qid: f[1].into(),
                split: f[2].into(),
                relevant: f[3].split(',').map(String::from).collect(),
                text: f[4].into(),
            }
        })
        .collect()
}

/// Release histories: key to its READMEs, oldest first.
pub fn history(dir: &str) -> BTreeMap<String, Vec<String>> {
    let mut h: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for l in fs::read_to_string(format!("{dir}/hist.tsv"))
        .unwrap()
        .lines()
    {
        let f: Vec<&str> = l.splitn(3, '\t').collect();
        h.entry(f[0].into())
            .or_default()
            .push(f[2].replace(RS, "\n"));
    }
    h
}

/// Exact top `k` of `rows` (key, vector) by dot product.
pub fn exact_top(rows: &[(String, Vec<f32>)], q: &[f32], k: usize) -> Vec<(String, f32)> {
    let mut s: Vec<(String, f32)> = rows
        .iter()
        .map(|(key, v)| (key.clone(), dot(q, v)))
        .collect();
    s.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    s.truncate(k);
    s
}

/// nDCG@10 (binary gains), recall@10, recall@100 and reciprocal rank@10.
#[derive(Clone, Copy, Default)]
pub struct Quality {
    pub ndcg10: f64,
    pub recall10: f64,
    pub recall100: f64,
    pub mrr10: f64,
}

pub fn quality(ranked: &[String], rel: &HashSet<String>) -> Quality {
    let gain = |i: usize| 1.0 / (i as f64 + 2.0).log2();
    let dcg: f64 = ranked
        .iter()
        .take(10)
        .enumerate()
        .filter(|(_, k)| rel.contains(*k))
        .map(|(i, _)| gain(i))
        .sum();
    let ideal: f64 = (0..rel.len().min(10)).map(gain).sum();
    let hits = |n: usize| ranked.iter().take(n).filter(|k| rel.contains(*k)).count() as f64;
    let first = ranked.iter().take(10).position(|k| rel.contains(k));
    Quality {
        ndcg10: dcg / ideal,
        recall10: hits(10) / rel.len().min(10) as f64,
        recall100: hits(100) / rel.len().min(100) as f64,
        mrr10: first.map_or(0.0, |p| 1.0 / (p as f64 + 1.0)),
    }
}

pub fn mean(qs: &[Quality]) -> Quality {
    let n = qs.len().max(1) as f64;
    qs.iter().fold(Quality::default(), |a, q| Quality {
        ndcg10: a.ndcg10 + q.ndcg10 / n,
        recall10: a.recall10 + q.recall10 / n,
        recall100: a.recall100 + q.recall100 / n,
        mrr10: a.mrr10 + q.mrr10 / n,
    })
}

#[cfg(feature = "search-tantivy")]
pub mod bm25 {
    use tantivy::collector::TopDocs;
    use tantivy::query::QueryParser;
    use tantivy::schema::{
        IndexRecordOption, STORED, STRING, Schema, TextFieldIndexing, TextOptions, Value,
    };
    use tantivy::{Index, IndexReader, TantivyDocument, doc};

    /// Tantivy over the READMEs, English stemming (`en_stem`) on both
    /// sides, the usual BM25 baseline setting.
    pub struct Bm25 {
        index: Index,
        reader: IndexReader,
        key: tantivy::schema::Field,
        body: tantivy::schema::Field,
    }

    impl Bm25 {
        pub fn build(docs: &[(String, String)]) -> Bm25 {
            let mut sb = Schema::builder();
            let key = sb.add_text_field("key", STRING | STORED);
            let opts = TextOptions::default().set_indexing_options(
                TextFieldIndexing::default()
                    .set_tokenizer("en_stem")
                    .set_index_option(IndexRecordOption::WithFreqsAndPositions),
            );
            let body = sb.add_text_field("body", opts);
            let index = Index::create_in_ram(sb.build());
            let mut w = index.writer_with_num_threads(1, 300_000_000).unwrap();
            for (k, t) in docs {
                w.add_document(doc!(key => k.clone(), body => t.clone()))
                    .unwrap();
            }
            w.commit().unwrap();
            let reader = index.reader().unwrap();
            Bm25 {
                index,
                reader,
                key,
                body,
            }
        }

        pub fn top(&self, q: &str, k: usize) -> Vec<(String, f32)> {
            let s = self.reader.searcher();
            let qp = QueryParser::for_index(&self.index, vec![self.body]);
            let (query, _) = qp.parse_query_lenient(q);
            s.search(&query, &TopDocs::with_limit(k))
                .unwrap()
                .into_iter()
                .map(|(score, a)| {
                    let d: TantivyDocument = s.doc(a).unwrap();
                    let key = d.get_first(self.key).and_then(|v| v.as_str()).unwrap();
                    (key.to_string(), score)
                })
                .collect()
        }
    }
}

/// A query's BM25 and vector top lists, (key, score) best first.
#[cfg(feature = "search-tantivy")]
pub type Lists = Vec<(Vec<(String, f32)>, Vec<(String, f32)>)>;

/// Each query's BM25 and exact-vector top 100 (over `doc_vecs`), the
/// lists both fusion harnesses combine.
#[cfg(feature = "search-tantivy")]
pub fn lists(
    docs: &[(String, String)],
    doc_vecs: &[Vec<f32>],
    queries: &[Query],
    query_vecs: &[Vec<f32>],
) -> Lists {
    let index = bm25::Bm25::build(docs);
    let rows: Vec<(String, Vec<f32>)> = docs
        .iter()
        .zip(doc_vecs)
        .map(|(d, v)| (d.0.clone(), v.clone()))
        .collect();
    queries
        .iter()
        .zip(query_vecs)
        .map(|(q, v)| (index.top(&q.text, 100), exact_top(&rows, v, 100)))
        .collect()
}

pub fn keys(l: &[(String, f32)]) -> Vec<String> {
    l.iter().map(|x| x.0.clone()).collect()
}
