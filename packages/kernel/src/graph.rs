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

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::{Error, Graph, Id, Node, Object, Repo, Store, node_key};

/// A node that differs between two graphs: `(label, before, after)`.
pub type NodeChange = (String, Option<Node>, Option<Node>);

/// No chunk ends before this many encoded bytes.
const MIN_CHUNK: usize = 4096;
/// Every chunk ends by this many encoded bytes.
const MAX_CHUNK: usize = 32768;

/// Whether two labels of a graph may share a key (see `node_key`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keys {
    /// Every label has its own key, so an edge names one node. What
    /// `put_graph` requires.
    Unique,
    /// Keys may repeat. For graphs no edge points into, such as the
    /// database's `ids` and `kv` views, whose labels clients choose: a
    /// repeated key there changes nothing, since nodes are found by label.
    Any,
}

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
    pub fn put_graph(&mut self, nodes: Vec<Node>) -> Result<Id, Error> {
        self.put_graph_with(nodes, Keys::Unique)
    }

    /// `put_graph`, with `keys` saying whether label keys must be unique.
    /// Labels must be unique either way. The root of a graph whose keys
    /// are unique is the same with either.
    pub fn put_graph_with(&mut self, mut nodes: Vec<Node>, keys: Keys) -> Result<Id, Error> {
        nodes.sort_by(|a, b| a.label.cmp(&b.label));
        if let Some(w) = nodes.windows(2).find(|w| w[0].label == w[1].label) {
            return Err(Error::InvalidName(w[1].label.clone()));
        }
        let mut seen = HashSet::new();
        for n in nodes.iter_mut() {
            if keys == Keys::Unique && !seen.insert(node_key(&n.label)) {
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

    /// Nodes with `start <= label < end` (no `end`: to the last label), in
    /// label order or, with `reverse`, from the last; at most `limit`. Reads
    /// only the chunks that can hold such a label, and stops once it has
    /// `limit`, so a page of a long range costs one path plus its leaves.
    pub fn graph_range(
        &self,
        root: &Id,
        start: &str,
        end: Option<&str>,
        limit: usize,
        reverse: bool,
    ) -> Result<Vec<Node>, Error> {
        let mut out = Vec::new();
        if limit > 0 && end.is_none_or(|e| start < e) {
            self.collect_range(root, start, end, limit, reverse, &mut out)?;
        }
        Ok(out)
    }

    fn collect_range(
        &self,
        id: &Id,
        start: &str,
        end: Option<&str>,
        limit: usize,
        reverse: bool,
        out: &mut Vec<Node>,
    ) -> Result<(), Error> {
        let inside = |l: &str| l >= start && end.is_none_or(|e| l < e);
        match self.graph_chunk(id)? {
            Graph::Nodes(nodes) => {
                let take = limit - out.len();
                if reverse {
                    out.extend(
                        nodes
                            .into_iter()
                            .rev()
                            .filter(|n| inside(&n.label))
                            .take(take),
                    );
                } else {
                    out.extend(nodes.into_iter().filter(|n| inside(&n.label)).take(take));
                }
            }
            Graph::Children(_, children) => {
                // Child i holds labels from its first (the first child: from
                // the start) up to the next child's first.
                let mut picked = Vec::new();
                for (i, (first, child)) in children.iter().enumerate() {
                    let low_ok = end.is_none_or(|e| i == 0 || first.as_str() < e);
                    let high_ok = children
                        .get(i + 1)
                        .is_none_or(|(next, _)| next.as_str() > start);
                    if low_ok && high_ok {
                        picked.push(*child);
                    }
                }
                if reverse {
                    picked.reverse();
                }
                for child in picked {
                    if out.len() >= limit {
                        break;
                    }
                    self.collect_range(&child, start, end, limit, reverse, out)?;
                }
            }
        }
        Ok(())
    }

    /// Nodes whose labels start with `prefix`, in label order, reading only
    /// the chunks that can hold them.
    pub fn graph_prefix(&self, root: &Id, prefix: &str) -> Result<Vec<Node>, Error> {
        let mut out = Vec::new();
        self.collect_prefix(root, prefix, &mut out)?;
        Ok(out)
    }

    /// `graph_prefix` of the graph that `graph_edit(root, pending)` would
    /// make, without making it (E148): the stored nodes, overridden or
    /// dropped by `pending`. A server can keep a view's new edits pending
    /// and fold them in with one `graph_edit` every few goals; reads see
    /// the same nodes either way.
    pub fn graph_prefix_pending(
        &self,
        root: &Id,
        pending: &BTreeMap<String, Option<Node>>,
        prefix: &str,
    ) -> Result<Vec<Node>, Error> {
        let mut out: BTreeMap<String, Node> = self
            .graph_prefix(root, prefix)?
            .into_iter()
            .map(|n| (n.label.clone(), n))
            .collect();
        for (label, node) in pending
            .range(prefix.to_string()..)
            .take_while(|(l, _)| l.starts_with(prefix))
        {
            match node {
                Some(n) => {
                    let mut n = n.clone();
                    n.edges.sort();
                    n.edges.dedup();
                    out.insert(label.clone(), n);
                }
                None => {
                    out.remove(label);
                }
            }
        }
        Ok(out.into_values().collect())
    }

    /// `graph_node` of the graph that `graph_edit(root, pending)` would make.
    pub fn graph_node_pending(
        &self,
        root: &Id,
        pending: &BTreeMap<String, Option<Node>>,
        label: &str,
    ) -> Result<Option<Node>, Error> {
        match pending.get(label) {
            Some(Some(n)) => {
                let mut n = n.clone();
                n.edges.sort();
                n.edges.dedup();
                Ok(Some(n))
            }
            Some(None) => Ok(None),
            None => self.graph_node(root, label),
        }
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

/// Child entries that changed in each run (the old entries a run replaced
/// and the new ones it wrote), as edits one level up. Runs cover disjoint
/// label ranges.
fn run_edits(runs: Vec<Run>) -> BTreeMap<String, Option<(String, Id)>> {
    let mut edits = BTreeMap::new();
    for (old, new) in runs {
        let a: HashSet<&(String, Id)> = old.iter().collect();
        let b: HashSet<&(String, Id)> = new.iter().collect();
        for c in old.iter().filter(|c| !b.contains(c)) {
            edits.insert(c.0.clone(), None);
        }
        for c in new.iter().filter(|c| !a.contains(c)) {
            edits.insert(c.0.clone(), Some(c.clone()));
        }
    }
    edits
}

/// One re-cut stretch of a level: the chunks it replaced and the chunks it
/// wrote, as `(first label, id)`.
type Run = (Vec<(String, Id)>, Vec<(String, Id)>);

/// Child lists read during one edit, by chunk id, so each upper chunk is
/// read at most once.
type Upper = HashMap<Id, Vec<(String, Id)>>;

/// A position among the chunks at one height: the child lists from the
/// root down to that height, each with the index taken. An empty path is
/// the root itself.
struct Cursor {
    root: Id,
    path: Vec<(Vec<(String, Id)>, usize)>,
}

impl Cursor {
    fn entry(&self) -> (String, Id) {
        match self.path.last() {
            Some((list, i)) => list[*i].clone(),
            None => (String::new(), self.root),
        }
    }
}

impl<S: Store> Repo<S> {
    /// Apply `edits` to the graph at `root` (`Some` sets a node, `None`
    /// drops a label, present or not) and return the new root, which
    /// equals `put_graph` of the edited node set (tests/graphs.rs). Reads
    /// only the chunks the edits fall in, their neighbours until the
    /// content-defined cut falls back into step, and the path from the
    /// root to each. Label keys are checked only within the chunks
    /// rewritten, so a clash elsewhere is found by `put_graph`
    /// (`db_rebuild`), not here; graphs no edge points into skip the check
    /// with `graph_edit_with` and `Keys::Any`.
    pub fn graph_edit(
        &mut self,
        root: &Id,
        edits: BTreeMap<String, Option<Node>>,
    ) -> Result<Id, Error> {
        self.graph_edit_with(root, edits, Keys::Unique)
    }

    /// `graph_edit`, with `keys` saying whether label keys are checked.
    /// With `Keys::Any` the root equals `put_graph_with(nodes, Keys::Any)`
    /// in every case, clashing keys included.
    pub fn graph_edit_with(
        &mut self,
        root: &Id,
        edits: BTreeMap<String, Option<Node>>,
        keys: Keys,
    ) -> Result<Id, Error> {
        let began = self.begin_memo();
        let hinting = self.begin_hints();
        let out = self.graph_edit_inner(root, edits, keys);
        self.end_hints(hinting);
        self.end_memo(began);
        out
    }

    fn graph_edit_inner(
        &mut self,
        root: &Id,
        edits: BTreeMap<String, Option<Node>>,
        keys: Keys,
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
        let mut upper = Upper::new();
        let top = match self.graph_chunk(root)? {
            Graph::Nodes(_) => 0,
            Graph::Children(height, children) => {
                upper.insert(*root, children);
                height
            }
        };
        let mut runs = self.splice(
            &mut upper,
            root,
            top,
            0,
            &edits,
            |r, _, id| match r.graph_chunk(id)? {
                Graph::Nodes(nodes) => Ok(nodes),
                _ => Err(Error::WrongKind(*id)),
            },
            |nodes| {
                if keys == Keys::Unique {
                    let mut seen = HashSet::new();
                    if let Some(n) = nodes.iter().find(|n| !seen.insert(node_key(&n.label))) {
                        return Err(Error::InvalidName(n.label.clone()));
                    }
                }
                Ok(Graph::Nodes(nodes))
            },
        )?;
        for height in 1..=top {
            let up = run_edits(runs);
            if up.is_empty() {
                return Ok(*root);
            }
            runs = self.splice(
                &mut upper,
                root,
                top,
                height,
                &up,
                |r, upper, id| r.children(upper, id),
                |children| Ok(Graph::Children(height, children)),
            )?;
        }
        // At the root's height the one run is the whole level.
        let Some((_, level)) = runs.into_iter().next() else {
            return Ok(*root);
        };
        match level.len() {
            0 => self.put_graph(Vec::new()),
            1 => {
                // The levels below may have shrunk to one chunk too.
                let mut id = level[0].1;
                while let Graph::Children(_, children) = self.graph_chunk(&id)? {
                    if children.len() != 1 {
                        break;
                    }
                    id = children[0].1;
                }
                Ok(id)
            }
            _ => {
                let height = top.checked_add(1).ok_or(Error::Decode("graph too tall"))?;
                self.put_parents(level, height)
            }
        }
    }

    /// The child list of an upper chunk, read once per edit.
    fn children(&self, upper: &mut Upper, id: &Id) -> Result<Vec<(String, Id)>, Error> {
        if let Some(children) = upper.get(id) {
            return Ok(children.clone());
        }
        match self.graph_chunk(id)? {
            Graph::Children(_, children) => {
                upper.insert(*id, children.clone());
                Ok(children)
            }
            _ => Err(Error::WrongKind(*id)),
        }
    }

    /// A cursor at the chunk of `height` that `label` falls in, under a
    /// root of height `top`: the last whose first label is at most
    /// `label`, or the first. Reads one child list per level above.
    fn cursor_at(
        &self,
        upper: &mut Upper,
        root: &Id,
        top: u8,
        height: u8,
        label: &str,
    ) -> Result<Cursor, Error> {
        let mut path = Vec::new();
        let mut id = *root;
        for _ in height..top {
            let list = self.children(upper, &id)?;
            let i = list
                .partition_point(|c| c.0.as_str() <= label)
                .saturating_sub(1);
            id = list[i].1;
            path.push((list, i));
        }
        Ok(Cursor { root: *root, path })
    }

    /// Move a cursor to the next chunk at its height; false at the end of
    /// the level, after which the cursor is not used.
    fn cursor_next(&self, upper: &mut Upper, cursor: &mut Cursor) -> Result<bool, Error> {
        let depth = cursor.path.len();
        let mut d = depth;
        loop {
            if d == 0 {
                return Ok(false);
            }
            let (list, i) = &mut cursor.path[d - 1];
            if *i + 1 < list.len() {
                *i += 1;
                break;
            }
            d -= 1;
        }
        cursor.path.truncate(d);
        while cursor.path.len() < depth {
            let (list, i) = cursor.path.last().unwrap();
            let id = list[*i].1;
            let child = self.children(upper, &id)?;
            cursor.path.push((child, 0));
        }
        Ok(true)
    }

    /// Re-cut the chunks at `height` that `edits` fall in: each run goes
    /// from the chunk an edit falls in until a new boundary lands on an
    /// old one, or the level ends. Returns the runs; chunks outside them
    /// are neither read nor rewritten. At the root's height the one run
    /// is the whole new level.
    #[allow(clippy::too_many_arguments)]
    fn splice<T: Item>(
        &mut self,
        upper: &mut Upper,
        root: &Id,
        top: u8,
        height: u8,
        edits: &BTreeMap<String, Option<T>>,
        load: impl Fn(&Self, &mut Upper, &Id) -> Result<Vec<T>, Error>,
        make: impl Fn(Vec<T>) -> Result<Graph, Error>,
    ) -> Result<Vec<Run>, Error> {
        let put = |r: &mut Self, items: Vec<T>| -> Result<(String, Id), Error> {
            let first = items[0].label().to_string();
            Ok((first, r.put(&Object::Graph(make(items)?))?))
        };
        // An edit falls in the chunk before the next one's first label.
        let falls_in = |label: &str, next: &Option<(String, Id)>| {
            next.as_ref().is_none_or(|n| label < n.0.as_str())
        };
        let mut pending = edits.iter().peekable();
        let mut runs = Vec::new();
        while let Some((label, _)) = pending.peek() {
            let mut cursor = self.cursor_at(upper, root, top, height, label)?;
            let mut cur = cursor.entry();
            let mut next = match self.cursor_next(upper, &mut cursor)? {
                true => Some(cursor.entry()),
                false => None,
            };
            let (mut old, mut new, mut buf, mut acc) = (Vec::new(), Vec::new(), Vec::new(), 0);
            loop {
                let mut mine = Vec::new();
                while let Some(e) = pending.next_if(|(l, _)| falls_in(l, &next)) {
                    mine.push(e);
                }
                let items = merge(load(self, upper, &cur.1)?, &mine);
                old.push(cur);
                for item in items {
                    acc += item.size();
                    let ends = ends_chunk(item.label(), acc);
                    buf.push(item);
                    if ends {
                        new.push(put(self, std::mem::take(&mut buf))?);
                        acc = 0;
                    }
                }
                let Some(n) = next.take() else { break };
                cur = n;
                next = match self.cursor_next(upper, &mut cursor)? {
                    true => Some(cursor.entry()),
                    false => None,
                };
                let more = pending.peek().is_some_and(|(l, _)| falls_in(l, &next));
                if buf.is_empty() && !more {
                    break;
                }
            }
            if !buf.is_empty() {
                new.push(put(self, buf)?);
            }
            runs.push((old, new));
        }
        Ok(runs)
    }
}

/// True if `label` is in `[lo, hi)`; `hi` of `None` is unbounded.
fn in_range(label: &str, lo: &str, hi: Option<&str>) -> bool {
    label >= lo && hi.is_none_or(|h| label < h)
}

impl<S: Store> Repo<S> {
    /// `(first label, id)` of every leaf chunk, in label order, reading only
    /// the parent chunks.
    fn graph_leaf_list(&self, root: &Id) -> Result<Vec<(String, Id)>, Error> {
        let Graph::Children(mut height, mut level) = self.graph_chunk(root)? else {
            return Ok(vec![(String::new(), *root)]);
        };
        while height > 1 {
            let mut next = Vec::new();
            for (_, id) in &level {
                match self.graph_chunk(id)? {
                    Graph::Children(_, children) => next.extend(children),
                    Graph::Nodes(_) => return Err(Error::WrongKind(*id)),
                }
            }
            (level, height) = (next, height - 1);
        }
        Ok(level)
    }

    /// The graph at `root` without the labels in `[lo, hi)` (E240). Equals
    /// `put_graph` of the remaining nodes. Leaves wholly inside the range
    /// are dropped unread; the leaves at its ends, and the ones after it
    /// until the content-defined cut falls back into step, are read and
    /// re-cut; the parent levels are rebuilt from the leaf list.
    pub fn graph_drop_range(&mut self, root: &Id, lo: &str, hi: Option<&str>) -> Result<Id, Error> {
        let leaves = self.graph_leaf_list(root)?;
        let next_first = |i: usize| leaves.get(i + 1).map(|l| l.0.as_str());
        // A leaf holds labels in [its first, the next leaf's first).
        let before = |i: usize| next_first(i).is_some_and(|n| n <= lo);
        let after = |i: usize| hi.is_some_and(|h| leaves[i].0.as_str() >= h);
        let Some(a) = (0..leaves.len()).find(|&i| !before(i)) else {
            return Ok(*root);
        };
        let b = (a..leaves.len())
            .find(|&i| after(i))
            .unwrap_or(leaves.len());
        if a == b {
            return Ok(*root);
        }
        let inside = |i: usize| {
            leaves[i].0.as_str() >= lo && hi.is_none_or(|h| next_first(i).is_some_and(|n| n <= h))
        };
        let mut out: Vec<(String, Id)> = leaves[..a].to_vec();
        let (mut buf, mut acc) = (Vec::new(), 0);
        let mut emit = |r: &mut Self, buf: &mut Vec<Node>| -> Result<(), Error> {
            let chunk = std::mem::take(buf);
            let first = chunk[0].label.clone();
            out.push((first, r.put(&Object::Graph(Graph::Nodes(chunk)))?));
            Ok(())
        };
        let mut i = a;
        while i < leaves.len() {
            if i >= b && buf.is_empty() {
                break;
            }
            if i < b && inside(i) {
                i += 1;
                continue;
            }
            let Graph::Nodes(nodes) = self.graph_chunk(&leaves[i].1)? else {
                return Err(Error::WrongKind(leaves[i].1));
            };
            for n in nodes.into_iter().filter(|n| !in_range(&n.label, lo, hi)) {
                acc += node_size(&n);
                let ends = ends_chunk(&n.label, acc);
                buf.push(n);
                if ends {
                    emit(self, &mut buf)?;
                    acc = 0;
                }
            }
            i += 1;
        }
        if !buf.is_empty() {
            emit(self, &mut buf)?;
        }
        out.extend(leaves[i..].iter().cloned());
        if out.is_empty() {
            return self.put_graph(Vec::new());
        }
        self.put_parents(out, 1)
    }
}
