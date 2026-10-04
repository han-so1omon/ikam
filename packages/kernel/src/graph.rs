//! Graphs as prolly trees of `G` chunks (Noms/Dolt): versioned, sliceable
//! and deduplicated by structural sharing, with cycles allowed.
//!
//! Nodes are sorted by label and cut into chunks where the content says so,
//! never at fixed positions: a chunk ends after a node whose label hashes
//! below a threshold that rises with the chunk's size (Dolt's size-aware
//! rule), so an edit moves at most nearby boundaries and every other chunk,
//! and its id, is unchanged. Parent chunks list `(first label, child id)`
//! and are cut the same way, up to one root. The same labelled graph always
//! has the same root, whatever order it was built in.
//!
//! Edges name their target by `node_key(label)`, never by hash link, so they
//! may form cycles; they are not on the rebuild path, and gc never follows
//! them.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::{Error, Graph, Id, Node, Object, Repo, Store, node_key};

/// A node that differs between two graphs: `(label, before, after)`.
pub type NodeChange = (String, Option<Node>, Option<Node>);

/// No chunk ends before this many encoded bytes.
const MIN_CHUNK: usize = 4096;
/// Every chunk ends by this many encoded bytes.
const MAX_CHUNK: usize = 32768;

/// True if a chunk of `size` bytes ends after the entry labelled `label`:
/// the boundary probability rises linearly from 0 at `MIN_CHUNK` to 1 at
/// `MAX_CHUNK`, decided by the label's hash alone.
fn ends_chunk(label: &str, size: usize) -> bool {
    let h = u64::from_be_bytes(node_key(label)) as f64 / u64::MAX as f64;
    size >= MAX_CHUNK
        || (size >= MIN_CHUNK && h < (size - MIN_CHUNK) as f64 / (MAX_CHUNK - MIN_CHUNK) as f64)
}

/// Cut `items` (label, encoded size) into chunks; returns end indices.
fn cut<T>(items: &[T], label: impl Fn(&T) -> &str, size: impl Fn(&T) -> usize) -> Vec<usize> {
    let (mut ends, mut acc) = (Vec::new(), 0);
    for (i, item) in items.iter().enumerate() {
        acc += size(item);
        if ends_chunk(label(item), acc) {
            ends.push(i + 1);
            acc = 0;
        }
    }
    if ends.last() != Some(&items.len()) {
        ends.push(items.len());
    }
    ends
}

impl<S: Store> Repo<S> {
    /// Store a graph and return its root id. Labels must be unique, and so
    /// must their keys within the graph; edges are sorted and deduplicated.
    pub fn put_graph(&mut self, mut nodes: Vec<Node>) -> Result<Id, Error> {
        nodes.sort_by(|a, b| a.label.cmp(&b.label));
        let mut keys = HashSet::new();
        for n in nodes.iter_mut() {
            if !keys.insert(node_key(&n.label)) {
                return Err(Error::InvalidName(n.label.clone()));
            }
            n.edges.sort();
            n.edges.dedup();
        }
        let size = |n: &Node| Object::Graph(Graph::Nodes(vec![n.clone()])).encode().len();
        let mut level: Vec<(String, Id)> = Vec::new();
        let mut start = 0;
        for end in cut(&nodes, |n| &n.label, size) {
            let chunk = nodes[start..end].to_vec();
            let first = chunk.first().map_or(String::new(), |n| n.label.clone());
            level.push((first, self.put(&Object::Graph(Graph::Nodes(chunk)))?));
            start = end;
        }
        let mut height = 0u8;
        while level.len() > 1 {
            height += 1;
            let (mut parents, mut start) = (Vec::new(), 0);
            for end in cut(&level, |c| &c.0, |c| c.0.len() + 36) {
                let children = level[start..end].to_vec();
                let id = self.put(&Object::Graph(Graph::Children(height, children)))?;
                parents.push((level[start].0.clone(), id));
                start = end;
            }
            level = parents;
        }
        Ok(level[0].1)
    }

    fn graph_chunk(&self, id: &Id) -> Result<Graph, Error> {
        match self.get(id)? {
            Object::Graph(g) => Ok(g),
            _ => Err(Error::WrongKind(*id)),
        }
    }

    /// Ids of the node chunks under `root`, in label order.
    fn graph_leaves(&self, root: &Id) -> Result<Vec<Id>, Error> {
        match self.graph_chunk(root)? {
            Graph::Nodes(_) => Ok(vec![*root]),
            Graph::Children(_, children) => {
                let mut out = Vec::new();
                for (_, id) in children {
                    out.extend(self.graph_leaves(&id)?);
                }
                Ok(out)
            }
        }
    }

    /// Every node of a graph, in label order.
    pub fn graph_nodes(&self, root: &Id) -> Result<Vec<Node>, Error> {
        let mut out = Vec::new();
        for leaf in self.graph_leaves(root)? {
            if let Graph::Nodes(nodes) = self.graph_chunk(&leaf)? {
                out.extend(nodes);
            }
        }
        Ok(out)
    }

    /// The node labelled `label`, reading one chunk per level.
    pub fn graph_node(&self, root: &Id, label: &str) -> Result<Option<Node>, Error> {
        let mut id = *root;
        loop {
            match self.graph_chunk(&id)? {
                Graph::Nodes(nodes) => return Ok(nodes.into_iter().find(|n| n.label == label)),
                Graph::Children(_, children) => {
                    match children
                        .iter()
                        .rev()
                        .find(|(first, _)| first.as_str() <= label)
                    {
                        Some((_, child)) => id = *child,
                        None => return Ok(None),
                    }
                }
            }
        }
    }

    /// The induced subgraph on `labels`: those nodes, and their edges to
    /// nodes among them. Its id depends only on that subgraph.
    pub fn graph_slice(&mut self, root: &Id, labels: &BTreeSet<String>) -> Result<Id, Error> {
        let keys: HashSet<[u8; 8]> = labels.iter().map(|l| node_key(l)).collect();
        let mut nodes: Vec<Node> = self
            .graph_nodes(root)?
            .into_iter()
            .filter(|n| labels.contains(&n.label))
            .collect();
        nodes
            .iter_mut()
            .for_each(|n| n.edges.retain(|e| keys.contains(&e.to)));
        self.put_graph(nodes)
    }

    /// Nodes that differ between two graphs: `(label, before, after)`.
    /// Chunks the two share are skipped without being read.
    pub fn graph_diff(&self, a: &Id, b: &Id) -> Result<Vec<NodeChange>, Error> {
        let (la, lb) = (self.graph_leaves(a)?, self.graph_leaves(b)?);
        let (sa, sb): (HashSet<Id>, HashSet<Id>) =
            (la.iter().copied().collect(), lb.iter().copied().collect());
        let mut side: BTreeMap<String, (Option<Node>, Option<Node>)> = BTreeMap::new();
        for (leaves, other, before) in [(&la, &sb, true), (&lb, &sa, false)] {
            for leaf in leaves.iter().filter(|l| !other.contains(l)) {
                if let Graph::Nodes(nodes) = self.graph_chunk(leaf)? {
                    for n in nodes {
                        let slot = side.entry(n.label.clone()).or_default();
                        if before {
                            slot.0 = Some(n)
                        } else {
                            slot.1 = Some(n)
                        }
                    }
                }
            }
        }
        Ok(side
            .into_iter()
            .filter(|(_, (x, y))| x != y)
            .map(|(label, (x, y))| (label, x, y))
            .collect())
    }
}
