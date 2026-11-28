//! Computations over graphs stored as connected fragments, kept up to date
//! from the delta between versions (E450, opt-in; nothing in the default
//! paths calls this).
//!
//! A graph is a `put_graph` prolly tree whose nodes name their out-edges'
//! targets by key. [`Repo::graph_delta`] is E446's delta generalized from
//! nets to any such graph: the nodes added, removed and changed and the
//! edges added and removed, found by E249's walk, which skips the chunks two
//! versions share unread.
//!
//! Each [`Compute`] has a result that is itself a graph, one node per key
//! (labelled with the key in hex), so a result is versioned, shares every
//! chunk an edit does not reach with the version before, and is checked by
//! fsck like any graph. A value is held in the node's edges: an edge to the
//! zero key whose label is the value's text (a node has no value field).
//!
//! - [`Compute::Index`]: for every key, the label of its node (if present)
//!   and its in-edges. Edges name targets by key, so following an edge, or
//!   going against one, needs this index; the traversals below read it.
//! - [`Compute::Degree`]: out- and in-degree.
//! - [`Compute::Hops`]: distance from a set of roots along out-edges, up to
//!   `k` (or every node reachable, like a net's marking spreading).
//! - [`Compute::Components`]: weakly connected components, each node
//!   labelled with the least key in its component.
//! - [`Compute::PageRank`]: a fixed number of power iterations in integer
//!   fixed point (so sums do not depend on order), quantized when stored.
//!
//! [`Repo::graph_compute`] computes a version from scratch (every node read,
//! the result put whole). [`Repo::graph_compute_update`] gets the next
//! version from the last one's results and the delta: index and degree
//! from the delta alone; hops and components by a traversal that starts at
//! the nodes the delta names and reads only what it reaches; PageRank, which
//! changes globally, by a full recompute (exactness is the default;
//! [`MemGraph::pagerank_local`] measures a bounded, approximate update). In every
//! case the result's root equals the full recompute's.
//!
//! [`Repo::put_graph_step`] records a computation as a step graph: nodes
//! `func` (the computation's text), `input` (the graph's root), `output`
//! (the result's root), `index` and `from` (the step this one was updated
//! from). Held in a commit's tree, gc keeps all of them through it, and
//! [`Repo::graph_step_check`] rebuilds the output from the input and
//! compares.
//!
//! [`MemGraph`] runs the same algorithms over a plain in-memory adjacency,
//! the floor the fragment form is measured against.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use crate::{Arg, Edge, Error, Graph, Id, Node, Repo, Store, Target, node_key};

use super::NodeChange;

/// A key as a number (`node_key` read big-endian).
pub type Key = u64;

/// The key of `label`.
pub fn key_of(label: &str) -> Key {
    u64::from_be_bytes(node_key(label))
}

/// A result node's label: the key in hex.
pub fn hex(k: Key) -> String {
    format!("{k:016x}")
}

fn unhex(s: &str) -> Option<Key> {
    if s.len() != 16 {
        return None;
    }
    u64::from_str_radix(s, 16).ok()
}

fn bad() -> Error {
    Error::Decode("graph compute")
}

const VALUE: [u8; 8] = [0; 8];

/// An edge and the label of the node it leaves.
pub type EdgeAt = (String, Edge);

/// The difference between two versions of a graph (E446's delta, for any
/// `put_graph` graph).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GraphDelta {
    /// Nodes that differ: `(label, before, after)`.
    pub nodes: Vec<NodeChange>,
    /// Graph chunks read; the subtrees both versions share are skipped.
    pub chunks_read: usize,
}

impl GraphDelta {
    pub fn added(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter_map(|(_, a, b)| match (a, b) {
            (None, Some(n)) => Some(n),
            _ => None,
        })
    }

    pub fn removed(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter().filter_map(|(_, a, b)| match (a, b) {
            (Some(n), None) => Some(n),
            _ => None,
        })
    }

    /// Nodes on both sides whose target or edges changed: `(before, after)`.
    pub fn changed(&self) -> impl Iterator<Item = (&Node, &Node)> {
        self.nodes.iter().filter_map(|(_, a, b)| match (a, b) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        })
    }

    /// Edges added and removed, as `(from label, edge)`; a node added or
    /// removed brings or takes all its out-edges.
    pub fn edges(&self) -> (Vec<EdgeAt>, Vec<EdgeAt>) {
        let (mut added, mut removed) = (Vec::new(), Vec::new());
        for (label, a, b) in &self.nodes {
            let ea: BTreeSet<&Edge> = a.iter().flat_map(|n| &n.edges).collect();
            let eb: BTreeSet<&Edge> = b.iter().flat_map(|n| &n.edges).collect();
            added.extend(eb.difference(&ea).map(|e| (label.clone(), (*e).clone())));
            removed.extend(ea.difference(&eb).map(|e| (label.clone(), (*e).clone())));
        }
        (added, removed)
    }

    /// The later version's nodes from the earlier one's, in label order.
    pub fn apply(&self, nodes: Vec<Node>) -> Vec<Node> {
        let mut map: BTreeMap<String, Node> =
            nodes.into_iter().map(|n| (n.label.clone(), n)).collect();
        for (label, _, after) in &self.nodes {
            match after {
                Some(n) => map.insert(label.clone(), n.clone()),
                None => map.remove(label),
            };
        }
        map.into_values().collect()
    }

    fn keys(&self) -> KeyDelta {
        let mut d = KeyDelta::default();
        let outs = |n: &Option<Node>| -> Vec<Key> {
            n.iter()
                .flat_map(|n| &n.edges)
                .map(|e| u64::from_be_bytes(e.to))
                .collect()
        };
        for (label, a, b) in &self.nodes {
            let k = key_of(label);
            let (oa, ob) = (outs(a), outs(b));
            match (a, b) {
                (None, Some(_)) => d.added.push(k),
                (Some(_), None) => d.removed.push(k),
                _ => {}
            }
            if let Some(n) = b {
                d.label.insert(k, n.label.clone());
            }
            d.old_out.insert(k, (a.is_some(), oa));
            d.new_out.insert(k, (b.is_some(), ob));
        }
        let (added, removed) = self.edges();
        let pair = |(l, e): &(String, Edge)| (key_of(l), u64::from_be_bytes(e.to), e.label.clone());
        d.edges_added = added.iter().map(pair).collect();
        d.edges_removed = removed.iter().map(pair).collect();
        d
    }
}

/// A delta by key.
#[derive(Default)]
struct KeyDelta {
    added: Vec<Key>,
    removed: Vec<Key>,
    /// Labels of the nodes present after.
    label: HashMap<Key, String>,
    /// Every node in the delta: present, and its out-edge targets, before
    /// and after.
    old_out: HashMap<Key, (bool, Vec<Key>)>,
    new_out: HashMap<Key, (bool, Vec<Key>)>,
    /// `(from, to, edge label)`.
    edges_added: Vec<(Key, Key, String)>,
    edges_removed: Vec<(Key, Key, String)>,
}

/// A computation over a graph whose result is a graph keyed by node key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Compute {
    /// For every key named by a node or an edge: its node's label and its
    /// in-edges `(source key, edge label)`.
    Index,
    /// Out- and in-degree of every key (edges counted with their labels).
    Degree,
    /// Distance from `roots` (labels) along out-edges to present nodes, up
    /// to `k`; `None` is every node reachable.
    Hops { roots: Vec<String>, k: Option<u32> },
    /// Weakly connected components of the present nodes: each node's value
    /// is the least key in its component.
    Components,
    /// `iters` power iterations, damping 0.85, uniform start, dangling mass
    /// spread evenly, in integer fixed point with total mass 2^40; stored
    /// with the low `quantum` bits dropped.
    PageRank { iters: u32, quantum: u32 },
}

impl Compute {
    /// The computation as text, as a step record names it.
    pub fn spec(&self) -> String {
        match self {
            Compute::Index => "index".into(),
            Compute::Degree => "degree".into(),
            Compute::Hops { roots, k } => {
                let k = k.map_or("all".to_string(), |k| k.to_string());
                format!("hops {k} {}", roots.join(" "))
            }
            Compute::Components => "components".into(),
            Compute::PageRank { iters, quantum } => format!("pagerank {iters} {quantum}"),
        }
    }

    pub fn parse(s: &str) -> Option<Compute> {
        let mut w = s.split(' ');
        match w.next()? {
            "index" => Some(Compute::Index),
            "degree" => Some(Compute::Degree),
            "components" => Some(Compute::Components),
            "hops" => {
                let k = match w.next()? {
                    "all" => None,
                    k => Some(k.parse().ok()?),
                };
                Some(Compute::Hops {
                    roots: w.map(String::from).collect(),
                    k,
                })
            }
            "pagerank" => Some(Compute::PageRank {
                iters: w.next()?.parse().ok()?,
                quantum: w.next()?.parse().ok()?,
            }),
            _ => None,
        }
    }

    /// A short name, for tables.
    pub fn name(&self) -> &'static str {
        match self {
            Compute::Index => "index",
            Compute::Degree => "degree",
            Compute::Hops { k: Some(_), .. } => "k-hop",
            Compute::Hops { k: None, .. } => "reach",
            Compute::Components => "components",
            Compute::PageRank { .. } => "pagerank",
        }
    }
}

/// A computed value, kept in a result node's edges.
pub trait Val: Clone + PartialEq + std::fmt::Debug {
    fn enc(&self) -> Vec<Edge>;
    fn dec(edges: &[Edge]) -> Option<Self>;
}

fn one(label: String) -> Vec<Edge> {
    vec![Edge { to: VALUE, label }]
}

fn first(edges: &[Edge]) -> Option<&str> {
    edges
        .iter()
        .find(|e| e.to == VALUE)
        .map(|e| e.label.as_str())
}

/// An index entry: the node's label, if present, and its in-edges.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Entry {
    pub label: Option<String>,
    pub inn: Vec<(Key, String)>,
}

impl Val for Entry {
    fn enc(&self) -> Vec<Edge> {
        let mut out: Vec<Edge> = self
            .label
            .iter()
            .map(|l| Edge {
                to: VALUE,
                label: format!("={l}"),
            })
            .collect();
        out.extend(self.inn.iter().map(|(k, l)| Edge {
            to: k.to_be_bytes(),
            label: l.clone(),
        }));
        out
    }
    fn dec(edges: &[Edge]) -> Option<Self> {
        let mut e = Entry::default();
        for edge in edges {
            match (edge.to == VALUE, edge.label.strip_prefix('=')) {
                (true, Some(l)) if e.label.is_none() => e.label = Some(l.to_string()),
                _ => e
                    .inn
                    .push((u64::from_be_bytes(edge.to), edge.label.clone())),
            }
        }
        Some(e)
    }
}

/// Out- and in-degree, and whether a node holds the key (a key may only be
/// named by edges).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Deg {
    pub present: bool,
    pub out: u32,
    pub inn: u32,
}

impl Val for Deg {
    fn enc(&self) -> Vec<Edge> {
        let out = match self.present {
            true => self.out.to_string(),
            false => "-".into(),
        };
        one(format!("{out} {}", self.inn))
    }
    fn dec(edges: &[Edge]) -> Option<Self> {
        let (out, inn) = first(edges)?.split_once(' ')?;
        Some(Deg {
            present: out != "-",
            out: if out == "-" { 0 } else { out.parse().ok()? },
            inn: inn.parse().ok()?,
        })
    }
}

impl Val for u64 {
    fn enc(&self) -> Vec<Edge> {
        one(self.to_string())
    }
    fn dec(edges: &[Edge]) -> Option<Self> {
        first(edges)?.parse().ok()
    }
}

impl Val for u32 {
    fn enc(&self) -> Vec<Edge> {
        one(self.to_string())
    }
    fn dec(edges: &[Edge]) -> Option<Self> {
        first(edges)?.parse().ok()
    }
}

fn value_node<V: Val>(k: Key, v: &V) -> Node {
    let mut edges = v.enc();
    edges.sort();
    edges.dedup();
    Node {
        label: hex(k),
        target: Target::None,
        edges,
    }
}

/// The graph's adjacency in the version being computed: a node's label (if
/// present), out-edge targets and in-edge sources, loaded on demand.
trait View {
    fn need(&mut self, keys: &[Key]) -> Result<(), Error>;
    fn present(&self, k: Key) -> bool;
    fn out(&self, k: Key) -> &[Key];
    fn inn(&self, k: Key) -> &[Key];
}

/// A result's values, loaded on demand, set as the computation goes.
trait Vals<V> {
    fn need(&mut self, keys: &[Key]) -> Result<(), Error>;
    fn get(&self, k: Key) -> Option<&V>;
    fn set(&mut self, k: Key, v: Option<V>);
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Adj {
    present: bool,
    out: Vec<Key>,
    inn: Vec<Key>,
}

/// A plain in-memory adjacency, the floor the fragment form is measured
/// against: every node by key, with its out- and in-edges.
#[derive(Clone, Debug, Default)]
pub struct MemGraph {
    adj: HashMap<Key, Adj>,
    /// `(from, to, label)` multiplicity is kept by listing every edge.
    labels: HashMap<Key, String>,
    inn_labels: HashMap<Key, Vec<(Key, String)>>,
}

impl MemGraph {
    pub fn from_nodes(nodes: &[Node]) -> MemGraph {
        let mut g = MemGraph::default();
        for n in nodes {
            let k = key_of(&n.label);
            g.labels.insert(k, n.label.clone());
            let a = g.adj.entry(k).or_default();
            a.present = true;
            a.out = n.edges.iter().map(|e| u64::from_be_bytes(e.to)).collect();
            for e in &n.edges {
                let t = u64::from_be_bytes(e.to);
                g.adj.entry(t).or_default().inn.push(k);
                g.inn_labels
                    .entry(t)
                    .or_default()
                    .push((k, e.label.clone()));
            }
        }
        g
    }

    /// The later version, from the earlier one and their delta.
    pub fn apply(&mut self, delta: &GraphDelta) {
        let kd = delta.keys();
        for (from, to, label) in &kd.edges_removed {
            if let Some(a) = self.adj.get_mut(to)
                && let Some(i) = a.inn.iter().position(|s| s == from)
            {
                a.inn.swap_remove(i);
            }
            if let Some(v) = self.inn_labels.get_mut(to)
                && let Some(i) = v.iter().position(|(s, l)| s == from && l == label)
            {
                v.swap_remove(i);
            }
        }
        for (from, to, label) in &kd.edges_added {
            self.adj.entry(*to).or_default().inn.push(*from);
            self.inn_labels
                .entry(*to)
                .or_default()
                .push((*from, label.clone()));
        }
        for (k, (present, out)) in kd.new_out {
            let a = self.adj.entry(k).or_default();
            a.present = present;
            a.out = out;
            match kd.label.get(&k) {
                Some(l) => self.labels.insert(k, l.clone()),
                None => self.labels.remove(&k),
            };
        }
        self.adj
            .retain(|_, a| a.present || !a.inn.is_empty() || !a.out.is_empty());
    }

    fn present_keys(&self) -> Vec<Key> {
        let mut ks: Vec<Key> = (self.adj.iter())
            .filter(|(_, a)| a.present)
            .map(|(k, _)| *k)
            .collect();
        ks.sort_unstable();
        ks
    }

    fn key_by_label(&self, label: &str) -> Option<Key> {
        let k = key_of(label);
        (self.labels.get(&k).map(String::as_str) == Some(label)).then_some(k)
    }

    /// `compute` from scratch, as `(key, value text)` sorted by key.
    pub fn compute(&self, c: &Compute) -> Vec<(Key, Vec<Edge>)> {
        fn enc<V: Val>(m: BTreeMap<Key, V>) -> Vec<(Key, Vec<Edge>)> {
            m.into_iter().map(|(k, v)| (k, v.enc())).collect()
        }
        match c {
            Compute::Index => enc(self.full_index()),
            Compute::Degree => enc(self.full_degree()),
            Compute::Hops { roots, k } => enc(self.full_hops(roots, *k)),
            Compute::Components => enc(self.full_components()),
            Compute::PageRank { iters, quantum } => enc(self.full_pagerank(*iters, *quantum)),
        }
    }

    fn full_index(&self) -> BTreeMap<Key, Entry> {
        let mut out = BTreeMap::new();
        for (k, a) in &self.adj {
            let mut inn = self.inn_labels.get(k).cloned().unwrap_or_default();
            inn.sort();
            if a.present || !inn.is_empty() {
                let label = self.labels.get(k).cloned().filter(|_| a.present);
                out.insert(*k, Entry { label, inn });
            }
        }
        out
    }

    fn full_degree(&self) -> BTreeMap<Key, Deg> {
        (self.adj.iter())
            .filter(|(_, a)| a.present || !a.inn.is_empty())
            .map(|(k, a)| {
                let d = Deg {
                    present: a.present,
                    out: if a.present { a.out.len() as u32 } else { 0 },
                    inn: a.inn.len() as u32,
                };
                (*k, d)
            })
            .collect()
    }

    fn full_hops(&self, roots: &[String], k: Option<u32>) -> BTreeMap<Key, u32> {
        let mut dist = BTreeMap::new();
        let mut frontier: Vec<Key> = roots.iter().filter_map(|r| self.key_by_label(r)).collect();
        frontier.sort_unstable();
        frontier.dedup();
        for f in &frontier {
            dist.insert(*f, 0);
        }
        let mut d = 0;
        while !frontier.is_empty() && k.is_none_or(|k| d < k) {
            d += 1;
            let mut next = Vec::new();
            for x in frontier {
                for y in &self.adj[&x].out {
                    if self.adj.get(y).is_some_and(|a| a.present) && !dist.contains_key(y) {
                        dist.insert(*y, d);
                        next.push(*y);
                    }
                }
            }
            frontier = next;
        }
        dist
    }

    fn full_components(&self) -> BTreeMap<Key, u64> {
        let keys = self.present_keys();
        let index: HashMap<Key, usize> = keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();
        let mut parent: Vec<usize> = (0..keys.len()).collect();
        for (i, k) in keys.iter().enumerate() {
            for t in &self.adj[k].out {
                if let Some(&j) = index.get(t) {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    // The smaller index (key) becomes the root.
                    match a.cmp(&b) {
                        std::cmp::Ordering::Less => parent[b] = a,
                        std::cmp::Ordering::Greater => parent[a] = b,
                        _ => {}
                    }
                }
            }
        }
        (0..keys.len())
            .map(|i| (keys[i], keys[find(&mut parent, i)]))
            .collect()
    }

    fn full_pagerank(&self, iters: u32, quantum: u32) -> BTreeMap<Key, u64> {
        let keys = self.present_keys();
        let x = pagerank(&keys, |k| &self.adj[&k].out, iters);
        keys.into_iter()
            .zip(x)
            .map(|(k, v)| (k, v >> quantum))
            .collect()
    }
}

/// Total rank mass in fixed point.
const MASS: u64 = 1 << 40;

/// Fixed-point PageRank over `keys` (sorted), `out` giving each key's
/// out-edge targets. Integer sums, so the result does not depend on the
/// order edges are visited in.
fn pagerank<'a>(keys: &[Key], out: impl Fn(Key) -> &'a [Key], iters: u32) -> Vec<u64> {
    let n = keys.len();
    if n == 0 {
        return Vec::new();
    }
    let index: HashMap<Key, usize> = keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();
    let targets: Vec<Vec<usize>> = keys
        .iter()
        .map(|k| {
            out(*k)
                .iter()
                .filter_map(|t| index.get(t).copied())
                .collect()
        })
        .collect();
    let mut x = vec![MASS / n as u64; n];
    for _ in 0..iters {
        let mut next = vec![0u64; n];
        let mut dangling: u128 = 0;
        for (i, ts) in targets.iter().enumerate() {
            if ts.is_empty() {
                dangling += x[i] as u128;
                continue;
            }
            let c = (x[i] as u128 * 85 / (100 * ts.len() as u128)) as u64;
            for &t in ts {
                next[t] += c;
            }
        }
        let base = ((MASS as u128 * 15 / 100 + dangling * 85 / 100) / n as u128) as u64;
        for v in next.iter_mut() {
            *v += base;
        }
        x = next;
    }
    x
}

/// Fragment view: the index (for labels and in-edges) and the graph (for
/// out-edges) of one version, read through a chunk cache.
struct FragView<'a, S: Store> {
    repo: &'a Repo<S>,
    graph: Id,
    index: Id,
    adj: HashMap<Key, Adj>,
    chunks: ChunkCache,
}

/// Decoded chunks read during one computation, so each is read once. A
/// chunk comes through `graph_chunk_cached`, so with `graph_cache` on it is
/// shared with other reads and versions (E451); with it off (the default)
/// each is decoded here as before.
#[derive(Default)]
struct ChunkCache {
    chunks: HashMap<Id, Arc<Graph>>,
    reads: usize,
}

impl<S: Store> Repo<S> {
    /// The nodes labelled with any of `labels`, reading each chunk that can
    /// hold one at most once per `cache`.
    fn nodes_in_cached(
        &self,
        root: &Id,
        labels: &BTreeSet<String>,
        cache: &mut ChunkCache,
        out: &mut Vec<Node>,
    ) -> Result<(), Error> {
        if labels.is_empty() {
            return Ok(());
        }
        if !cache.chunks.contains_key(root) {
            cache.reads += 1;
            let g = self.graph_chunk_cached(root)?;
            cache.chunks.insert(*root, g);
        }
        let chunk = Arc::clone(&cache.chunks[root]);
        match &*chunk {
            Graph::Nodes(nodes) => {
                out.extend(nodes.iter().filter(|n| labels.contains(&n.label)).cloned())
            }
            Graph::Children(_, children) => {
                for (i, (first, child)) in children.iter().enumerate() {
                    let low = if i == 0 { String::new() } else { first.clone() };
                    let mine: BTreeSet<String> = match children.get(i + 1) {
                        Some((next, _)) => labels.range(low..next.clone()).cloned().collect(),
                        None => labels.range(low..).cloned().collect(),
                    };
                    self.nodes_in_cached(child, &mine, cache, out)?;
                }
            }
        }
        Ok(())
    }

    fn values_in<V: Val>(
        &self,
        root: &Id,
        keys: &[Key],
        cache: &mut ChunkCache,
    ) -> Result<HashMap<Key, V>, Error> {
        let labels: BTreeSet<String> = keys.iter().map(|k| hex(*k)).collect();
        let mut nodes = Vec::new();
        self.nodes_in_cached(root, &labels, cache, &mut nodes)?;
        nodes
            .into_iter()
            .map(|n| {
                Ok((
                    unhex(&n.label).ok_or_else(bad)?,
                    V::dec(&n.edges).ok_or_else(bad)?,
                ))
            })
            .collect()
    }
}

impl<S: Store> View for FragView<'_, S> {
    fn need(&mut self, keys: &[Key]) -> Result<(), Error> {
        let missing: Vec<Key> = (keys.iter())
            .filter(|k| !self.adj.contains_key(k))
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let entries: HashMap<Key, Entry> =
            self.repo
                .values_in(&self.index, &missing, &mut self.chunks)?;
        let labels: BTreeSet<String> = entries.values().filter_map(|e| e.label.clone()).collect();
        let mut nodes = Vec::new();
        self.repo
            .nodes_in_cached(&self.graph, &labels, &mut self.chunks, &mut nodes)?;
        let outs: HashMap<String, Vec<Key>> = nodes
            .into_iter()
            .map(|n| {
                let out = n.edges.iter().map(|e| u64::from_be_bytes(e.to)).collect();
                (n.label, out)
            })
            .collect();
        for k in missing {
            let e = entries.get(&k);
            let label = e.and_then(|e| e.label.as_ref());
            let adj = Adj {
                present: label.is_some(),
                out: label.and_then(|l| outs.get(l)).cloned().unwrap_or_default(),
                inn: e
                    .map(|e| e.inn.iter().map(|x| x.0).collect())
                    .unwrap_or_default(),
            };
            self.adj.insert(k, adj);
        }
        Ok(())
    }
    fn present(&self, k: Key) -> bool {
        self.adj.get(&k).is_some_and(|a| a.present)
    }
    fn out(&self, k: Key) -> &[Key] {
        self.adj.get(&k).map_or(&[], |a| &a.out)
    }
    fn inn(&self, k: Key) -> &[Key] {
        self.adj.get(&k).map_or(&[], |a| &a.inn)
    }
}

impl View for MemGraph {
    fn need(&mut self, _: &[Key]) -> Result<(), Error> {
        Ok(())
    }
    fn present(&self, k: Key) -> bool {
        self.adj.get(&k).is_some_and(|a| a.present)
    }
    fn out(&self, k: Key) -> &[Key] {
        self.adj.get(&k).map_or(&[], |a| &a.out)
    }
    fn inn(&self, k: Key) -> &[Key] {
        self.adj.get(&k).map_or(&[], |a| &a.inn)
    }
}

/// A stored result's values: read on demand from the earlier result's
/// graph, changes kept as graph edits.
struct FragVals<'a, S: Store, V> {
    repo: &'a Repo<S>,
    root: Id,
    vals: HashMap<Key, Option<V>>,
    edits: BTreeMap<String, Option<Node>>,
    chunks: ChunkCache,
}

impl<S: Store, V: Val> Vals<V> for FragVals<'_, S, V> {
    fn need(&mut self, keys: &[Key]) -> Result<(), Error> {
        let missing: Vec<Key> = (keys.iter())
            .filter(|k| !self.vals.contains_key(k))
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let mut found: HashMap<Key, V> =
            self.repo
                .values_in(&self.root, &missing, &mut self.chunks)?;
        for k in missing {
            self.vals.insert(k, found.remove(&k));
        }
        Ok(())
    }
    fn get(&self, k: Key) -> Option<&V> {
        self.vals.get(&k).and_then(|v| v.as_ref())
    }
    fn set(&mut self, k: Key, v: Option<V>) {
        self.edits
            .insert(hex(k), v.as_ref().map(|v| value_node(k, v)));
        self.vals.insert(k, v);
    }
}

/// In-memory values, for the baseline.
impl<V: Val> Vals<V> for HashMap<Key, V> {
    fn need(&mut self, _: &[Key]) -> Result<(), Error> {
        Ok(())
    }
    fn get(&self, k: Key) -> Option<&V> {
        HashMap::get(self, &k)
    }
    fn set(&mut self, k: Key, v: Option<V>) {
        match v {
            Some(v) => self.insert(k, v),
            None => self.remove(&k),
        };
    }
}

// ---------------------------------------------------------------- updates

fn update_index(vals: &mut impl Vals<Entry>, kd: &KeyDelta) -> Result<(), Error> {
    let mut keys: Vec<Key> = kd.new_out.keys().copied().collect();
    keys.extend(kd.edges_added.iter().map(|e| e.1));
    keys.extend(kd.edges_removed.iter().map(|e| e.1));
    vals.need(&keys)?;
    let mut new: BTreeMap<Key, Entry> = BTreeMap::new();
    for k in keys {
        new.entry(k)
            .or_insert_with(|| vals.get(k).cloned().unwrap_or_default());
    }
    for (k, (present, _)) in &kd.new_out {
        new.get_mut(k).unwrap().label = present.then(|| kd.label[k].clone());
    }
    for (from, to, label) in &kd.edges_removed {
        let inn = &mut new.get_mut(to).unwrap().inn;
        if let Some(i) = inn.iter().position(|(s, l)| s == from && l == label) {
            inn.remove(i);
        }
    }
    for (from, to, label) in &kd.edges_added {
        new.get_mut(to).unwrap().inn.push((*from, label.clone()));
    }
    for (k, mut e) in new {
        e.inn.sort();
        let keep = e.label.is_some() || !e.inn.is_empty();
        let v = keep.then_some(e);
        if vals.get(k) != v.as_ref() {
            vals.set(k, v);
        }
    }
    Ok(())
}

fn update_degree(vals: &mut impl Vals<Deg>, kd: &KeyDelta) -> Result<(), Error> {
    let mut keys: Vec<Key> = kd.new_out.keys().copied().collect();
    keys.extend(kd.edges_added.iter().map(|e| e.1));
    keys.extend(kd.edges_removed.iter().map(|e| e.1));
    vals.need(&keys)?;
    let mut new: BTreeMap<Key, Deg> = BTreeMap::new();
    for k in keys {
        new.entry(k).or_insert_with(|| {
            vals.get(k).copied().unwrap_or(Deg {
                present: false,
                out: 0,
                inn: 0,
            })
        });
    }
    for (k, (present, out)) in &kd.new_out {
        let d = new.get_mut(k).unwrap();
        d.present = *present;
        d.out = if *present { out.len() as u32 } else { 0 };
    }
    for (_, to, _) in &kd.edges_removed {
        new.get_mut(to).unwrap().inn -= 1;
    }
    for (_, to, _) in &kd.edges_added {
        new.get_mut(to).unwrap().inn += 1;
    }
    for (k, d) in new {
        let v = (d.present || d.inn > 0).then_some(d);
        if vals.get(k) != v.as_ref() {
            vals.set(k, v);
        }
    }
    Ok(())
}

fn live_out(view: &impl View, x: Key) -> Vec<Key> {
    view.out(x)
        .iter()
        .copied()
        .filter(|y| view.present(*y))
        .collect()
}

/// Dynamic BFS: distances from `roots` in the later version, from the
/// earlier version's distances and the delta. Nodes that may have lost their
/// shortest path are checked level by level for another in-neighbour one
/// level up and invalidated if none; then the invalidated nodes and the
/// targets of new edges are settled from their in-neighbours, best first.
fn update_hops(
    view: &mut impl View,
    vals: &mut impl Vals<u32>,
    kd: &KeyDelta,
    roots: &BTreeSet<Key>,
    k: Option<u32>,
) -> Result<(), Error> {
    let within = |d: u32| k.is_none_or(|k| d <= k);
    // Phase A: invalidate.
    let mut touched: Vec<Key> = kd.old_out.keys().copied().collect();
    touched.extend(kd.edges_removed.iter().flat_map(|e| [e.0, e.1]));
    touched.extend(kd.old_out.values().flat_map(|(_, o)| o.iter().copied()));
    vals.need(&touched)?;
    let mut levels: BTreeMap<u32, BTreeSet<Key>> = BTreeMap::new();
    let mut invalid: BTreeSet<Key> = BTreeSet::new();
    // The level each invalidated node had.
    let mut old: HashMap<Key, u32> = HashMap::new();
    for r in &kd.removed {
        if let Some(&d) = vals.get(*r) {
            vals.set(*r, None);
            invalid.insert(*r);
            for y in &kd.old_out[r].1 {
                if vals.get(*y) == Some(&(d + 1)) {
                    levels.entry(d + 1).or_default().insert(*y);
                }
            }
        }
    }
    for (u, w, _) in &kd.edges_removed {
        if let (Some(&du), Some(&dw)) = (vals.get(*u), vals.get(*w))
            && dw == du + 1
        {
            levels.entry(dw).or_default().insert(*w);
        }
    }
    // A removed node was invalidated above; its old level-up targets are
    // candidates whatever its value now. A node with a value is present
    // (removed nodes lost theirs), and an in-edge's source is always a
    // present node, so support is read from the values alone.
    while let Some((j, cands)) = levels.pop_first() {
        let cands: Vec<Key> = cands.into_iter().collect();
        view.need(&cands)?;
        let mut lost = Vec::new();
        for x in cands {
            if invalid.contains(&x) || vals.get(x) != Some(&j) {
                continue;
            }
            let supported = view.present(x)
                && (roots.contains(&x) || (j > 0 && has_value(vals, view.inn(x), j - 1)?));
            if !supported {
                vals.set(x, None);
                old.insert(x, j);
                invalid.insert(x);
                lost.push(x);
            }
        }
        let outs: Vec<Key> = lost.iter().flat_map(|x| view.out(*x).to_vec()).collect();
        vals.need(&outs)?;
        for x in lost {
            for y in view.out(x).to_vec() {
                if vals.get(y) == Some(&(j + 1)) {
                    levels.entry(j + 1).or_default().insert(y);
                }
            }
        }
    }
    // Phase B: settle the invalidated nodes, the added nodes and roots
    // (from all their in-neighbours) and the targets of new edges (from the
    // edge's source alone: the rest of their in-neighbours did not change).
    let mut tent: BTreeMap<Key, u32> = BTreeMap::new();
    let offer = |tent: &mut BTreeMap<Key, u32>, x: Key, t: u32| {
        if within(t) {
            let e = tent.entry(x).or_insert(t);
            *e = (*e).min(t);
        }
    };
    let mut scan: BTreeSet<Key> = invalid;
    scan.extend(kd.added.iter().copied());
    scan.extend(roots.iter().copied().filter(|r| kd.new_out.contains_key(r)));
    let scan: Vec<Key> = scan.into_iter().collect();
    view.need(&scan)?;
    for x in &scan {
        if !view.present(*x) {
            continue;
        }
        if roots.contains(x) {
            offer(&mut tent, *x, 0);
            continue;
        }
        // An invalidated node lost every in-neighbour one level up, and
        // nothing has moved closer yet, so an in-neighbour at its old level
        // gives the best it can now have: the scan stops there.
        let floor = old.get(x).copied();
        if let Some(t) = least_value(vals, view.inn(*x), floor)? {
            offer(&mut tent, *x, t + 1);
        }
    }
    let sources: Vec<Key> = kd.edges_added.iter().map(|e| e.0).collect();
    vals.need(&sources)?;
    let reached: Vec<(Key, u32)> = (kd.edges_added.iter())
        .filter_map(|(u, w, _)| vals.get(*u).map(|d| (*w, d + 1)))
        .collect();
    view.need(&reached.iter().map(|r| r.0).collect::<Vec<_>>())?;
    for (w, t) in reached {
        if view.present(w) {
            offer(&mut tent, w, t);
        }
    }
    let mut queue: BTreeMap<u32, BTreeSet<Key>> = BTreeMap::new();
    let tk: Vec<Key> = tent.keys().copied().collect();
    vals.need(&tk)?;
    for (x, t) in tent {
        if vals.get(x).is_none_or(|d| *d > t) {
            queue.entry(t).or_default().insert(x);
        }
    }
    while let Some((j, xs)) = queue.pop_first() {
        let mut settled = Vec::new();
        for x in xs {
            if vals.get(x).is_some_and(|d| *d <= j) {
                continue;
            }
            vals.set(x, Some(j));
            settled.push(x);
        }
        if !within(j + 1) {
            continue;
        }
        view.need(&settled)?;
        let outs: Vec<Key> = settled.iter().flat_map(|x| view.out(*x).to_vec()).collect();
        vals.need(&outs)?;
        // Only a target that would get closer needs its presence read.
        let better: Vec<Key> = (outs.into_iter())
            .filter(|y| vals.get(*y).is_none_or(|d| *d > j + 1))
            .collect();
        view.need(&better)?;
        for y in better {
            if view.present(y) {
                queue.entry(j + 1).or_default().insert(y);
            }
        }
    }
    Ok(())
}

/// The least value among `keys`, loading them a few at a time and stopping
/// at `floor` (no key can have less).
fn least_value(
    vals: &mut impl Vals<u32>,
    keys: &[Key],
    floor: Option<u32>,
) -> Result<Option<u32>, Error> {
    let (mut at, mut step, mut best) = (0, 16, None::<u32>);
    while at < keys.len() {
        let end = (at + step).min(keys.len());
        vals.need(&keys[at..end])?;
        for k in &keys[at..end] {
            if let Some(&d) = vals.get(*k) {
                best = Some(best.map_or(d, |b| b.min(d)));
            }
        }
        if best.is_some() && best == floor {
            break;
        }
        at = end;
        step *= 4;
    }
    Ok(best)
}

/// Whether any of `keys` has value `want`, loading the values a few at a
/// time (a hub has many in-neighbours, and one supporter is enough).
fn has_value(vals: &mut impl Vals<u32>, keys: &[Key], want: u32) -> Result<bool, Error> {
    let mut at = 0;
    let mut step = 16;
    while at < keys.len() {
        let end = (at + step).min(keys.len());
        vals.need(&keys[at..end])?;
        if keys[at..end].iter().any(|k| vals.get(*k) == Some(&want)) {
            return Ok(true);
        }
        at = end;
        step *= 4;
    }
    Ok(false)
}

/// Union-find over search ids.
fn find(p: &mut [usize], mut x: usize) -> usize {
    while p[x] != x {
        p[x] = p[p[x]];
        x = p[x];
    }
    x
}

/// Weakly connected components in the later version, from the earlier
/// one's labels and the delta. First the split check, on the nodes present
/// in both versions with only the adjacencies both have: searches start
/// from every node that lost a neighbour, merge when they meet, and stop
/// once at most one is still growing; each finished search is a piece that
/// gets its least key. Then the merges: new nodes start alone, every new
/// adjacency unions two labels, and each component whose label is not the
/// least of its union is relabelled by a walk over the nodes it labels.
fn update_components(
    view: &mut impl View,
    vals: &mut impl Vals<u64>,
    kd: &KeyDelta,
) -> Result<(), Error> {
    let added: HashSet<Key> = kd.added.iter().copied().collect();
    let removed: HashSet<Key> = kd.removed.iter().copied().collect();
    // Endpoints the delta touches, with their old and new out-edges.
    let mut ends: BTreeSet<Key> = kd.old_out.keys().copied().collect();
    ends.extend(kd.edges_added.iter().flat_map(|e| [e.0, e.1]));
    ends.extend(kd.edges_removed.iter().flat_map(|e| [e.0, e.1]));
    let ends: Vec<Key> = ends.into_iter().collect();
    view.need(&ends)?;
    // In-neighbours of added and removed nodes are adjacencies that came
    // or went without being in the delta's edge lists.
    let around: Vec<Key> = (kd.added.iter().chain(&kd.removed))
        .flat_map(|k| view.inn(*k).to_vec())
        .collect();
    view.need(&around)?;
    let old_out = |view: &dyn ViewDyn, x: Key| -> Vec<Key> {
        match kd.old_out.get(&x) {
            Some((_, o)) => o.clone(),
            None => view.out_dyn(x).to_vec(),
        }
    };
    let old_present = |view: &dyn ViewDyn, x: Key| -> bool {
        match kd.old_out.get(&x) {
            Some((p, _)) => *p,
            None => view.present_dyn(x),
        }
    };
    let pair = |a: Key, b: Key| (a.min(b), a.max(b));
    // Candidate pairs: every edge added or removed, and every in-edge of an
    // added or removed node.
    let mut pairs: BTreeSet<(Key, Key)> = BTreeSet::new();
    for (u, w, _) in kd.edges_added.iter().chain(&kd.edges_removed) {
        if u != w {
            pairs.insert(pair(*u, *w));
        }
    }
    for k in kd.added.iter().chain(&kd.removed) {
        for s in view.inn(*k).to_vec() {
            if s != *k {
                pairs.insert(pair(s, *k));
            }
        }
    }
    let vd: &mut dyn ViewDyn = &mut DynWrap(view);
    let all: Vec<Key> = pairs.iter().flat_map(|p| [p.0, p.1]).collect();
    vd.need_dyn(&all)?;
    let mut new_pairs: HashSet<(Key, Key)> = HashSet::new();
    let mut touched: BTreeSet<Key> = BTreeSet::new();
    for &(a, b) in &pairs {
        let old_adj = old_present(vd, a)
            && old_present(vd, b)
            && (old_out(vd, a).contains(&b) || old_out(vd, b).contains(&a));
        let new_adj = vd.present_dyn(a)
            && vd.present_dyn(b)
            && (vd.out_dyn(a).contains(&b) || vd.out_dyn(b).contains(&a));
        if new_adj && !old_adj {
            new_pairs.insert((a, b));
        }
        if old_adj && !new_adj {
            for x in [a, b] {
                if vd.present_dyn(x) && !added.contains(&x) {
                    touched.insert(x);
                }
            }
        }
    }
    // Neighbours in the graph both versions share, each with whether it is
    // an in-edge's source (always a present node; an out-edge's target may
    // be a key no node holds).
    let mid_raw = |vd: &dyn ViewDyn, x: Key| -> Vec<(Key, bool)> {
        let outs = vd.out_dyn(x).iter().map(|y| (*y, false));
        let mut out: Vec<(Key, bool)> = (outs.chain(vd.inn_dyn(x).iter().map(|y| (*y, true))))
            .filter(|(y, _)| *y != x && !added.contains(y) && !new_pairs.contains(&pair(x, *y)))
            .collect();
        out.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
        out.dedup_by_key(|e| e.0);
        out
    };
    // Phase 1: splits, one old component at a time.
    let t: Vec<Key> = touched.iter().copied().collect();
    vals.need(&t)?;
    for r in &kd.removed {
        vals.set(*r, None);
    }
    let mut groups: BTreeMap<u64, Vec<Key>> = BTreeMap::new();
    for x in t {
        if let Some(&rep) = vals.get(x) {
            groups.entry(rep).or_default().push(x);
        }
    }
    for (rep, starts) in groups {
        let mut parent: Vec<usize> = (0..starts.len()).collect();
        let mut seen: HashMap<Key, usize> = HashMap::new();
        let mut frontier: Vec<Vec<Key>> = vec![Vec::new(); starts.len()];
        let mut done = vec![false; starts.len()];
        let mut absent: HashSet<Key> = HashSet::new();
        for (i, s) in starts.iter().enumerate() {
            seen.insert(*s, i);
            frontier[i].push(*s);
        }
        vd.need_dyn(&[rep])?;
        let rep_ok = vd.present_dyn(rep) && !removed.contains(&rep);
        loop {
            let roots: Vec<usize> = (0..starts.len())
                .filter(|i| find(&mut parent, *i) == *i)
                .collect();
            let live: Vec<usize> = roots.iter().copied().filter(|i| !done[*i]).collect();
            // One live search left: it holds the old label's node, so
            // nothing in it changes, unless that node left it or the graph.
            let rep_in_done = seen.get(&rep).is_some_and(|i| done[find(&mut parent, *i)]);
            if live.is_empty() || (live.len() == 1 && rep_ok && !rep_in_done) {
                break;
            }
            let i = *live.iter().min_by_key(|i| frontier[**i].len()).unwrap();
            // A target's presence is read only where it matters: when the
            // search expands it (an absent key has no adjacency and is not a
            // member) or when it is where two searches meet.
            let f = std::mem::take(&mut frontier[i]);
            vd.need_dyn(&f)?;
            let mut next = Vec::new();
            for x in f {
                if !vd.present_dyn(x) {
                    absent.insert(x);
                    continue;
                }
                for (y, is_in) in mid_raw(vd, x) {
                    match seen.get(&y) {
                        Some(&j) => {
                            let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                            if a == b {
                                continue;
                            }
                            if !is_in {
                                vd.need_dyn(&[y])?;
                                if !vd.present_dyn(y) {
                                    continue;
                                }
                            }
                            parent[b] = a;
                            let more = std::mem::take(&mut frontier[b]);
                            frontier[a].extend(more);
                        }
                        None => {
                            seen.insert(y, i);
                            next.push(y);
                        }
                    }
                }
            }
            let a = find(&mut parent, i);
            frontier[a].extend(next);
            if frontier[a].is_empty() {
                done[a] = true;
            }
        }
        // Each finished search is a whole piece: give it its least key.
        let mut pieces: BTreeMap<usize, Vec<Key>> = BTreeMap::new();
        for (x, i) in &seen {
            let r = find(&mut parent, *i);
            if done[r] && !absent.contains(x) {
                pieces.entry(r).or_default().push(*x);
            }
        }
        for members in pieces.into_values() {
            let least = *members.iter().min().unwrap();
            vals.need(&members)?;
            for m in members {
                if vals.get(m) != Some(&least) {
                    vals.set(m, Some(least));
                }
            }
        }
    }
    // Phase 2: merges.
    for a in &kd.added {
        if vd.present_dyn(*a) {
            vals.set(*a, Some(*a));
        }
    }
    let ends: Vec<Key> = new_pairs.iter().flat_map(|p| [p.0, p.1]).collect();
    vals.need(&ends)?;
    let mut labels: Vec<u64> = Vec::new();
    let mut at: HashMap<u64, usize> = HashMap::new();
    let mut start: HashMap<u64, Key> = HashMap::new();
    let mut parent: Vec<usize> = Vec::new();
    let mut id = |l: u64, x: Key, parent: &mut Vec<usize>| -> usize {
        *at.entry(l).or_insert_with(|| {
            labels.push(l);
            start.insert(l, x);
            parent.push(parent.len());
            parent.len() - 1
        })
    };
    let mut sorted: Vec<(Key, Key)> = new_pairs.iter().copied().collect();
    sorted.sort_unstable();
    for (a, b) in sorted {
        let (la, lb) = (*vals.get(a).ok_or_else(bad)?, *vals.get(b).ok_or_else(bad)?);
        let (i, j) = (id(la, a, &mut parent), id(lb, b, &mut parent));
        let (x, y) = (find(&mut parent, i), find(&mut parent, j));
        if x != y {
            parent[y] = x;
        }
    }
    let mut unions: BTreeMap<usize, Vec<u64>> = BTreeMap::new();
    for (i, l) in labels.iter().enumerate() {
        unions.entry(find(&mut parent, i)).or_default().push(*l);
    }
    for ls in unions.into_values() {
        let least = *ls.iter().min().unwrap();
        for l in ls.into_iter().filter(|l| *l != least) {
            // Walk the nodes labelled `l` and relabel them.
            let mut frontier = vec![start[&l]];
            let mut seen: HashSet<Key> = frontier.iter().copied().collect();
            while !frontier.is_empty() {
                vd.need_dyn(&frontier)?;
                let nb: Vec<Key> = (frontier.iter())
                    .flat_map(|x| {
                        let mut v = vd.out_dyn(*x).to_vec();
                        v.extend(vd.inn_dyn(*x));
                        v
                    })
                    .collect();
                vals.need(&nb)?;
                for x in &frontier {
                    vals.set(*x, Some(least));
                }
                let mut next = Vec::new();
                // A key with a label is a present node's.
                for y in nb {
                    if vals.get(y) == Some(&l) && seen.insert(y) {
                        next.push(y);
                    }
                }
                frontier = next;
            }
        }
    }
    Ok(())
}

/// `View` as a trait object, for the closures above.
trait ViewDyn {
    fn need_dyn(&mut self, keys: &[Key]) -> Result<(), Error>;
    fn present_dyn(&self, k: Key) -> bool;
    fn out_dyn(&self, k: Key) -> &[Key];
    fn inn_dyn(&self, k: Key) -> &[Key];
}

struct DynWrap<'a, V: View>(&'a mut V);

impl<V: View> ViewDyn for DynWrap<'_, V> {
    fn need_dyn(&mut self, keys: &[Key]) -> Result<(), Error> {
        self.0.need(keys)
    }
    fn present_dyn(&self, k: Key) -> bool {
        self.0.present(k)
    }
    fn out_dyn(&self, k: Key) -> &[Key] {
        self.0.out(k)
    }
    fn inn_dyn(&self, k: Key) -> &[Key] {
        self.0.inn(k)
    }
}

/// Root keys: a root counts while its label is a present node's (keys are
/// unique in a `put_graph` graph, so the key names it).
fn root_keys(roots: &[String]) -> BTreeSet<Key> {
    roots.iter().map(|r| key_of(r)).collect()
}

// ------------------------------------------------------------ the repo API

/// The results of one version of a graph: its root, its index and one
/// result per computation, by spec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Computed {
    pub graph: Id,
    pub index: Id,
    pub results: BTreeMap<String, Id>,
}

/// Edits to a result graph, as `graph_edit` takes them.
type Edits = BTreeMap<String, Option<Node>>;

/// What one computation cost.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cost {
    pub spec: String,
    /// Graph chunks read: the graph's, the index's and the result's.
    pub chunks_read: usize,
    /// Result entries (keys) written.
    pub changed: usize,
    pub seconds: f64,
}

impl<S: Store> Repo<S> {
    /// The delta between two versions of a graph: E249's walk, which skips
    /// every chunk both roots share, then the nodes left compared.
    pub fn graph_delta(&self, a: &Id, b: &Id) -> Result<GraphDelta, Error> {
        let (nodes, chunks_read) = self.graph_diff_counted(a, b, true)?;
        Ok(GraphDelta { nodes, chunks_read })
    }

    fn put_values(&mut self, values: Vec<(Key, Vec<Edge>)>) -> Result<Id, Error> {
        let nodes = values
            .into_iter()
            .map(|(k, mut edges)| {
                edges.sort();
                edges.dedup();
                Node {
                    label: hex(k),
                    target: Target::None,
                    edges,
                }
            })
            .collect();
        self.put_graph(nodes)
    }

    /// Every node of a graph, counting the chunks read.
    fn graph_nodes_counted(&self, root: &Id, reads: &mut usize) -> Result<Vec<Node>, Error> {
        let mut out = Vec::new();
        let mut stack = vec![*root];
        while let Some(id) = stack.pop() {
            *reads += 1;
            match &*self.graph_chunk_cached(&id)? {
                Graph::Nodes(nodes) => out.extend(nodes.iter().cloned()),
                Graph::Children(_, children) => stack.extend(children.iter().rev().map(|c| c.1)),
            }
        }
        Ok(out)
    }

    /// Every computation in `computes` (and the index) over the graph at
    /// `graph`, from scratch: every node read, every result put whole. The
    /// read is counted against each result, since each needs it.
    pub fn graph_compute(
        &mut self,
        graph: &Id,
        computes: &[Compute],
    ) -> Result<(Computed, Vec<Cost>), Error> {
        let at = std::time::Instant::now();
        let mut reads = 0;
        let nodes = self.graph_nodes_counted(graph, &mut reads)?;
        let mem = MemGraph::from_nodes(&nodes);
        let load = at.elapsed().as_secs_f64();
        let mut costs = Vec::new();
        let mut results = BTreeMap::new();
        let mut index = None;
        for c in std::iter::once(&Compute::Index).chain(computes) {
            let at = std::time::Instant::now();
            let id = self.put_values(mem.compute(c))?;
            match c {
                Compute::Index => index = Some(id),
                c => {
                    results.insert(c.spec(), id);
                }
            }
            costs.push(Cost {
                spec: c.spec(),
                chunks_read: reads,
                changed: 0,
                seconds: load + at.elapsed().as_secs_f64(),
            });
        }
        Ok((
            Computed {
                graph: *graph,
                index: index.unwrap(),
                results,
            },
            costs,
        ))
    }

    /// One computation from scratch: every node read, the result put whole.
    pub fn graph_compute_one(&mut self, graph: &Id, c: &Compute) -> Result<(Id, Cost), Error> {
        let at = std::time::Instant::now();
        let mut reads = 0;
        let nodes = self.graph_nodes_counted(graph, &mut reads)?;
        let id = self.put_values(MemGraph::from_nodes(&nodes).compute(c))?;
        let cost = Cost {
            spec: c.spec(),
            chunks_read: reads,
            changed: 0,
            seconds: at.elapsed().as_secs_f64(),
        };
        Ok((id, cost))
    }

    /// The results for the graph at `graph` from those of an earlier version
    /// (`prev`) and the delta between them (computed when not given). Index
    /// and degree come from the delta alone; hops and components from a
    /// traversal that starts where the delta does; PageRank is recomputed.
    /// Each result's root equals `graph_compute`'s.
    pub fn graph_compute_update(
        &mut self,
        prev: &Computed,
        graph: &Id,
        delta: Option<&GraphDelta>,
        computes: &[Compute],
    ) -> Result<(Computed, Vec<Cost>), Error> {
        let owned;
        let delta = match delta {
            Some(d) => d,
            None => {
                owned = self.graph_delta(&prev.graph, graph)?;
                &owned
            }
        };
        let kd = delta.keys();
        let mut costs = Vec::new();
        let at = std::time::Instant::now();
        let (edits, reads, changed) = {
            let mut v: FragVals<'_, S, Entry> = FragVals {
                repo: self,
                root: prev.index,
                vals: HashMap::new(),
                edits: BTreeMap::new(),
                chunks: ChunkCache::default(),
            };
            update_index(&mut v, &kd)?;
            let n = v.edits.len();
            (v.edits, v.chunks.reads, n)
        };
        let index = self.graph_edit(&prev.index, edits)?;
        costs.push(Cost {
            spec: Compute::Index.spec(),
            chunks_read: reads,
            changed,
            seconds: at.elapsed().as_secs_f64(),
        });
        let mut results = BTreeMap::new();
        for c in computes {
            let spec = c.spec();
            let old = *prev.results.get(&spec).ok_or_else(bad)?;
            let at = std::time::Instant::now();
            let (root, reads, changed) = match c {
                Compute::PageRank { .. } => {
                    let (id, cost) = self.graph_compute_one(graph, c)?;
                    (id, cost.chunks_read, 0)
                }
                _ => {
                    let (edits, reads) = self.update_one(c, old, graph, &index, &kd)?;
                    let n = edits.len();
                    (self.graph_edit(&old, edits)?, reads, n)
                }
            };
            results.insert(spec.clone(), root);
            costs.push(Cost {
                spec,
                chunks_read: reads,
                changed,
                seconds: at.elapsed().as_secs_f64(),
            });
        }
        Ok((
            Computed {
                graph: *graph,
                index,
                results,
            },
            costs,
        ))
    }

    fn update_one(
        &self,
        c: &Compute,
        old: Id,
        graph: &Id,
        index: &Id,
        kd: &KeyDelta,
    ) -> Result<(Edits, usize), Error> {
        let mut view = FragView {
            repo: self,
            graph: *graph,
            index: *index,
            adj: HashMap::new(),
            chunks: ChunkCache::default(),
        };
        match c {
            Compute::Degree => self.run_update(old, &mut view, |_, v| update_degree(v, kd)),
            Compute::Hops { roots, k } => {
                let rk = root_keys(roots);
                self.run_update(old, &mut view, |w, v| update_hops(w, v, kd, &rk, *k))
            }
            Compute::Components => {
                self.run_update(old, &mut view, |w, v| update_components(w, v, kd))
            }
            Compute::Index | Compute::PageRank { .. } => Err(bad()),
        }
    }

    fn run_update<'a, V: Val>(
        &'a self,
        old: Id,
        view: &mut FragView<'a, S>,
        f: impl FnOnce(&mut FragView<'a, S>, &mut FragVals<'a, S, V>) -> Result<(), Error>,
    ) -> Result<(Edits, usize), Error> {
        let mut vals: FragVals<'a, S, V> = FragVals {
            repo: self,
            root: old,
            vals: HashMap::new(),
            edits: BTreeMap::new(),
            chunks: ChunkCache::default(),
        };
        f(view, &mut vals)?;
        Ok((vals.edits, vals.chunks.reads + view.chunks.reads))
    }

    /// Record one computation of a version as a step graph: `func` (the
    /// spec, as content), `input` (the graph), `output` (the result),
    /// `index` (the index it read, for any but the index itself) and `from`
    /// (the step it was updated from, if any). Its root is the step's id.
    pub fn put_graph_step(
        &mut self,
        computed: &Computed,
        spec: &str,
        from: Option<Id>,
    ) -> Result<Id, Error> {
        let output = match spec == Compute::Index.spec() {
            true => computed.index,
            false => *computed.results.get(spec).ok_or_else(bad)?,
        };
        let func = self.put_content(format!("ikam/graph-compute/1 {spec}").as_bytes())?;
        let node = |label: &str, target| Node {
            label: label.into(),
            target,
            edges: Vec::new(),
        };
        let mut nodes = vec![
            node("func", Target::Content(Arg::Whole(func.id))),
            node("input", Target::Object(computed.graph)),
            node("output", Target::Object(output)),
        ];
        if spec != Compute::Index.spec() {
            nodes.push(node("index", Target::Object(computed.index)));
        }
        if let Some(f) = from {
            nodes.push(node("from", Target::Object(f)));
        }
        self.put_graph(nodes)
    }

    /// A step graph's computation, input and output.
    pub fn graph_step(&self, step: &Id) -> Result<(Compute, Id, Id), Error> {
        let nodes = self.graph_nodes(step)?;
        let get = |l: &str| nodes.iter().find(|n| n.label == l).map(|n| &n.target);
        let Some(Target::Content(func)) = get("func") else {
            return Err(bad());
        };
        let text = String::from_utf8(self.read_content(&func.id())?).map_err(|_| bad())?;
        let spec = text.strip_prefix("ikam/graph-compute/1 ").ok_or_else(bad)?;
        let c = Compute::parse(spec).ok_or_else(bad)?;
        let (Some(Target::Object(input)), Some(Target::Object(output))) =
            (get("input"), get("output"))
        else {
            return Err(bad());
        };
        Ok((c, *input, *output))
    }

    /// Rebuild a step's output from its input from scratch and compare:
    /// true when the stored output is the one a full recompute gives.
    pub fn graph_step_check(&mut self, step: &Id) -> Result<bool, Error> {
        let (c, input, output) = self.graph_step(step)?;
        let computes: Vec<Compute> = match c {
            Compute::Index => Vec::new(),
            ref c => vec![c.clone()],
        };
        let (fresh, _) = self.graph_compute(&input, &computes)?;
        Ok(match c {
            Compute::Index => fresh.index == output,
            c => fresh.results[&c.spec()] == output,
        })
    }

    /// The values of a result graph, by key.
    pub fn graph_values<V: Val>(&self, result: &Id) -> Result<BTreeMap<Key, V>, Error> {
        self.graph_nodes(result)?
            .into_iter()
            .map(|n| {
                Ok((
                    unhex(&n.label).ok_or_else(bad)?,
                    V::dec(&n.edges).ok_or_else(bad)?,
                ))
            })
            .collect()
    }
}

impl MemGraph {
    /// The in-memory baseline's incremental update: this graph is the later
    /// version (with `delta` already applied); `prev` holds the earlier
    /// version's values and becomes the later one's.
    pub fn update_index(&mut self, prev: &mut HashMap<Key, Entry>, delta: &GraphDelta) {
        update_index(prev, &delta.keys()).unwrap()
    }

    pub fn update_degree(&mut self, prev: &mut HashMap<Key, Deg>, delta: &GraphDelta) {
        update_degree(prev, &delta.keys()).unwrap()
    }

    pub fn update_hops(
        &mut self,
        prev: &mut HashMap<Key, u32>,
        delta: &GraphDelta,
        roots: &[String],
        k: Option<u32>,
    ) {
        let kd = delta.keys();
        let rk = root_keys(roots);
        update_hops(self, prev, &kd, &rk, k).unwrap()
    }

    pub fn update_components(&mut self, prev: &mut HashMap<Key, u64>, delta: &GraphDelta) {
        update_components(self, prev, &delta.keys()).unwrap()
    }

    /// Values of a computation from scratch, decoded.
    pub fn values<V: Val>(&self, c: &Compute) -> HashMap<Key, V> {
        self.compute(c)
            .into_iter()
            .map(|(k, e)| (k, V::dec(&e).unwrap()))
            .collect()
    }

    /// PageRank before quantizing, by key (what `Compute::PageRank` stores
    /// shifted right by its quantum).
    pub fn pagerank_raw(&self, iters: u32) -> HashMap<Key, u64> {
        let keys = self.present_keys();
        let x = pagerank(&keys, |k| &self.adj[&k].out, iters);
        keys.into_iter().zip(x).collect()
    }

    /// A bounded, approximate PageRank update (not exact; measured against
    /// the full recompute, never stored as one): the nodes within `hops`
    /// out-edges of a node the delta changed are re-iterated `iters` times
    /// from `prev`, every other node keeps its earlier rank, and the
    /// teleport and dangling terms are the earlier version's scaled to the
    /// new node count. Returns the ranks and how many nodes were re-iterated.
    pub fn pagerank_local(
        &self,
        prev: &HashMap<Key, u64>,
        delta: &GraphDelta,
        hops: u32,
        iters: u32,
    ) -> (HashMap<Key, u64>, usize) {
        let kd = delta.keys();
        let n = self.present_keys().len().max(1) as u128;
        let mut x: HashMap<Key, u64> = (prev.iter())
            .filter(|(k, _)| self.present(**k))
            .map(|(k, v)| (*k, *v))
            .collect();
        let mut seeds: BTreeSet<Key> = kd.new_out.keys().copied().collect();
        seeds.extend(kd.edges_removed.iter().map(|e| e.1));
        seeds.extend(kd.edges_added.iter().map(|e| e.1));
        let mut area: BTreeSet<Key> = seeds.iter().copied().filter(|k| self.present(*k)).collect();
        let mut frontier: Vec<Key> = area.iter().copied().collect();
        for _ in 0..hops {
            let mut next = Vec::new();
            for f in frontier {
                for y in live_out(self, f) {
                    if area.insert(y) {
                        next.push(y);
                    }
                }
            }
            frontier = next;
        }
        // The earlier teleport-plus-dangling term, as the least rank of a
        // node with no live in-edges (or the mass spread evenly).
        let base_prev = (prev.iter())
            .filter(|(k, _)| self.present(**k) && self.inn(**k).iter().all(|s| !self.present(*s)))
            .map(|(_, v)| *v)
            .min()
            .unwrap_or((MASS as u128 / n) as u64);
        let base = (base_prev as u128 * prev.len().max(1) as u128 / n) as u64;
        for a in &area {
            x.entry(*a).or_insert(base);
        }
        let mut outdeg: HashMap<Key, u128> = HashMap::new();
        for v in &area {
            for u in self.inn(*v) {
                outdeg
                    .entry(*u)
                    .or_insert_with(|| live_out(self, *u).len().max(1) as u128);
            }
        }
        for _ in 0..iters {
            let mut next = HashMap::new();
            for v in &area {
                let mut r = base as u128;
                for u in self.inn(*v) {
                    if let Some(xu) = x.get(u) {
                        r += *xu as u128 * 85 / (100 * outdeg[u]);
                    }
                }
                next.insert(*v, r as u64);
            }
            x.extend(next);
        }
        (x, area.len())
    }
}
