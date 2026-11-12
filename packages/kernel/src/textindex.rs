//! A full-text index kept as a view of the database (E244): an inverted
//! index from each word to the rows that hold it, stored as a graph with
//! one node `word US rowkey` per posting (US is U+001F), so the postings
//! of a word are one prefix and an update edits only the postings of the
//! rows that changed. Words are split as Tantivy's default tokenizer
//! splits them (runs of alphanumeric characters, lowercased, words over
//! 40 bytes dropped), so the two can be compared on the same rows.
//!
//! An update equals a build from scratch (tests/textindex.rs). A query is
//! a conjunction: the rows holding every word.
//!
//! Opt-in: nothing in the kernel indexes text unless a caller asks. This
//! is word search; the trigram index in search.rs finds any substring.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Db, Error, Id, Keys, Node, Repo, Store, Target};

const US: char = '\u{1f}';
/// Longest word kept, in bytes (Tantivy's `RemoveLongFilter` default).
const MAX_WORD: usize = 40;

/// The distinct words of `text`.
pub fn words(text: &[u8]) -> BTreeSet<String> {
    String::from_utf8_lossy(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .filter(|w| w.len() < MAX_WORD)
        .collect()
}

fn posting(word: &str, row: &str) -> Node {
    Node {
        label: format!("{word}{US}{row}"),
        target: Target::None,
        edges: Vec::new(),
    }
}

impl<S: Store> Repo<S> {
    /// Index every row of `kv` whose key starts with `prefix`.
    pub fn text_index_build(&mut self, db: &Db, prefix: &str) -> Result<Id, Error> {
        let mut nodes = Vec::new();
        for n in self.graph_prefix(&db.kv, prefix)? {
            if n.label.contains(US) {
                return Err(Error::InvalidName(n.label));
            }
            let text = self.db_get(db, &n.label)?.unwrap_or_default();
            nodes.extend(words(&text).iter().map(|w| posting(w, &n.label)));
        }
        self.put_graph_with(nodes, Keys::Any)
    }

    /// Bring `index`, built at `old`, up to `new`: only the postings of
    /// rows that changed are edited.
    pub fn text_index_update(
        &mut self,
        index: &Id,
        old: &Db,
        new: &Db,
        prefix: &str,
    ) -> Result<Id, Error> {
        let mut edits: BTreeMap<String, Option<Node>> = BTreeMap::new();
        for (label, _, _) in self.graph_diff(&old.kv, &new.kv)? {
            if !label.starts_with(prefix) {
                continue;
            }
            if label.contains(US) {
                return Err(Error::InvalidName(label));
            }
            let before = words(&self.db_get(old, &label)?.unwrap_or_default());
            let after = self
                .db_get(new, &label)?
                .map(|t| words(&t))
                .unwrap_or_default();
            for w in before.difference(&after) {
                edits.insert(format!("{w}{US}{label}"), None);
            }
            for w in after.difference(&before) {
                let p = posting(w, &label);
                edits.insert(p.label.clone(), Some(p));
            }
        }
        self.graph_edit_with(index, edits, Keys::Any)
    }

    /// Rows holding every word of `query`, in key order.
    pub fn text_search(&self, index: &Id, query: &str) -> Result<Vec<String>, Error> {
        let mut hits: Option<BTreeSet<String>> = None;
        for w in words(query.as_bytes()) {
            let rows: BTreeSet<String> = self
                .graph_prefix(index, &format!("{w}{US}"))?
                .into_iter()
                .filter_map(|n| n.label.split_once(US).map(|(_, r)| r.to_string()))
                .collect();
            hits = Some(match hits {
                Some(h) => h.intersection(&rows).cloned().collect(),
                None => rows,
            });
            if hits.as_ref().is_some_and(BTreeSet::is_empty) {
                break;
            }
        }
        Ok(hits.unwrap_or_default().into_iter().collect())
    }
}
