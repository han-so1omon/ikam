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

/// Options for a segmented text index (E247).
#[derive(Clone, Copy, Debug)]
pub struct SegmentOpts {
    /// Merge the delta and dead postings into the base when together they
    /// pass this share of the base's postings.
    pub merge_above: f64,
    /// Store chunks plain, without the slice search (as E245's
    /// `plain_writes`): the index can be rebuilt from the rows.
    pub plain_writes: bool,
    /// Keep the base as block lists of row ids (E251, after Tantivy's
    /// postings) instead of a graph of postings; a merge rebuilds it.
    pub block_base: bool,
}

/// A text index kept as segments (E247), after Lucene's segments and
/// deletes: `base` holds postings as of its last merge, `delta` the
/// postings of rows changed since, `dead` the base postings those changes
/// made stale, and `moved` the changed rows. An update edits only `delta`,
/// `dead` and `moved`; a merge folds them into `base` in one edit. A
/// word's rows are `base - dead + delta`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segments {
    pub base: Id,
    pub delta: Id,
    pub dead: Id,
    pub moved: Id,
    pub base_postings: usize,
    pub delta_postings: usize,
    pub dead_postings: usize,
    /// E251: the base as block lists (then `base` is unused).
    pub block: Option<crate::postblocks::BlockIndex>,
}

impl<S: Store> Repo<S> {
    /// Run `f` with `opts.plain_writes` in force.
    fn plain_scope<T>(&mut self, opts: &SegmentOpts, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = self.read_weight;
        if opts.plain_writes {
            self.read_weight = f64::INFINITY;
        }
        let out = f(self);
        self.read_weight = saved;
        out
    }

    /// A row's value from its `kv` node, without looking the key up again.
    pub(crate) fn kv_value(&self, n: &Node) -> Result<Vec<u8>, Error> {
        match &n.target {
            Target::Content(crate::Arg::Range { id, start, len }) => {
                let bytes = self.read_content(id)?;
                let v = crate::repo::select(&bytes, *start, *len).ok_or(Error::Corrupt(*id))?;
                Ok(v.to_vec())
            }
            Target::Content(crate::Arg::Whole(id)) => self.read_content(id),
            _ => Err(Error::Decode("db kv")),
        }
    }

    /// Segments with every posting in the base (E247).
    pub fn text_segments_build(
        &mut self,
        db: &Db,
        prefix: &str,
        opts: &SegmentOpts,
    ) -> Result<Segments, Error> {
        self.plain_scope(opts, |r| {
            let empty = r.put_graph_with(Vec::new(), Keys::Any)?;
            let (base, block, base_postings) = match opts.block_base {
                true => {
                    let b = r.block_index_build(db, prefix)?;
                    (empty, Some(b), b.postings)
                }
                false => {
                    let base = r.text_index_build(db, prefix)?;
                    (base, None, r.graph_nodes(&base)?.len())
                }
            };
            Ok(Segments {
                base,
                delta: empty,
                dead: empty,
                moved: empty,
                base_postings,
                delta_postings: 0,
                dead_postings: 0,
                block,
            })
        })
    }

    /// Bring `seg`, kept at `old`, up to `new`; merges when the delta and
    /// dead postings pass `opts.merge_above` of the base.
    pub fn text_segments_update(
        &mut self,
        seg: &Segments,
        old: &Db,
        new: &Db,
        prefix: &str,
        opts: &SegmentOpts,
    ) -> Result<Segments, Error> {
        let changes: Vec<_> = self
            .graph_diff(&old.kv, &new.kv)?
            .into_iter()
            .filter(|c| c.0.starts_with(prefix))
            .collect();
        if changes.iter().any(|c| c.0.contains(US)) {
            return Err(Error::InvalidName(prefix.to_string()));
        }
        let labels: BTreeSet<String> = changes.iter().map(|c| c.0.clone()).collect();
        let moved: BTreeSet<String> = self
            .graph_nodes_in(&seg.moved, &labels)?
            .into_iter()
            .map(|n| n.label)
            .collect();
        let (mut delta, mut dead, mut rows) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        let mut next = *seg;
        for (row, before, after) in &changes {
            let was = match before {
                Some(n) => words(&self.kv_value(n)?),
                None => BTreeSet::new(),
            };
            let now = match after {
                Some(n) => words(&self.kv_value(n)?),
                None => BTreeSet::new(),
            };
            if moved.contains(row) {
                for w in was.difference(&now) {
                    delta.insert(format!("{w}{US}{row}"), None);
                }
                next.delta_postings -= was.len();
            } else {
                for w in &was {
                    let p = posting(w, row);
                    dead.insert(p.label.clone(), Some(p));
                }
                next.dead_postings += was.len();
                rows.insert(
                    row.clone(),
                    Some(Node {
                        label: row.clone(),
                        target: Target::None,
                        edges: Vec::new(),
                    }),
                );
            }
            for w in &now {
                let p = posting(w, row);
                delta.insert(p.label.clone(), Some(p));
            }
            next.delta_postings += now.len();
        }
        self.plain_scope(opts, |r| {
            next.delta = r.graph_edit_with(&seg.delta, delta, Keys::Any)?;
            next.dead = r.graph_edit_with(&seg.dead, dead, Keys::Any)?;
            next.moved = r.graph_edit_with(&seg.moved, rows, Keys::Any)?;
            if (next.delta_postings + next.dead_postings) as f64
                > opts.merge_above * next.base_postings.max(1) as f64
            {
                return r.text_segments_merge_at(&next, new, prefix, opts);
            }
            Ok(next)
        })
    }

    /// Merge `seg`, kept at `db`: a graph base takes its delta and dead
    /// postings in one edit; a block base (E251) is rebuilt at `db`, as
    /// Tantivy writes a merged segment anew.
    pub fn text_segments_merge_at(
        &mut self,
        seg: &Segments,
        db: &Db,
        prefix: &str,
        opts: &SegmentOpts,
    ) -> Result<Segments, Error> {
        match seg.block {
            Some(_) => self.text_segments_build(db, prefix, opts),
            None => self.text_segments_merge(seg, opts),
        }
    }

    /// Fold `seg`'s delta and dead postings into its base (a graph base).
    pub fn text_segments_merge(
        &mut self,
        seg: &Segments,
        opts: &SegmentOpts,
    ) -> Result<Segments, Error> {
        if seg.block.is_some() {
            return Err(Error::Decode(
                "a block base merges with text_segments_merge_at",
            ));
        }
        let mut edits: BTreeMap<String, Option<Node>> = BTreeMap::new();
        for n in self.graph_nodes(&seg.dead)? {
            edits.insert(n.label, None);
        }
        for n in self.graph_nodes(&seg.delta)? {
            edits.insert(n.label.clone(), Some(n));
        }
        self.plain_scope(opts, |r| {
            let base = r.graph_edit_with(&seg.base, edits, Keys::Any)?;
            let empty = r.put_graph_with(Vec::new(), Keys::Any)?;
            let base_postings = seg.base_postings + seg.delta_postings - seg.dead_postings;
            Ok(Segments {
                base,
                delta: empty,
                dead: empty,
                moved: empty,
                base_postings,
                delta_postings: 0,
                dead_postings: 0,
                block: None,
            })
        })
    }

    /// `text_search` over segments: each word's rows are `base - dead + delta`.
    pub fn text_segments_search(&self, seg: &Segments, query: &str) -> Result<Vec<String>, Error> {
        let rows = |r: &Self, root: &Id, w: &str| -> Result<BTreeSet<String>, Error> {
            Ok(r.graph_prefix(root, &format!("{w}{US}"))?
                .into_iter()
                .filter_map(|n| n.label.split_once(US).map(|(_, r)| r.to_string()))
                .collect())
        };
        if let Some(b) = &seg.block {
            return self.block_segments_search(seg, b, query, &rows);
        }
        let mut hits: Option<BTreeSet<String>> = None;
        for w in words(query.as_bytes()) {
            let mut here = rows(self, &seg.base, &w)?;
            for d in rows(self, &seg.dead, &w)? {
                here.remove(&d);
            }
            here.extend(rows(self, &seg.delta, &w)?);
            hits = Some(match hits {
                Some(h) => h.intersection(&here).cloned().collect(),
                None => here,
            });
            if hits.as_ref().is_some_and(BTreeSet::is_empty) {
                break;
            }
        }
        Ok(hits.unwrap_or_default().into_iter().collect())
    }

    /// E251's search: every moved row's base postings are dead and only
    /// moved rows have delta postings, so the rows are the base's
    /// intersection less the moved rows, plus the delta's intersection.
    fn block_segments_search(
        &self,
        seg: &Segments,
        b: &crate::postblocks::BlockIndex,
        query: &str,
        rows: &dyn Fn(&Self, &Id, &str) -> Result<BTreeSet<String>, Error>,
    ) -> Result<Vec<String>, Error> {
        let ws: Vec<String> = words(query.as_bytes()).into_iter().collect();
        if ws.is_empty() {
            return Ok(Vec::new());
        }
        let ids = self.block_index_ids(b, &ws)?;
        let base: BTreeSet<String> = self.block_index_keys(b, &ids)?.into_iter().collect();
        let moved: BTreeSet<String> = self
            .graph_nodes_in(&seg.moved, &base)?
            .into_iter()
            .map(|n| n.label)
            .collect();
        let mut hits: BTreeSet<String> = base.difference(&moved).cloned().collect();
        let mut delta: Option<BTreeSet<String>> = None;
        for w in &ws {
            let here = rows(self, &seg.delta, w)?;
            delta = Some(match delta {
                Some(d) => d.intersection(&here).cloned().collect(),
                None => here,
            });
        }
        hits.extend(delta.unwrap_or_default());
        Ok(hits.into_iter().collect())
    }
}
