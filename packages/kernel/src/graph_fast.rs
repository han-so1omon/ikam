//! Graph reads that pay for what changed (E449; options, off by default).
//!
//! A graph version is a prolly tree whose chunks are shared with the
//! versions before it (`graph.rs`). The plain reads decode every chunk of
//! a version and read every node's content one at a time, each through a
//! full reconstruction (`read_content`). Here:
//!
//! - [`Repo::graph_cache`] keeps decoded chunks, by chunk id, across reads
//!   and versions. A chunk id names its bytes, so a kept chunk is always
//!   right; reading version N+1 after N decodes only the chunks the edit
//!   wrote. Anything decoded from a chunk (a leaf's payloads, a net's
//!   elements) may be kept beside it under its own kind
//!   ([`Repo::graph_decoded`]), so a caller's own decoding is paid once a
//!   chunk too.
//! - [`Repo::read_contents`] reads many contents in one call: a stored
//!   content is read, decoded and checked against its id directly, and
//!   only the rest go through the full reconstruction.
//! - Lookups by label ([`Repo::graph_node_cached`],
//!   [`Repo::graph_nodes_in_cached`], [`Repo::graph_hops`]) binary-search
//!   kept chunks, so a partial read or a k-hop walk reads only the paths it
//!   needs, and those once.
//! - With `S: Sync`, the `_par` reads decode the chunks not kept on up to
//!   [`Repo::read_threads`] threads.
//!
//! Nothing stored changes: these read the same chunks the plain reads do.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

use crate::{Arg, Edge, Error, Graph, Id, Node, Repo, Store, Target};

type Entry = Arc<dyn Any + Send + Sync>;

/// Decoded chunks (and what callers decode from them), dropped oldest
/// first once more than `graph_cache` are kept.
#[derive(Default)]
pub(crate) struct ChunkCache {
    map: HashMap<(Id, &'static str), Entry>,
    order: VecDeque<(Id, &'static str)>,
    /// Lookups served from the cache, and decodes, since the last reset.
    pub(crate) hits: usize,
    pub(crate) misses: usize,
    /// Values kept by name, not by chunk, and not counted against
    /// `graph_cache` (the last fragment version written of each net, E449).
    named: HashMap<String, Entry>,
}

/// The kind under which a decoded chunk itself is kept.
const CHUNK: &str = "graph";
/// The kind under which a leaf's nodes with their payloads are kept.
const PAYLOADS: &str = "payloads";

/// A leaf chunk's nodes, each with its content's bytes (None for a node
/// whose target is not a whole content).
pub struct Leaf {
    pub id: Id,
    pub nodes: Vec<(Node, Option<Vec<u8>>)>,
}

impl<S: Store> Repo<S> {
    /// What the cache keeps of chunk `id` under `kind`, or `make()` kept
    /// there. With `graph_cache` 0 nothing is kept.
    pub fn graph_decoded<T: Any + Send + Sync>(
        &self,
        id: &Id,
        kind: &'static str,
        make: impl FnOnce() -> Result<T, Error>,
    ) -> Result<Arc<T>, Error> {
        if self.graph_cache == 0 {
            return Ok(Arc::new(make()?));
        }
        let key = (*id, kind);
        {
            let mut c = self.chunk_cache.lock().unwrap();
            if let Some(e) = c.map.get(&key).cloned() {
                c.hits += 1;
                if let Ok(t) = e.downcast::<T>() {
                    return Ok(t);
                }
            }
        }
        let t = Arc::new(make()?);
        self.graph_keep(key, t.clone());
        Ok(t)
    }

    fn graph_keep(&self, key: (Id, &'static str), e: Entry) {
        let mut c = self.chunk_cache.lock().unwrap();
        c.misses += 1;
        if c.map.insert(key, e).is_none() {
            c.order.push_back(key);
        }
        while c.order.len() > self.graph_cache {
            let Some(old) = c.order.pop_front() else {
                break;
            };
            c.map.remove(&old);
        }
    }

    /// What is kept under `name` (`graph_name`), if it is a `T`.
    pub(crate) fn graph_named<T: Any + Send + Sync>(&self, name: &str) -> Option<Arc<T>> {
        let c = self.chunk_cache.lock().unwrap();
        c.named.get(name).cloned()?.downcast::<T>().ok()
    }

    /// Keep `value` under `name` until gc or `graph_cache_clear`.
    pub(crate) fn graph_name<T: Any + Send + Sync>(&self, name: String, value: Arc<T>) {
        self.chunk_cache.lock().unwrap().named.insert(name, value);
    }

    /// Lookups the cache served and chunks it decoded, since the last
    /// [`graph_cache_clear`](Repo::graph_cache_clear).
    pub fn graph_cache_stats(&self) -> (usize, usize) {
        let c = self.chunk_cache.lock().unwrap();
        (c.hits, c.misses)
    }

    /// Drop everything `graph_cache` keeps.
    pub fn graph_cache_clear(&self) {
        *self.chunk_cache.lock().unwrap() = ChunkCache::default();
    }

    /// A graph chunk, decoded once while `graph_cache` keeps it.
    pub fn graph_chunk_cached(&self, id: &Id) -> Result<Arc<Graph>, Error> {
        self.graph_decoded(id, CHUNK, || self.graph_chunk(id))
    }

    /// The leaf chunks under `root`, in label order, the chunks above them
    /// read through the cache.
    pub fn graph_leaf_ids(&self, root: &Id) -> Result<Vec<Id>, Error> {
        // The children of a chunk at height 1 are leaves: listed, not read
        // (a root that is a leaf is read once, to tell).
        if let Graph::Nodes(_) = &*self.graph_chunk_cached(root)? {
            return Ok(vec![*root]);
        }
        let mut out = Vec::new();
        let mut stack = vec![*root];
        while let Some(id) = stack.pop() {
            match &*self.graph_chunk_cached(&id)? {
                Graph::Children(1, children) => out.extend(children.iter().map(|c| c.1)),
                Graph::Children(_, children) => stack.extend(children.iter().rev().map(|c| c.1)),
                Graph::Nodes(_) => return Err(Error::Decode("graph leaf above height 0")),
            }
        }
        Ok(out)
    }

    /// Every node of a graph, as `graph_nodes`, through the cache.
    pub fn graph_nodes_cached(&self, root: &Id) -> Result<Vec<Node>, Error> {
        let mut out = Vec::new();
        for leaf in self.graph_leaf_ids(root)? {
            if let Graph::Nodes(nodes) = &*self.graph_chunk_cached(&leaf)? {
                out.extend(nodes.iter().cloned());
            }
        }
        Ok(out)
    }

    /// The contents `ids` names, in order: those stored as bytes read,
    /// decoded and checked against their id directly; any other through
    /// `read_content` (a derivation, a kept copy, a missing content).
    pub fn read_contents(&self, ids: &[Id]) -> Result<Vec<Vec<u8>>, Error> {
        ids.iter().map(|id| self.read_content_direct(id)).collect()
    }

    /// One content as `read_contents` reads it.
    pub(crate) fn read_content_direct(&self, id: &Id) -> Result<Vec<u8>, Error> {
        if self.read_cache == 0
            && let Ok(encoded) = self.store.read(id)
            && let Ok(bytes) = self.decode_plain(id, &encoded)
            && (crate::dict::self_verified(&encoded) || Id::of_content(&bytes) == *id)
        {
            return Ok(bytes);
        }
        self.read_content(id)
    }

    fn leaf_payloads(&self, leaf: &Id) -> Result<Leaf, Error> {
        let Graph::Nodes(nodes) = &*self.graph_chunk_cached(leaf)? else {
            return Err(Error::WrongKind(*leaf));
        };
        let mut out = Vec::with_capacity(nodes.len());
        for n in nodes {
            let bytes = match &n.target {
                Target::Content(Arg::Whole(id)) => Some(self.read_content_direct(id)?),
                _ => None,
            };
            out.push((n.clone(), bytes));
        }
        Ok(Leaf {
            id: *leaf,
            nodes: out,
        })
    }

    /// Every leaf of a graph with its nodes' contents (a whole read), each
    /// leaf fetched and decoded once while `graph_cache` keeps it.
    pub fn graph_read(&self, root: &Id) -> Result<Vec<Arc<Leaf>>, Error> {
        (self.graph_leaf_ids(root)?.iter())
            .map(|l| self.graph_decoded(l, PAYLOADS, || self.leaf_payloads(l)))
            .collect()
    }

    /// The node labelled `label`, as `graph_node`, one cached chunk a
    /// level, found by binary search.
    pub fn graph_node_cached(&self, root: &Id, label: &str) -> Result<Option<Node>, Error> {
        let mut id = *root;
        loop {
            match &*self.graph_chunk_cached(&id)? {
                Graph::Nodes(nodes) => {
                    return Ok(nodes
                        .binary_search_by(|n| n.label.as_str().cmp(label))
                        .ok()
                        .map(|i| nodes[i].clone()));
                }
                Graph::Children(_, children) => {
                    let i = children.partition_point(|(first, _)| first.as_str() <= label);
                    match i {
                        0 => return Ok(None),
                        i => id = children[i - 1].1,
                    }
                }
            }
        }
    }

    /// The nodes labelled with any of `labels`, as `graph_nodes_in`,
    /// through the cache: each chunk that can hold one read once.
    pub fn graph_nodes_in_cached(
        &self,
        root: &Id,
        labels: &BTreeSet<String>,
    ) -> Result<Vec<Node>, Error> {
        let mut out = Vec::new();
        if !labels.is_empty() {
            self.collect_in_cached(root, labels, &mut out)?;
        }
        Ok(out)
    }

    fn collect_in_cached(
        &self,
        id: &Id,
        labels: &BTreeSet<String>,
        out: &mut Vec<Node>,
    ) -> Result<(), Error> {
        match &*self.graph_chunk_cached(id)? {
            Graph::Nodes(nodes) => {
                for l in labels {
                    if let Ok(i) = nodes.binary_search_by(|n| n.label.as_str().cmp(l)) {
                        out.push(nodes[i].clone());
                    }
                }
            }
            Graph::Children(_, children) => {
                for (i, (first, child)) in children.iter().enumerate() {
                    let low = if i == 0 { String::new() } else { first.clone() };
                    let mine: BTreeSet<String> = match children.get(i + 1) {
                        Some((next, _)) => labels.range(low..next.clone()).cloned().collect(),
                        None => labels.range(low..).cloned().collect(),
                    };
                    if !mine.is_empty() {
                        self.collect_in_cached(child, &mine, out)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The nodes within `k` hops of `start`, by label: a breadth-first walk
    /// where `follow` names the label an edge leads to (None: not
    /// followed). Each hop reads the chunks its frontier falls in, once.
    pub fn graph_hops(
        &self,
        root: &Id,
        start: &[String],
        k: usize,
        follow: impl Fn(&Edge) -> Option<String>,
    ) -> Result<BTreeMap<String, Node>, Error> {
        let mut seen = BTreeMap::new();
        let mut frontier: BTreeSet<String> = start.iter().cloned().collect();
        for hop in 0..=k {
            let mut next = BTreeSet::new();
            for n in self.graph_nodes_in_cached(root, &frontier)? {
                if hop < k {
                    next.extend(n.edges.iter().filter_map(&follow));
                }
                seen.insert(n.label.clone(), n);
            }
            frontier = next.into_iter().filter(|l| !seen.contains_key(l)).collect();
            if frontier.is_empty() {
                break;
            }
        }
        Ok(seen)
    }
}

impl<S: Store + Sync> Repo<S> {
    /// Decode each of `leaves` under `kind` with `make`, the ones the
    /// cache does not keep on up to `read_threads` threads (1 or 0: this
    /// one), in order.
    pub fn graph_decoded_par<T: Any + Send + Sync>(
        &self,
        leaves: &[Id],
        kind: &'static str,
        make: impl Fn(&Id) -> Result<T, Error> + Sync,
    ) -> Result<Vec<Arc<T>>, Error> {
        let mut out: Vec<Option<Arc<T>>> = vec![None; leaves.len()];
        let mut todo = Vec::new();
        if self.graph_cache > 0 {
            let mut c = self.chunk_cache.lock().unwrap();
            for (i, l) in leaves.iter().enumerate() {
                match c.map.get(&(*l, kind)).cloned().map(|e| e.downcast::<T>()) {
                    Some(Ok(t)) => {
                        c.hits += 1;
                        out[i] = Some(t);
                    }
                    _ => todo.push(i),
                }
            }
        } else {
            todo = (0..leaves.len()).collect();
        }
        let threads = self.read_threads.clamp(1, todo.len().max(1));
        let made: Vec<(usize, Result<T, Error>)> = if threads == 1 {
            todo.iter().map(|&i| (i, make(&leaves[i]))).collect()
        } else {
            let per = todo.len().div_ceil(threads);
            std::thread::scope(|s| {
                let make = &make;
                let parts: Vec<_> = todo
                    .chunks(per)
                    .map(|part| {
                        s.spawn(move || {
                            part.iter()
                                .map(|&i| (i, make(&leaves[i])))
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                parts
                    .into_iter()
                    .flat_map(|p| p.join().expect("a read thread panicked"))
                    .collect()
            })
        };
        for (i, t) in made {
            let t = Arc::new(t?);
            if self.graph_cache > 0 {
                self.graph_keep((leaves[i], kind), t.clone());
            }
            out[i] = Some(t);
        }
        Ok(out
            .into_iter()
            .map(|t| t.expect("every leaf decoded"))
            .collect())
    }

    /// [`graph_read`](Repo::graph_read), the leaves not kept read on up to
    /// `read_threads` threads.
    pub fn graph_read_par(&self, root: &Id) -> Result<Vec<Arc<Leaf>>, Error> {
        let leaves = self.graph_leaf_ids(root)?;
        self.graph_decoded_par(&leaves, PAYLOADS, |l| self.leaf_payloads(l))
    }
}
