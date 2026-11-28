//! Graph derivations (E452, an option; nothing in the default paths calls
//! this): computations that take graphs and produce a graph, recorded as
//! facts `output = func(args)` with graph roots as arguments and output.
//!
//! - [`GraphDerivation`] (tag `H`) names its output and its graph arguments
//!   ([`Arg::Graph`]) as objects. It is kept outside the content ledger, so
//!   a graph root is never taken for content (`has_content` stays false),
//!   and gc keeps a record while its output graph is live, with the whole
//!   of every graph it names (`reachable_from`) and the content it names
//!   (the function's text, content arguments).
//! - The memo key is `Derivation::key(func, args)`: for a computation over
//!   one graph, (spec, input root), with no history. A rerun with an
//!   unchanged input root is a hit; an incremental update from an earlier
//!   version is a plan for reaching the same output, so the record is the
//!   same whichever way the output was reached.
//! - [`GraphRead`] is the read interface a graph function gets: a node by
//!   label, a key's label and a node's in-edges (through the index), and a
//!   label range, read lazily over the fragments. [`GraphFn`] is a native
//!   function over it; E450's computations run through it on a memo miss
//!   with no earlier version ([`Repo::graph_derive`]).
//! - fsck checks a record's graphs are stored; `fsck_deep` recomputes a
//!   built-in computation in full in a scratch store and compares roots.
//!   A function the kernel does not know is checked shallowly only.
//! - WASM graph functions (`wasm_read.rs`) read through the same
//!   interface as read-only host imports, and are rechecked by `fsck_deep`.
//! - [`LinkIndex`]: the store's object links as a graph with E450's index
//!   over it (key to label and in-edges), kept up to date from the store's
//!   new and deleted ids, so `used_by` becomes an index lookup.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::compute::{ChunkCache, Compute, Cost, Entry, GraphDelta, Key, Layout, MemGraph};
use crate::dict::{is_plain, object_tag};
use crate::{
    Arg, Derivation, Edge, Error, GraphDerivation, Id, MemStore, Node, Object, Repo, Store, Target,
    func, node_key,
};

/// Graph derivations by memo key and by output.
#[derive(Default)]
pub(crate) struct GraphLedger {
    by_key: HashMap<Id, (Id, Id)>,
    by_output: HashMap<Id, Vec<(Id, GraphDerivation)>>,
}

impl GraphLedger {
    fn insert(&mut self, record: Id, d: GraphDerivation) {
        self.by_key
            .insert(Derivation::key(&d.func, &d.args), (record, d.output));
        let v = self.by_output.entry(d.output).or_default();
        if !v.iter().any(|(r, _)| *r == record) {
            v.push((record, d));
        }
    }
}

/// What a derivation call found or did.
#[derive(Clone, Debug, PartialEq)]
pub struct Derived {
    pub output: Id,
    /// The record's id.
    pub record: Id,
    /// True when the memo already held `(func, args)`.
    pub hit: bool,
    pub cost: Cost,
}

/// A graph read lazily over its fragments: what a graph function gets.
pub trait GraphRead {
    /// The node labelled `label`, if present.
    fn node(&mut self, label: &str) -> Result<Option<Node>, Error>;
    /// The label of the present node whose key is `key` (through the index).
    fn label_of(&mut self, key: Key) -> Result<Option<String>, Error>;
    /// Edges into `label`: `(source label, edge label)` (through the index).
    fn in_edges(&mut self, label: &str) -> Result<Vec<(String, String)>, Error>;
    /// Labels of the present nodes `label` has an edge to.
    fn out_neighbours(&mut self, label: &str) -> Result<Vec<String>, Error>;
    /// The nodes with `lo <= label < hi` (no `hi`: to the end), in order.
    fn range(&mut self, lo: &str, hi: Option<&str>) -> Result<Vec<Node>, Error>;
    /// Graph and index chunks read so far.
    fn chunks_read(&self) -> usize;
}

/// [`GraphRead`] over a stored graph and (for key lookups) its index, each
/// chunk read at most once.
pub struct FragRead<'a, S: Store> {
    repo: &'a Repo<S>,
    graph: Id,
    index: Option<Id>,
    cache: ChunkCache,
    range_reads: usize,
}

impl<'a, S: Store> FragRead<'a, S> {
    pub fn new(repo: &'a Repo<S>, graph: Id, index: Option<Id>) -> Self {
        FragRead {
            repo,
            graph,
            index,
            cache: ChunkCache::default(),
            range_reads: 0,
        }
    }

    fn entry(&mut self, key: Key) -> Result<Option<Entry>, Error> {
        let index = self
            .index
            .ok_or(Error::Exec("graph read: no index".into()))?;
        let mut found = self.repo.values_in(&index, &[key], &mut self.cache)?;
        Ok(found.remove(&key))
    }
}

impl<S: Store> GraphRead for FragRead<'_, S> {
    fn node(&mut self, label: &str) -> Result<Option<Node>, Error> {
        let mut out = Vec::new();
        let labels = BTreeSet::from([label.to_string()]);
        self.repo
            .nodes_in_cached(&self.graph, &labels, &mut self.cache, &mut out)?;
        Ok(out.pop())
    }
    fn label_of(&mut self, key: Key) -> Result<Option<String>, Error> {
        Ok(self.entry(key)?.and_then(|e| e.label))
    }
    fn in_edges(&mut self, label: &str) -> Result<Vec<(String, String)>, Error> {
        let Some(e) = self.entry(super::compute::key_of(label))? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for (k, l) in e.inn {
            out.extend(self.label_of(k)?.map(|s| (s, l)));
        }
        Ok(out)
    }
    fn out_neighbours(&mut self, label: &str) -> Result<Vec<String>, Error> {
        let Some(n) = self.node(label)? else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for e in n.edges {
            out.extend(self.label_of(u64::from_be_bytes(e.to))?);
        }
        Ok(out)
    }
    fn range(&mut self, lo: &str, hi: Option<&str>) -> Result<Vec<Node>, Error> {
        if lo.is_empty() && hi.is_none() {
            return self
                .repo
                .graph_nodes_counted(&self.graph, &mut self.range_reads);
        }
        self.range_reads += 1;
        self.repo
            .graph_range(&self.graph, lo, hi, usize::MAX, false)
    }
    fn chunks_read(&self) -> usize {
        self.cache.reads + self.range_reads
    }
}

/// A native graph function: a graph in (through [`GraphRead`]), a graph's
/// nodes out.
pub trait GraphFn {
    /// The name its derivation's function text records.
    fn name(&self) -> String;
    fn run(&self, g: &mut dyn GraphRead) -> Result<Vec<Node>, Error>;
}

/// E450's computations as a [`GraphFn`], recomputed in full from a range
/// read of the whole graph.
pub struct Builtin {
    pub compute: Compute,
    pub layout: Layout,
}

impl GraphFn for Builtin {
    fn name(&self) -> String {
        self.layout.func_text(&self.compute.spec())
    }
    fn run(&self, g: &mut dyn GraphRead) -> Result<Vec<Node>, Error> {
        let nodes = g.range("", None)?;
        let mem = MemGraph::from_nodes(&nodes);
        super::compute::value_nodes(mem.compute(&self.compute), self.layout, &mem)
    }
}

impl<S: Store> Repo<S> {
    fn with_graph_ledger<R>(&self, f: impl FnOnce(&mut GraphLedger) -> R) -> Result<R, Error> {
        let mut slot = self.graph_ledger.lock().unwrap();
        if slot.is_none() {
            let mut l = GraphLedger::default();
            for id in self.store.ids()? {
                if object_tag(&self.store.read_head(&id, 2)?) == Some(b'H')
                    && let Object::GraphDerivation(d) = self.get(&id)?
                {
                    l.insert(id, d);
                }
            }
            *slot = Some(l);
        }
        Ok(f(slot.as_mut().unwrap()))
    }

    /// Record `output = func(args)` over graphs. Its graphs (output and
    /// graph arguments) must be stored; nothing is evaluated here, so the
    /// caller vouches for it, and `fsck_deep` checks built-in ones.
    pub fn record_graph_derivation(&mut self, d: GraphDerivation) -> Result<Id, Error> {
        let graphs = (d.args.iter().filter(|a| a.is_graph()).map(Arg::id)).chain([d.output]);
        for g in graphs {
            if !self.store.has(&g) {
                return Err(Error::NotFound(g));
            }
        }
        let encoded = Object::GraphDerivation(d.clone()).encode();
        let id = Id::of(&encoded);
        self.write(id, &encoded)?;
        self.with_graph_ledger(|l| l.insert(id, d))?;
        Ok(id)
    }

    /// The memo: the record and output of `func(args)`, if recorded.
    pub fn graph_memo(&self, func: &Id, args: &[Arg]) -> Result<Option<(Id, Id)>, Error> {
        let key = Derivation::key(func, args);
        self.with_graph_ledger(|l| l.by_key.get(&key).copied())
    }

    /// Every graph derivation recorded for the graph `output`.
    pub fn graph_derivations(&self, output: &Id) -> Result<Vec<(Id, GraphDerivation)>, Error> {
        self.with_graph_ledger(|l| l.by_output.get(output).cloned().unwrap_or_default())
    }

    fn compute_func(&mut self, c: &Compute) -> Result<(Id, Layout), Error> {
        let layout = match c {
            Compute::Index => Layout::Hash,
            _ => self.graph_result_layout,
        };
        let text = layout.func_text(&c.spec());
        Ok((self.put_content(text.as_bytes())?.id, layout))
    }

    fn hit(record: Id, output: Id, spec: String, at: std::time::Instant) -> Derived {
        Derived {
            output,
            record,
            hit: true,
            cost: Cost {
                spec,
                seconds: at.elapsed().as_secs_f64(),
                ..Cost::default()
            },
        }
    }

    /// `c` over the graph at `input`, as a recorded graph derivation: a memo
    /// hit when `(c, input)` was derived before, otherwise a full
    /// recompute through [`GraphRead`] ([`Builtin`]), recorded.
    pub fn graph_derive(&mut self, c: &Compute, input: &Id) -> Result<Derived, Error> {
        let at = std::time::Instant::now();
        let (func, layout) = self.compute_func(c)?;
        let args = vec![Arg::Graph(*input)];
        if let Some((record, output)) = self.graph_memo(&func, &args)? {
            return Ok(Self::hit(record, output, c.spec(), at));
        }
        let f = Builtin {
            compute: c.clone(),
            layout,
        };
        let (nodes, reads) = {
            let mut g = FragRead::new(self, *input, None);
            (f.run(&mut g)?, g.chunks_read())
        };
        let output = self.put_graph(nodes)?;
        let record = self.record_graph_derivation(GraphDerivation { output, func, args })?;
        Ok(Derived {
            output,
            record,
            hit: false,
            cost: Cost {
                spec: c.spec(),
                chunks_read: reads,
                changed: 0,
                seconds: at.elapsed().as_secs_f64(),
            },
        })
    }

    /// `c` over `input`, reached from what was derived for an earlier
    /// version `prev` and their delta (computed when not given): a memo hit
    /// when `(c, input)` is recorded; an incremental update when
    /// `(c, prev)` is (the index first, the same way); otherwise in full.
    /// The record is `(c, input) -> output` whichever way it was reached.
    pub fn graph_derive_from(
        &mut self,
        c: &Compute,
        input: &Id,
        prev: &Id,
        delta: Option<&GraphDelta>,
    ) -> Result<Derived, Error> {
        let at = std::time::Instant::now();
        let (func, _) = self.compute_func(c)?;
        let args = vec![Arg::Graph(*input)];
        if let Some((record, output)) = self.graph_memo(&func, &args)? {
            return Ok(Self::hit(record, output, c.spec(), at));
        }
        let Some((_, old)) = self.graph_memo(&func, &[Arg::Graph(*prev)])? else {
            return self.graph_derive(c, input);
        };
        let owned;
        let delta = match delta {
            Some(d) => d,
            None => {
                owned = self.graph_delta(prev, input)?;
                &owned
            }
        };
        let (output, cost) = match c {
            Compute::Index => self.graph_update_result(c, old, input, &old, &old, delta)?,
            _ => {
                let index = self
                    .graph_derive_from(&Compute::Index, input, prev, Some(delta))?
                    .output;
                let prev_index = self.graph_derive(&Compute::Index, prev)?.output;
                self.graph_update_result(c, old, input, &index, &prev_index, delta)?
            }
        };
        let record = self.record_graph_derivation(GraphDerivation { output, func, args })?;
        Ok(Derived {
            output,
            record,
            hit: false,
            cost,
        })
    }

    /// A native graph function over `input`, as a recorded graph
    /// derivation (its function text is `ikam/graph-fn/1 <name>`): a memo
    /// hit for an input it has seen, otherwise run through [`FragRead`]
    /// with the input's index (derived, so a hit too when known).
    pub fn graph_derive_fn(&mut self, f: &dyn GraphFn, input: &Id) -> Result<Derived, Error> {
        let at = std::time::Instant::now();
        let text = format!("ikam/graph-fn/1 {}", f.name());
        let func = self.put_content(text.as_bytes())?.id;
        let args = vec![Arg::Graph(*input)];
        if let Some((record, output)) = self.graph_memo(&func, &args)? {
            return Ok(Self::hit(record, output, f.name(), at));
        }
        let index = self.graph_derive(&Compute::Index, input)?.output;
        let (nodes, reads) = {
            let mut g = FragRead::new(self, *input, Some(index));
            (f.run(&mut g)?, g.chunks_read())
        };
        let output = self.put_graph(nodes)?;
        let record = self.record_graph_derivation(GraphDerivation { output, func, args })?;
        Ok(Derived {
            output,
            record,
            hit: false,
            cost: Cost {
                spec: f.name(),
                chunks_read: reads,
                changed: 0,
                seconds: at.elapsed().as_secs_f64(),
            },
        })
    }

    /// gc (E452): graph derivations whose output is among `objects` are
    /// kept, with every graph they name (whole) and every content they name
    /// (through derivations), to a fixed point. True if any was kept.
    pub(crate) fn add_graph_derivations(
        &self,
        objects: &mut HashSet<Id>,
        content: &mut HashSet<Id>,
        records: &mut HashSet<Id>,
    ) -> Result<bool, Error> {
        let mut all = Vec::new();
        for id in self.store.ids()? {
            if object_tag(&self.store.read_head(&id, 2)?) == Some(b'H')
                && let Object::GraphDerivation(d) = self.get(&id)?
            {
                all.push((id, d));
            }
        }
        let mut kept = false;
        loop {
            let (mut graphs, mut contents) = (Vec::new(), Vec::new());
            for (id, d) in &all {
                if objects.contains(&d.output) && records.insert(*id) {
                    kept = true;
                    for a in &d.args {
                        match a {
                            Arg::Graph(g) => graphs.push(*g),
                            a => contents.push(a.id()),
                        }
                    }
                    contents.extend(Some(d.func).filter(|f| !func::is_builtin(f)));
                }
            }
            if graphs.is_empty() && contents.is_empty() {
                return Ok(kept);
            }
            let (objs, files) = self.reachable_from(graphs, objects)?;
            objects.extend(objs);
            contents.extend(files);
            let (c, r) = self.closure(contents, |_| true)?;
            content.extend(c);
            records.extend(r);
        }
    }

    /// fsck (E452): a graph derivation's graphs and function are here; with
    /// `deep`, a built-in computation is recomputed in full in a scratch
    /// store and its root compared.
    pub(crate) fn check_graph_derivation(
        &self,
        d: &GraphDerivation,
        deep: bool,
    ) -> Result<(), Error> {
        for a in d.args.iter().chain([&Arg::Graph(d.output)]) {
            let ok = match a {
                Arg::Graph(g) => self.store.has(g),
                a => self.has_content(&a.id())?,
            };
            if !ok {
                return Err(Error::NotFound(a.id()));
            }
        }
        if !func::is_builtin(&d.func) && !self.has_content(&d.func)? {
            return Err(Error::NotFound(d.func));
        }
        if !deep {
            return Ok(());
        }
        if let Some(ok) = self.check_wasm_graph_derivation(d)? {
            return match ok {
                true => Ok(()),
                false => Err(Error::Corrupt(d.output)),
            };
        }
        let text = String::from_utf8(self.read_content(&d.func)?).unwrap_or_default();
        let (Some((layout, c)), [Arg::Graph(input)]) = (Layout::parse_func(&text), &d.args[..])
        else {
            // Not a computation the kernel knows: checked shallowly.
            return Ok(());
        };
        let mut scratch = Repo::new(MemStore::default());
        scratch.plain_hot_writes = true;
        scratch.graph_result_layout = layout;
        let copy = scratch.put_graph(self.graph_nodes(input)?)?;
        if copy != *input {
            return Err(Error::Corrupt(*input));
        }
        let (out, _) = scratch.graph_compute_one(&copy, &c)?;
        match out == d.output {
            true => Ok(()),
            false => Err(Error::Corrupt(d.output)),
        }
    }
}

/// E452: the reverse links of a store's objects, kept as a graph (one node
/// per object, labelled with its id in hex, an edge per link to the linked
/// id's node, labelled as `Object::links` labels it) and E450's index over
/// it, so the objects that link to an id are an index lookup instead of a
/// scan of the store. It is kept in a store of its own (a projection) and
/// brought up to date from the ids the source store gained or lost.
#[derive(Clone, Debug)]
pub struct LinkIndex {
    pub graph: Id,
    pub index: Id,
    /// Every id the source store held at the last update.
    pub seen: HashSet<Id>,
}

/// What one link index update did.
#[derive(Clone, Debug, Default)]
pub struct LinkUpdate {
    pub added: usize,
    pub removed: usize,
    pub objects_read: usize,
    pub index_chunks_read: usize,
}

fn link_node(id: &Id, obj: &Object) -> Node {
    let mut edges: Vec<Edge> = (obj.links().into_iter())
        .map(|(label, to)| Edge {
            to: node_key(&to.to_string()),
            label,
        })
        .collect();
    edges.sort();
    edges.dedup();
    Node {
        label: id.to_string(),
        target: Target::None,
        edges,
    }
}

impl<S: Store> Repo<S> {
    /// Build (no `prev`) or bring up to date the link index of `src`'s
    /// objects, stored in this repo. Plain content has no links and is
    /// only noted as seen.
    pub fn link_index_update<T: Store>(
        &mut self,
        src: &Repo<T>,
        prev: Option<&LinkIndex>,
    ) -> Result<(LinkIndex, LinkUpdate), Error> {
        let ids: HashSet<Id> = src.store().ids()?.into_iter().collect();
        let mut u = LinkUpdate::default();
        let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
        for id in &ids {
            if prev.is_some_and(|p| p.seen.contains(id)) {
                continue;
            }
            let stored = src.store().read(id)?;
            if is_plain(&stored) {
                continue;
            }
            u.objects_read += 1;
            let n = link_node(id, &src.get(id)?);
            nodes.insert(n.label.clone(), n);
        }
        let Some(prev) = prev else {
            u.added = nodes.len();
            let graph = self.put_graph(nodes.into_values().collect())?;
            let (done, _) = self.graph_compute(&graph, &[])?;
            return Ok((
                LinkIndex {
                    graph,
                    index: done.index,
                    seen: ids,
                },
                u,
            ));
        };
        let gone: BTreeSet<String> = (prev.seen.iter())
            .filter(|id| !ids.contains(id))
            .map(|id| id.to_string())
            .collect();
        let before = self.graph_nodes_in(&prev.graph, &gone)?;
        let mut delta = GraphDelta::default();
        let mut edits = BTreeMap::new();
        for n in before {
            edits.insert(n.label.clone(), None);
            delta.nodes.push((n.label.clone(), Some(n), None));
        }
        for (l, n) in nodes {
            edits.insert(l.clone(), Some(n.clone()));
            delta.nodes.push((l, None, Some(n)));
        }
        u.removed = gone.len();
        u.added = edits.len() - u.removed;
        let graph = self.graph_edit(&prev.graph, edits)?;
        let (index, cost) = self.graph_update_result(
            &Compute::Index,
            prev.index,
            &graph,
            &prev.index,
            &prev.index,
            &delta,
        )?;
        u.index_chunks_read = cost.chunks_read;
        Ok((
            LinkIndex {
                graph,
                index,
                seen: ids,
            },
            u,
        ))
    }

    /// `used_by` through a link index: every object of the source store
    /// that links to `id`, with the link's label, sorted.
    pub fn used_by_indexed(&self, ix: &LinkIndex, id: &Id) -> Result<Vec<(Id, String)>, Error> {
        let mut r = FragRead::new(self, ix.graph, Some(ix.index));
        let mut out = Vec::new();
        for (src, label) in r.in_edges(&id.to_string())? {
            out.push((src.parse::<Id>()?, label));
        }
        out.sort();
        Ok(out)
    }
}

/// E454: the link index in segments by commit order, an option beside
/// [`LinkIndex`]. Each update writes the in-links of the objects it found
/// new as a segment of its own: a graph with one node per linked id
/// (labelled with the id in hex) holding an edge per link into it (the
/// edge's key is the source's, its label the source's id in hex, a space
/// and the link's label). New objects are in a segment of their own, so
/// they no longer scatter over a hash-ordered index: an update writes one
/// small graph whole. Segments merge when the newer is at least half the
/// older's size (so there are about log2 of them), and a lookup reads the
/// id's node in each. Objects the source store lost are kept as a graph of
/// tombstones, which a lookup and a merge drop; an object that comes back
/// leaves it.
#[derive(Clone, Debug)]
pub struct LinkSegments {
    /// `(root, objects indexed in it)`, oldest first.
    pub segments: Vec<(Id, usize)>,
    /// The ids the source store lost (a graph of empty nodes by id hex).
    pub gone: Id,
    /// Every id the source store held at the last update.
    pub seen: HashSet<Id>,
}

/// What one segmented link index update did.
#[derive(Clone, Debug, Default)]
pub struct SegmentUpdate {
    pub added: usize,
    pub removed: usize,
    pub objects_read: usize,
    /// Chunks of earlier segments read to merge them.
    pub chunks_read: usize,
    /// Segments merged into one.
    pub merged: usize,
}

fn segment_nodes(links: BTreeMap<Id, Vec<(Id, String)>>) -> Vec<Node> {
    (links.into_iter())
        .map(|(to, from)| {
            let mut edges: Vec<Edge> = (from.into_iter())
                .map(|(src, label)| {
                    let hex = src.to_string();
                    Edge {
                        to: node_key(&hex),
                        label: format!("{hex} {label}"),
                    }
                })
                .collect();
            edges.sort();
            edges.dedup();
            Node {
                label: to.to_string(),
                target: Target::None,
                edges,
            }
        })
        .collect()
}

fn segment_link(e: &Edge) -> Result<(Id, String), Error> {
    let (src, label) = e
        .label
        .split_once(' ')
        .ok_or(Error::Decode("link segment"))?;
    Ok((src.parse::<Id>()?, label.to_string()))
}

impl<S: Store> Repo<S> {
    /// Build (no `prev`) or bring up to date the segmented link index of
    /// `src`'s objects, stored in this repo.
    pub fn link_segments_update<T: Store>(
        &mut self,
        src: &Repo<T>,
        prev: Option<&LinkSegments>,
    ) -> Result<(LinkSegments, SegmentUpdate), Error> {
        let ids: HashSet<Id> = src.store().ids()?.into_iter().collect();
        let mut u = SegmentUpdate::default();
        let mut links: BTreeMap<Id, Vec<(Id, String)>> = BTreeMap::new();
        let mut back = BTreeSet::new();
        for id in &ids {
            if prev.is_some_and(|p| p.seen.contains(id)) {
                continue;
            }
            back.insert(id.to_string());
            let stored = src.store().read(id)?;
            if is_plain(&stored) {
                continue;
            }
            u.objects_read += 1;
            u.added += 1;
            for (label, to) in src.get(id)?.links() {
                links.entry(to).or_default().push((*id, label));
            }
        }
        let objects = u.added;
        let segment = self.put_graph(segment_nodes(links))?;
        let Some(prev) = prev else {
            let gone = self.put_graph(Vec::new())?;
            let ix = LinkSegments {
                segments: vec![(segment, objects)],
                gone,
                seen: ids,
            };
            return Ok((ix, u));
        };
        // Tombstones: the ids lost since the last update join, the ids back
        // again leave.
        let mut edits: BTreeMap<String, Option<Node>> = BTreeMap::new();
        for id in prev.seen.iter().filter(|id| !ids.contains(id)) {
            u.removed += 1;
            let label = id.to_string();
            let n = Node {
                label: label.clone(),
                target: Target::None,
                edges: Vec::new(),
            };
            edits.insert(label, Some(n));
        }
        let was_gone = self.graph_nodes_in(&prev.gone, &back)?;
        for n in was_gone {
            edits.insert(n.label, None);
        }
        let gone = self.graph_edit(&prev.gone, edits)?;
        let mut segments = prev.segments.clone();
        if objects > 0 {
            segments.push((segment, objects));
        }
        // Merge while the newest is at least half the one before.
        while segments.len() >= 2 {
            let (b, nb) = segments[segments.len() - 1];
            let (a, na) = segments[segments.len() - 2];
            if nb * 2 < na {
                break;
            }
            let mut reads = 0;
            let mut merged: BTreeMap<String, Node> = BTreeMap::new();
            for root in [a, b] {
                for n in self.graph_nodes_counted(&root, &mut reads)? {
                    match merged.get_mut(&n.label) {
                        Some(m) => m.edges.extend(n.edges),
                        None => {
                            merged.insert(n.label.clone(), n);
                        }
                    }
                }
            }
            u.chunks_read += reads;
            // Drop the links of sources gone.
            let sources: BTreeSet<String> = (merged.values())
                .flat_map(|n| n.edges.iter())
                .filter_map(|e| e.label.split_once(' ').map(|s| s.0.to_string()))
                .collect();
            let dead: HashSet<String> = (self.graph_nodes_in(&gone, &sources)?.into_iter())
                .map(|n| n.label)
                .collect();
            let nodes: Vec<Node> = (merged.into_values())
                .filter_map(|mut n| {
                    n.edges
                        .retain(|e| e.label.split_once(' ').is_none_or(|s| !dead.contains(s.0)));
                    n.edges.sort();
                    n.edges.dedup();
                    (!n.edges.is_empty()).then_some(n)
                })
                .collect();
            let root = self.put_graph(nodes)?;
            segments.truncate(segments.len() - 2);
            segments.push((root, na + nb));
            u.merged += 1;
        }
        Ok((
            LinkSegments {
                segments,
                gone,
                seen: ids,
            },
            u,
        ))
    }

    /// `used_by` through a segmented link index: every object of the
    /// source store that links to `id`, with the link's label, sorted.
    pub fn used_by_segments(&self, ix: &LinkSegments, id: &Id) -> Result<Vec<(Id, String)>, Error> {
        let want = BTreeSet::from([id.to_string()]);
        let mut out = Vec::new();
        for (root, _) in &ix.segments {
            for n in self.graph_nodes_in(root, &want)? {
                for e in &n.edges {
                    out.push(segment_link(e)?);
                }
            }
        }
        if !out.is_empty() {
            let sources: BTreeSet<String> = out.iter().map(|(s, _)| s.to_string()).collect();
            let dead: HashSet<String> = (self.graph_nodes_in(&ix.gone, &sources)?.into_iter())
                .map(|n| n.label)
                .collect();
            out.retain(|(s, _)| !dead.contains(&s.to_string()));
        }
        out.sort();
        out.dedup();
        Ok(out)
    }
}
