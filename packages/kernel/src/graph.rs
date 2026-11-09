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

/// Encoded size of a node, as chunk boundaries count it.
fn node_size(n: &Node) -> usize {
    Object::Graph(Graph::Nodes(vec![n.clone()])).encode().len()
}

/// Size of a child entry, as chunk boundaries count it.
fn child_size(c: &(String, Id)) -> usize {
    c.0.len() + 36
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
        let mut level: Vec<(String, Id)> = Vec::new();
        let mut start = 0;
        for end in cut(&nodes, |n| &n.label, node_size) {
            let chunk = nodes[start..end].to_vec();
            let first = chunk.first().map_or(String::new(), |n| n.label.clone());
            level.push((first, self.put(&Object::Graph(Graph::Nodes(chunk)))?));
            start = end;
        }
        self.put_parents(level, 1)
    }

    /// Build the levels above `level` (whose chunks are at height
    /// `height - 1`) up to one root, as `put_graph` does.
    fn put_parents(&mut self, mut level: Vec<(String, Id)>, height: u8) -> Result<Id, Error> {
        let mut height = height - 1;
        while level.len() > 1 {
            height += 1;
            let (mut parents, mut start) = (Vec::new(), 0);
            for end in cut(&level, |c| &c.0, child_size) {
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

    /// Every node of a graph, in label order. One pass: each chunk is read
    /// once (finding the leaves first, then reading them, read every leaf
    /// twice; E043).
    pub fn graph_nodes(&self, root: &Id) -> Result<Vec<Node>, Error> {
        let mut out = Vec::new();
        self.collect_nodes(root, &mut out)?;
        Ok(out)
    }

    fn collect_nodes(&self, id: &Id, out: &mut Vec<Node>) -> Result<(), Error> {
        match self.graph_chunk(id)? {
            Graph::Nodes(nodes) => out.extend(nodes),
            Graph::Children(_, children) => {
                for (_, child) in children {
                    self.collect_nodes(&child, out)?;
                }
            }
        }
        Ok(())
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

    /// The nodes labelled with any of `labels`, reading each chunk that can
    /// hold one of them once.
    pub fn graph_nodes_in(&self, root: &Id, labels: &BTreeSet<String>) -> Result<Vec<Node>, Error> {
        let mut out = Vec::new();
        if !labels.is_empty() {
            self.collect_in(root, labels, &mut out)?;
        }
        Ok(out)
    }

    fn collect_in(
        &self,
        id: &Id,
        labels: &BTreeSet<String>,
        out: &mut Vec<Node>,
    ) -> Result<(), Error> {
        match self.graph_chunk(id)? {
            Graph::Nodes(nodes) => {
                out.extend(nodes.into_iter().filter(|n| labels.contains(&n.label)))
            }
            Graph::Children(_, children) => {
                for (i, (first, child)) in children.iter().enumerate() {
                    // The first child also holds labels before its first.
                    let low = if i == 0 { String::new() } else { first.clone() };
                    let mine: BTreeSet<String> = match children.get(i + 1) {
                        Some((next, _)) => labels.range(low..next.clone()).cloned().collect(),
                        None => labels.range(low..).cloned().collect(),
                    };
                    if !mine.is_empty() {
                        self.collect_in(child, &mine, out)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The last node of a graph, reading one chunk per level.
    pub fn graph_last(&self, root: &Id) -> Result<Option<Node>, Error> {
        let mut id = *root;
        loop {
            match self.graph_chunk(&id)? {
                Graph::Nodes(nodes) => return Ok(nodes.into_iter().last()),
                Graph::Children(_, children) => match children.last() {
                    Some((_, child)) => id = *child,
                    None => return Ok(None),
                },
            }
        }
    }

    /// Nodes whose labels start with `prefix`, in label order, reading only
    /// the chunks that can hold them.
    pub fn graph_prefix(&self, root: &Id, prefix: &str) -> Result<Vec<Node>, Error> {
        let mut out = Vec::new();
        self.collect_prefix(root, prefix, &mut out)?;
        Ok(out)
    }

    fn collect_prefix(&self, id: &Id, prefix: &str, out: &mut Vec<Node>) -> Result<(), Error> {
        match self.graph_chunk(id)? {
            Graph::Nodes(nodes) => {
                out.extend(nodes.into_iter().filter(|n| n.label.starts_with(prefix)))
            }
            Graph::Children(_, children) => {
                for (i, (first, child)) in children.iter().enumerate() {
                    // A child holds labels from its first up to the next child's.
                    let starts_low = first.as_str() < prefix || first.starts_with(prefix);
                    let ends_high = children
                        .get(i + 1)
                        .is_none_or(|(next, _)| next.as_str() > prefix);
                    if starts_low && ends_high {
                        self.collect_prefix(child, prefix, out)?;
                    }
                }
            }
        }
        Ok(())
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

/// A level's entry: a node, or a child `(first label, chunk id)`.
trait Item: Clone {
    fn label(&self) -> &str;
    fn size(&self) -> usize;
}

impl Item for Node {
    fn label(&self) -> &str {
        &self.label
    }
    fn size(&self) -> usize {
        node_size(self)
    }
}

impl Item for (String, Id) {
    fn label(&self) -> &str {
        &self.0
    }
    fn size(&self) -> usize {
        child_size(self)
    }
}

/// A chunk's entries with `edits` applied: `Some` sets an entry, `None`
/// drops it (absent or not).
fn merge<T: Item>(items: Vec<T>, edits: &[(&String, &Option<T>)]) -> Vec<T> {
    let mut map: BTreeMap<String, T> = items
        .into_iter()
        .map(|t| (t.label().to_string(), t))
        .collect();
    for (label, edit) in edits {
        match edit {
            Some(t) => map.insert((*label).clone(), t.clone()),
            None => map.remove(*label),
        };
    }
    map.into_values().collect()
}

/// Entries that differ between two sorted levels, as edits.
fn level_edits(
    old: &[(String, Id)],
    new: &[(String, Id)],
) -> BTreeMap<String, Option<(String, Id)>> {
    let a: HashSet<&(String, Id)> = old.iter().collect();
    let b: HashSet<&(String, Id)> = new.iter().collect();
    let mut edits = BTreeMap::new();
    for c in old.iter().filter(|c| !b.contains(c)) {
        edits.insert(c.0.clone(), None);
    }
    for c in new.iter().filter(|c| !a.contains(c)) {
        edits.insert(c.0.clone(), Some(c.clone()));
    }
    edits
}

impl<S: Store> Repo<S> {
    /// Apply `edits` to the graph at `root` (`Some` sets a node, `None`
    /// drops a label, present or not) and return the new root. Only the
    /// chunks the edits fall in are read and rewritten, plus neighbours
    /// until the content-defined cut falls back into step, and the levels
    /// above them; the root equals `put_graph` of the edited node set
    /// (tests/graphs.rs). Label keys are checked for clashes only within
    /// the chunks rewritten; `put_graph` checks the whole graph.
    pub fn graph_edit(
        &mut self,
        root: &Id,
        edits: BTreeMap<String, Option<Node>>,
    ) -> Result<Id, Error> {
        if edits.is_empty() {
            return Ok(*root);
        }
        let mut edits = edits;
        for (label, node) in edits.iter_mut() {
            if let Some(n) = node {
                if &n.label != label {
                    return Err(Error::InvalidName(label.clone()));
                }
                n.edges.sort();
                n.edges.dedup();
            }
        }
        let levels = self.graph_levels(root)?;
        let mut items = self.splice(
            &levels[0],
            &edits,
            |r, id| match r.graph_chunk(id)? {
                Graph::Nodes(nodes) => Ok(nodes),
                _ => Err(Error::WrongKind(*id)),
            },
            |nodes| {
                let mut keys = HashSet::new();
                match nodes.iter().find(|n| !keys.insert(node_key(&n.label))) {
                    Some(n) => Err(Error::InvalidName(n.label.clone())),
                    None => Ok(Graph::Nodes(nodes)),
                }
            },
        )?;
        if items.is_empty() {
            return self.put_graph(Vec::new());
        }
        let mut h = 1;
        loop {
            if items.len() == 1 {
                return Ok(items[0].1);
            }
            let height = u8::try_from(h).map_err(|_| Error::Decode("graph too tall"))?;
            let Some(chunks) = levels.get(h) else {
                return self.put_parents(items, height);
            };
            let edits = level_edits(&levels[h - 1], &items);
            items = self.splice(
                chunks,
                &edits,
                |r, id| match r.graph_chunk(id)? {
                    Graph::Children(_, children) => Ok(children),
                    _ => Err(Error::WrongKind(*id)),
                },
                |children| Ok(Graph::Children(height, children)),
            )?;
            h += 1;
        }
    }

    /// The chunk lists of a graph, bottom up: `[0]` lists its leaves as
    /// `(first label, id)`, and the last holds only the root. Reads every
    /// chunk above the leaves, and no leaf.
    fn graph_levels(&self, root: &Id) -> Result<Vec<Vec<(String, Id)>>, Error> {
        let mut levels = vec![vec![(String::new(), *root)]];
        loop {
            let (mut next, mut leaves) = (Vec::new(), false);
            for (_, id) in levels.last().unwrap() {
                match self.graph_chunk(id)? {
                    Graph::Children(height, children) => {
                        leaves = height == 1;
                        next.extend(children)
                    }
                    Graph::Nodes(_) => break,
                }
            }
            if next.is_empty() {
                break;
            }
            levels.push(next);
            if leaves {
                break;
            }
        }
        levels.reverse();
        Ok(levels)
    }

    /// Re-cut one level where `edits` fall: each run of chunks from the one
    /// an edit falls in until a new boundary lands on an old one. Returns
    /// the level's new chunk list; untouched chunks keep their ids.
    fn splice<T: Item>(
        &mut self,
        chunks: &[(String, Id)],
        edits: &BTreeMap<String, Option<T>>,
        load: impl Fn(&Self, &Id) -> Result<Vec<T>, Error>,
        make: impl Fn(Vec<T>) -> Result<Graph, Error>,
    ) -> Result<Vec<(String, Id)>, Error> {
        let mut by_chunk: BTreeMap<usize, Vec<(&String, &Option<T>)>> = BTreeMap::new();
        for (label, edit) in edits {
            let at = chunks.partition_point(|c| c.0.as_str() <= label.as_str());
            by_chunk
                .entry(at.saturating_sub(1))
                .or_default()
                .push((label, edit));
        }
        let mut out = Vec::new();
        let put = |r: &mut Self, items: Vec<T>| -> Result<(String, Id), Error> {
            let first = items[0].label().to_string();
            Ok((first, r.put(&Object::Graph(make(items)?))?))
        };
        let mut k = 0;
        while k < chunks.len() {
            if !by_chunk.contains_key(&k) {
                out.push(chunks[k].clone());
                k += 1;
                continue;
            }
            let (mut buf, mut acc) = (Vec::new(), 0);
            loop {
                let edits = by_chunk.get(&k).map_or(&[][..], |e| &e[..]);
                let items = merge(load(self, &chunks[k].1)?, edits);
                k += 1;
                for item in items {
                    acc += item.size();
                    let ends = ends_chunk(item.label(), acc);
                    buf.push(item);
                    if ends {
                        out.push(put(self, std::mem::take(&mut buf))?);
                        acc = 0;
                    }
                }
                if buf.is_empty() || k == chunks.len() {
                    break;
                }
            }
            if !buf.is_empty() {
                out.push(put(self, buf)?);
            }
        }
        Ok(out)
    }
}
