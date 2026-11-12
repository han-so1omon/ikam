//! Embeddings kept as a view of the database (E255), keyed by content
//! hash: a row's value is split into fragments (its lines), and each
//! fragment's embedding is stored once, under its model and its content id,
//! in a cache graph (`model US content-id`). The row view maps each row's
//! fragments (`row US index`) to their vectors. Identical fragments, in one
//! row or many, share one embedding; an update embeds only fragments the
//! cache does not hold; a new model id finds nothing cached and embeds each
//! distinct fragment once.
//!
//! Opt-in: nothing in the kernel embeds unless a caller asks.

use std::collections::{BTreeMap, BTreeSet};

use crate::vectors::from_f32s;
use crate::{Arg, Db, Error, Id, Keys, Node, Repo, Store, Target};

const US: char = '\u{1f}';

/// The cache (every model's embeddings) and one model's row view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Embeds {
    pub cache: Id,
    pub rows: Id,
}

/// Fragments embedded and found in the cache during one call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EmbedStats {
    pub embedded: usize,
    pub cached: usize,
}

fn fragments(value: &[u8]) -> impl Iterator<Item = &[u8]> {
    value.split(|&b| b == b'\n').filter(|f| !f.is_empty())
}

fn node(label: String, id: Id) -> Node {
    Node {
        label,
        target: Target::Content(Arg::Whole(id)),
        edges: Vec::new(),
    }
}

impl<S: Store> Repo<S> {
    /// Empty cache and view.
    pub fn embeds_empty(&mut self) -> Result<Embeds, Error> {
        let empty = self.put_graph_with(Vec::new(), Keys::Any)?;
        Ok(Embeds {
            cache: empty,
            rows: empty,
        })
    }

    /// Bring `e`, kept at `old` for `model`, up to `new`: rows in the diff
    /// get their fragments' vectors, from the cache or from `embed`. With
    /// an empty row view and `old` empty, this is a build.
    pub fn embeds_update(
        &mut self,
        e: &Embeds,
        old: &Db,
        new: &Db,
        prefix: &str,
        model: &str,
        embed: &mut dyn FnMut(&[u8]) -> Vec<f32>,
    ) -> Result<(Embeds, EmbedStats), Error> {
        let changes: Vec<_> = self
            .graph_diff(&old.kv, &new.kv)?
            .into_iter()
            .filter(|c| c.0.starts_with(prefix))
            .collect();
        // Fragments of every changed row, and which the cache holds.
        let mut frags: BTreeMap<String, Vec<(Id, Vec<u8>)>> = BTreeMap::new();
        for (row, _, after) in &changes {
            let v = match after {
                Some(n) => self.kv_value(n)?,
                None => Vec::new(),
            };
            frags.insert(
                row.clone(),
                fragments(&v)
                    .map(|f| (Id::of_content(f), f.to_vec()))
                    .collect(),
            );
        }
        let wanted: BTreeSet<String> = frags
            .values()
            .flatten()
            .map(|(id, _)| format!("{model}{US}{id}"))
            .collect();
        let mut have: BTreeMap<String, Id> = BTreeMap::new();
        for n in self.graph_nodes_in(&e.cache, &wanted)? {
            if let Target::Content(a) = n.target {
                have.insert(n.label, a.id());
            }
        }
        let (mut stats, mut cache_edits, mut row_edits) =
            (EmbedStats::default(), BTreeMap::new(), BTreeMap::new());
        for (row, list) in &frags {
            for (i, (id, bytes)) in list.iter().enumerate() {
                let key = format!("{model}{US}{id}");
                let vid = match have.get(&key) {
                    Some(v) => {
                        stats.cached += 1;
                        *v
                    }
                    None => {
                        stats.embedded += 1;
                        let v = self.store_plain(&from_f32s(&embed(bytes)))?;
                        have.insert(key.clone(), v);
                        cache_edits.insert(key.clone(), Some(node(key, v)));
                        v
                    }
                };
                let label = format!("{row}{US}{i:04}");
                row_edits.insert(label.clone(), Some(node(label, vid)));
            }
        }
        // Fragments a changed row no longer has leave the row view.
        for (row, before, _) in &changes {
            if before.is_some() {
                for n in self.graph_prefix(&e.rows, &format!("{row}{US}"))? {
                    row_edits.entry(n.label).or_insert(None);
                }
            }
        }
        let next = Embeds {
            cache: self.graph_edit_with(&e.cache, cache_edits, Keys::Any)?,
            rows: self.graph_edit_with(&e.rows, row_edits, Keys::Any)?,
        };
        Ok((next, stats))
    }

    /// The vectors of `row`'s fragments, in order.
    pub fn embeds_of(&self, e: &Embeds, row: &str) -> Result<Vec<Vec<f32>>, Error> {
        self.graph_prefix(&e.rows, &format!("{row}{US}"))?
            .into_iter()
            .map(|n| match n.target {
                Target::Content(a) => Ok(crate::vectors::to_f32s(&self.read_content(&a.id())?)),
                _ => Err(Error::Decode("embeds row")),
            })
            .collect()
    }
}
