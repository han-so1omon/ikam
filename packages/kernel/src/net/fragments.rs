//! Nets as connected fragments (E446, opt-in; nothing in the default paths
//! calls this).
//!
//! E422 stores a version of a net as one canonical text file, so every
//! version is a new content however small the edit. Here a version is a
//! graph of content-addressed fragments instead:
//!
//! - the head fragment is the net's `net NAME VERSION` line, each place
//!   fragment its `place` line and each transition fragment its
//!   `transition` line, arcs and weights included (the lines of
//!   [`Net::to_text`]), each stored once by `put_content`;
//! - the root is a graph (`graph.rs`) with one node per fragment, labelled
//!   `net`, `p/<place>` and `t/<transition>`, whose target is the fragment.
//!   Order is kept by edges, never by position: the head node's edges
//!   `place` and `transition` name the first of each, and each element's
//!   edge `next` names the one after it (an edge's label carries its
//!   target's label, so a node's links read without any other node). An insert or a removal changes the
//!   nodes either side of it, not every node after it, and the same net
//!   always has the same nodes, whatever its history.
//!
//! Two layouts of the same nodes ([`Layout`]): `Flat`, one chunk holding
//! every node, rewritten whole by any edit; and `Tree`, the prolly tree
//! `put_graph` cuts, where an edit rewrites the chunks it falls in and the
//! path above them. A net small enough for one chunk has the same root in
//! both. The root's id is the version's id: firings may name it as they
//! name E422's text id ([`Repo::net_track_fragments`]), and
//! [`Repo::net_version`] reads either.
//!
//! [`Repo::net_delta`] is the difference between two versions as a view
//! over the traversal: both roots are walked a level at a time, a chunk id
//! both share is skipped unread (E249's walk), and only the nodes in the
//! chunks left are compared and their fragments read. Nothing is stored.

mod inline;

use std::collections::{BTreeMap, HashMap};

use super::text::{head_line, parse_place, parse_transition, place_line, transition_line};
use super::{Net, Place, Transition};
use crate::{Arg, Edge, Error, Graph, Id, Node, Object, Repo, Store, Target, node_key};

pub use inline::InlineCodec;
pub(super) use inline::{head_payload, place_payload, read_inline, transition_payload};

/// How a fragment version's nodes are kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// One chunk with every node: an edit rewrites it whole.
    Flat,
    /// The prolly tree `put_graph` cuts: an edit rewrites the chunks it
    /// falls in and their parents.
    Tree,
    /// `Tree`'s nodes with each element's bytes inline and no edges
    /// (E447, [`InlineCodec::Compact`]): no element is an object.
    Inline,
    /// `Inline`'s nodes in one chunk, as `Flat` keeps E446's (E447): an
    /// edit rewrites the chunk, which the planner stores as slices of the
    /// one before, as it stores a text file's versions.
    InlineFlat,
}

impl Layout {
    pub(super) fn name(self) -> &'static str {
        match self {
            Layout::Flat => "flat",
            Layout::Tree => "tree",
            Layout::Inline => "inline",
            Layout::InlineFlat => "inline-flat",
        }
    }

    pub(super) fn parse(s: &str) -> Option<Layout> {
        match s {
            "flat" => Some(Layout::Flat),
            "tree" => Some(Layout::Tree),
            "inline" => Some(Layout::Inline),
            "inline-flat" => Some(Layout::InlineFlat),
            _ => None,
        }
    }
}

pub(super) const HEAD: &str = "net";
pub(super) const FIRST_PLACE: &str = "place";
pub(super) const FIRST_TRANSITION: &str = "transition";
pub(super) const NEXT: &str = "next";

pub(super) fn place_label(name: &str) -> String {
    format!("p/{name}")
}

pub(super) fn transition_label(name: &str) -> String {
    format!("t/{name}")
}

pub(super) fn bad() -> Error {
    Error::Decode("net fragments")
}

/// A node of the fragment form: its label, the fragment `text` it
/// targets, and `edges` (sorted and deduplicated here).
pub(super) fn fragment_node(label: String, text: String, mut edges: Vec<Edge>) -> (Node, String) {
    edges.sort();
    edges.dedup();
    let target = Target::Content(Arg::Whole(Id::of_content(text.as_bytes())));
    (
        Node {
            label,
            target,
            edges,
        },
        text,
    )
}

/// An order edge: its label names its target's label too, so a node's
/// links read without looking up any other node.
pub(super) fn order_edge(label: &str, to: &str) -> Edge {
    Edge {
        to: node_key(to),
        label: format!("{label} {to}"),
    }
}

/// The head node of `net`'s fragment form.
pub(super) fn head_node(net: &Net) -> (Node, String) {
    let mut head = Vec::new();
    if let Some(p) = net.places.first() {
        head.push(order_edge(FIRST_PLACE, &place_label(&p.name)));
    }
    if let Some(t) = net.transitions.first() {
        head.push(order_edge(FIRST_TRANSITION, &transition_label(&t.name)));
    }
    fragment_node(HEAD.into(), head_line(net), head)
}

/// The node of place `p`, followed by `next`.
pub(super) fn place_node(p: &Place, next: Option<&Place>) -> (Node, String) {
    let edges = next.map(|n| order_edge(NEXT, &place_label(&n.name)));
    fragment_node(
        place_label(&p.name),
        place_line(p),
        edges.into_iter().collect(),
    )
}

/// The node of transition `t`, followed by `next`.
pub(super) fn transition_node(t: &Transition, next: Option<&Transition>) -> (Node, String) {
    let edges = next.map(|n| order_edge(NEXT, &transition_label(&n.name)));
    fragment_node(
        transition_label(&t.name),
        transition_line(t),
        edges.into_iter().collect(),
    )
}

/// The nodes of `net`'s fragment form, each with its fragment's text, in
/// no particular order.
/// Encoded bytes of `net`'s nodes in `layout`, as chunk boundaries count
/// them: the size [`NetChunkPolicy`](super::NetChunkPolicy) picks by (E453).
pub(super) fn node_bytes(net: &Net, layout: Layout) -> usize {
    match layout {
        Layout::Inline | Layout::InlineFlat => {
            crate::graph::nodes_size(&inline::inline_nodes(net, InlineCodec::Compact))
        }
        Layout::Tree | Layout::Flat => {
            let nodes: Vec<Node> = fragment_nodes(net).into_iter().map(|n| n.0).collect();
            crate::graph::nodes_size(&nodes)
        }
    }
}

fn fragment_nodes(net: &Net) -> Vec<(Node, String)> {
    let mut out = vec![head_node(net)];
    for (i, p) in net.places.iter().enumerate() {
        out.push(place_node(p, net.places.get(i + 1)));
    }
    for (i, t) in net.transitions.iter().enumerate() {
        out.push(transition_node(t, net.transitions.get(i + 1)));
    }
    out
}

/// What one fragment declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Element {
    Head(String, u32),
    Place(Place),
    Transition(Transition),
}

pub(super) fn parse_fragment(label: &str, text: &str) -> Result<Element, Error> {
    let line = text.strip_suffix('\n').ok_or_else(bad)?;
    if line.contains('\n') {
        return Err(bad());
    }
    let (word, rest) = line.split_once(' ').ok_or_else(bad)?;
    let el = match word {
        "net" if label == HEAD => {
            let n = Net::parse(text)?;
            Element::Head(n.name, n.version)
        }
        "place" => Element::Place(parse_place(0, rest)?),
        "transition" => Element::Transition(parse_transition(0, rest)?),
        _ => return Err(bad()),
    };
    let name_ok = match &el {
        Element::Head(..) => true,
        Element::Place(p) => label == place_label(&p.name),
        Element::Transition(t) => label == transition_label(&t.name),
    };
    if !name_ok {
        return Err(bad());
    }
    Ok(el)
}

/// A node's element and the labels its order edges name.
pub(super) struct Read {
    pub(super) element: Element,
    /// Edge label to target label (`next`, or the head's `place` and
    /// `transition`).
    pub(super) links: BTreeMap<String, String>,
}

/// One element added, removed or changed between two versions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change<T> {
    Added(T),
    Removed(T),
    Changed { before: T, after: T },
}

/// An arc of a transition in both versions whose weight changed, or an arc
/// one of them lacks (`None`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArcChange {
    pub transition: String,
    /// An output arc (else an input).
    pub output: bool,
    pub place: String,
    pub before: Option<u32>,
    pub after: Option<u32>,
}

/// One order link of the later version, for an element the delta touches:
/// in list `transitions` (else places), the element after `from` (None:
/// the first) is `to` (None: there is none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub transitions: bool,
    pub from: Option<String>,
    pub to: Option<String>,
}

/// The difference between two fragment versions of a net
/// ([`Repo::net_delta`]): computed by walking both, never stored.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetDelta {
    /// The net's name and version, before and after, when they differ.
    pub head: Option<((String, u32), (String, u32))>,
    pub places: Vec<Change<Place>>,
    pub transitions: Vec<Change<Transition>>,
    /// Arcs of the transitions in `Changed` whose weight was added,
    /// removed or changed (an added or removed transition's arcs come with
    /// it).
    pub arcs: Vec<ArcChange>,
    /// Elements whose fragment is unchanged but whose neighbour is not.
    pub relinked: Vec<String>,
    /// The order links of the later version on every node that differs.
    pub links: Vec<Link>,
    /// Graph chunks read, the shared subtrees skipped.
    pub chunks_read: usize,
    /// Fragments read: those of the nodes that differ.
    pub fragments_read: usize,
}

impl NetDelta {
    /// True when the two versions are the same net.
    pub fn is_empty(&self) -> bool {
        self.head.is_none()
            && self.places.is_empty()
            && self.transitions.is_empty()
            && self.links.is_empty()
    }

    /// The later version, from the earlier one `a` and this delta. It
    /// equals the later version exactly, order included, when `a` is the
    /// earlier version the delta was computed from.
    pub fn apply(&self, a: &Net) -> Result<Net, Error> {
        let mut out = a.clone();
        if let Some((_, (name, version))) = &self.head {
            out.name = name.clone();
            out.version = *version;
        }
        let mut place_changes = BTreeMap::new();
        for c in &self.places {
            match c {
                Change::Removed(p) => place_changes.insert(p.name.clone(), None),
                Change::Added(p) | Change::Changed { after: p, .. } => {
                    place_changes.insert(p.name.clone(), Some(p.clone()))
                }
            };
        }
        let mut transition_changes = BTreeMap::new();
        for c in &self.transitions {
            match c {
                Change::Removed(t) => transition_changes.insert(t.name.clone(), None),
                Change::Added(t) | Change::Changed { after: t, .. } => {
                    transition_changes.insert(t.name.clone(), Some(t.clone()))
                }
            };
        }
        let links = |transitions: bool| {
            self.links
                .iter()
                .filter(move |l| l.transitions == transitions)
        };
        out.places = relist(&a.places, |p| &p.name, place_changes, links(false))?;
        out.transitions = relist(&a.transitions, |t| &t.name, transition_changes, links(true))?;
        out.check()?;
        Ok(out)
    }
}

/// One list of a net after a delta: its elements changed as `changes` says
/// and reordered by following the later version's links.
fn relist<'a, T: Clone>(
    list: &[T],
    name: impl Fn(&T) -> &String,
    changes: BTreeMap<String, Option<T>>,
    links: impl Iterator<Item = &'a Link>,
) -> Result<Vec<T>, Error> {
    let mut by_name: HashMap<String, T> =
        list.iter().map(|x| (name(x).clone(), x.clone())).collect();
    let mut next: HashMap<Option<String>, Option<String>> = HashMap::new();
    let mut prev = None;
    for x in list {
        next.insert(prev, Some(name(x).clone()));
        prev = Some(name(x).clone());
    }
    next.insert(prev, None);
    for (n, c) in changes {
        match c {
            Some(x) => {
                by_name.insert(n, x);
            }
            None => {
                by_name.remove(&n);
                next.remove(&Some(n));
            }
        }
    }
    for l in links {
        next.insert(l.from.clone(), l.to.clone());
    }
    let mut out = Vec::with_capacity(by_name.len());
    let mut at = None;
    while let Some(Some(n)) = next.get(&at) {
        let x = by_name
            .get(n)
            .ok_or(Error::Decode("net delta: link to no element"))?;
        out.push(x.clone());
        if out.len() > by_name.len() {
            return Err(Error::Decode("net delta: links form a cycle"));
        }
        at = Some(n.clone());
    }
    if out.len() != by_name.len() {
        return Err(Error::Decode("net delta: links miss an element"));
    }
    Ok(out)
}

fn arc_changes(before: &Transition, after: &Transition) -> Vec<ArcChange> {
    let mut out = Vec::new();
    for output in [false, true] {
        let side = |t: &Transition| -> BTreeMap<String, u32> {
            let arcs = if output { &t.outputs } else { &t.inputs };
            let mut m = BTreeMap::new();
            for a in arcs {
                *m.entry(a.place.clone()).or_default() += a.weight;
            }
            m
        };
        let (b, a) = (side(before), side(after));
        let places: std::collections::BTreeSet<&String> = b.keys().chain(a.keys()).collect();
        for p in places {
            let (x, y) = (b.get(p).copied(), a.get(p).copied());
            if x != y {
                out.push(ArcChange {
                    transition: after.name.clone(),
                    output,
                    place: p.clone(),
                    before: x,
                    after: y,
                });
            }
        }
    }
    out
}

impl<S: Store> Repo<S> {
    /// Store `net` as fragments (E446): each line of its canonical text a
    /// content of its own, and a graph naming them laid out as `layout`
    /// says. Returns the root's id, the version's id in this form.
    /// Fragments and chunks earlier versions wrote are found, not written
    /// again.
    pub fn put_net_fragments(&mut self, net: &Net, layout: Layout) -> Result<Id, Error> {
        let layout = super::test_layout(layout);
        // E453: with `net_chunk_policy`, the graph is cut with the bounds
        // kept for this net (picked by its size when first written).
        self.with_net_bounds(net, layout, |r| {
            // E449: with `net_delta_write`, only what changed since the
            // last version written is hashed and stored; the root is the
            // same (every layout, the inline ones of E447 included: E451).
            if r.net_delta_write {
                return r.put_net_fragments_delta(net, layout);
            }
            r.put_net_fragments_whole(net, layout)
        })
    }

    /// `put_net_fragments` without the E449 options: every node built,
    /// every fragment put.
    pub(super) fn put_net_fragments_whole(
        &mut self,
        net: &Net,
        layout: Layout,
    ) -> Result<Id, Error> {
        match layout {
            Layout::Inline => return self.put_net_inline(net, InlineCodec::Compact),
            Layout::InlineFlat => {
                net.check()?;
                let mut nodes = inline::inline_nodes(net, InlineCodec::Compact);
                nodes.sort_by(|a, b| a.label.cmp(&b.label));
                return self.put(&Object::Graph(Graph::Nodes(nodes)));
            }
            Layout::Tree | Layout::Flat => {}
        }
        net.check()?;
        let mut nodes = Vec::new();
        for (node, text) in fragment_nodes(net) {
            self.put_content(text.as_bytes())?;
            nodes.push(node);
        }
        match layout {
            Layout::Flat => {
                nodes.sort_by(|a, b| a.label.cmp(&b.label));
                self.put(&Object::Graph(Graph::Nodes(nodes)))
            }
            _ => self.put_graph(nodes),
        }
    }

    /// Store `net` as fragments at `path` in tree `root` (None: an empty
    /// tree), as a graph entry, so commits chain its versions as they chain
    /// files. Returns the new tree and the version's id.
    pub fn put_net_fragment_file(
        &mut self,
        root: Option<Id>,
        path: &str,
        net: &Net,
        layout: Layout,
    ) -> Result<(Id, Id), Error> {
        // E453: a version already at `path` keeps the bounds it was cut with.
        if self.net_chunk_policy.is_some()
            && let Some(previous) = self.graph_at(root.as_ref(), path)?
        {
            self.find_net_bounds(net, super::test_layout(layout), &previous)?;
        }
        let id = self.put_net_fragments(net, layout)?;
        let edit = crate::worktree::Edit::PutGraph {
            path: path.into(),
            id,
        };
        Ok((self.edit_tree(root, &[edit])?, id))
    }

    /// The graph entry at `path` in tree `root`, if there is one (E453).
    fn graph_at(&self, root: Option<&Id>, path: &str) -> Result<Option<Id>, Error> {
        let Some(mut tree) = root.copied() else {
            return Ok(None);
        };
        let parts = crate::worktree::split_path(path)?;
        let (last, dirs) = parts.split_last().expect("split_path is never empty");
        for (i, name) in dirs.iter().chain([last]).enumerate() {
            let Object::Tree(entries) = self.get(&tree)? else {
                return Ok(None);
            };
            let kind = match i == dirs.len() {
                true => crate::Kind::Graph,
                false => crate::Kind::Tree,
            };
            match entries
                .into_iter()
                .find(|e| e.name == *name && e.kind == kind)
            {
                Some(e) => tree = e.id,
                None => return Ok(None),
            }
        }
        Ok(Some(tree))
    }

    /// The net a commit or tree holds as fragments at `path`.
    pub fn net_fragment_file(&self, rev: &Id, path: &str) -> Result<Net, Error> {
        let mut tree = crate::snapshot::tree_of(self, rev)?;
        let parts = crate::worktree::split_path(path)?;
        let (last, dirs) = parts.split_last().expect("split_path is never empty");
        let entry = |r: &Self, t: &Id, name: &str, kind: crate::Kind| -> Result<Id, Error> {
            let Object::Tree(entries) = r.get(t)? else {
                return Err(Error::WrongKind(*t));
            };
            (entries.into_iter())
                .find(|e| e.name == name && e.kind == kind)
                .map(|e| e.id)
                .ok_or_else(|| Error::InvalidName(path.into()))
        };
        for d in dirs {
            tree = entry(self, &tree, d, crate::Kind::Tree)?;
        }
        let id = entry(self, &tree, last, crate::Kind::Graph)?;
        self.net_fragments(&id)
    }

    /// Store `net` inline (E447): one graph, laid out as `Tree` is, whose
    /// nodes hold their elements' bytes as `codec` writes them. Returns the
    /// root, the version's id in this form; [`net_fragments`](Repo::net_fragments),
    /// [`net_delta`](Repo::net_delta) and [`net_version`](Repo::net_version)
    /// read it as they read E446's.
    pub fn put_net_inline(&mut self, net: &Net, codec: InlineCodec) -> Result<Id, Error> {
        net.check()?;
        self.put_graph(inline::inline_nodes(net, codec))
    }

    pub(super) fn read_node(&self, n: &Node) -> Result<Read, Error> {
        if let Target::Inline(bytes) = &n.target {
            return inline::read_inline(n, bytes);
        }
        let Target::Content(Arg::Whole(id)) = &n.target else {
            return Err(bad());
        };
        let text = String::from_utf8(self.read_content(id)?).map_err(|_| bad())?;
        let mut links = BTreeMap::new();
        for e in &n.edges {
            let (label, to) = e.label.split_once(' ').ok_or_else(bad)?;
            if node_key(to) != e.to || links.insert(label.to_string(), to.to_string()).is_some() {
                return Err(bad());
            }
        }
        Ok(Read {
            element: parse_fragment(&n.label, &text)?,
            links,
        })
    }

    /// Read a net stored by [`put_net_fragments`](Repo::put_net_fragments)
    /// from its root: every node, every fragment, the order followed from
    /// the head. The nodes must be exactly the ones the net it reads as
    /// would be stored with.
    pub fn net_fragments(&self, root: &Id) -> Result<Net, Error> {
        // E449: with `net_fast_read`, leaves decoded once and kept.
        if self.net_fast_read {
            return self.net_fragments_fast(root);
        }
        self.net_fragments_full(root)
    }

    /// `net_fragments` without the E449 options: every node and fragment
    /// read, and the nodes the net it reads as would be stored with built
    /// again and compared. The full check of a fragment version.
    pub fn net_fragments_full(&self, root: &Id) -> Result<Net, Error> {
        let nodes = self.graph_nodes(root)?;
        let mut read = HashMap::new();
        for n in &nodes {
            read.insert(n.label.clone(), self.read_node(n)?);
        }
        let Some(Read {
            element: Element::Head(name, version),
            links,
        }) = read.get(HEAD)
        else {
            return Err(bad());
        };
        let follow = |first: &str| -> Result<Vec<&Element>, Error> {
            let mut out = Vec::new();
            let mut at = links.get(first);
            while let Some(label) = at {
                let r = read.get(label).ok_or_else(bad)?;
                out.push(&r.element);
                if out.len() > read.len() {
                    return Err(bad());
                }
                at = r.links.get(NEXT);
            }
            Ok(out)
        };
        let mut net = Net {
            name: name.clone(),
            version: *version,
            places: vec![],
            transitions: vec![],
        };
        for e in follow(FIRST_PLACE)? {
            let Element::Place(p) = e else {
                return Err(bad());
            };
            net.places.push(p.clone());
        }
        for e in follow(FIRST_TRANSITION)? {
            let Element::Transition(t) = e else {
                return Err(bad());
            };
            net.transitions.push(t.clone());
        }
        if 1 + net.places.len() + net.transitions.len() != nodes.len() {
            return Err(bad());
        }
        net.check()?;
        // The root names this net's nodes and no others.
        let mut want: Vec<Node> = match &nodes.iter().find(|n| n.label == HEAD).map(|n| &n.target) {
            Some(Target::Inline(head)) => inline::inline_nodes(&net, inline::codec_of(head)?),
            _ => fragment_nodes(&net).into_iter().map(|(n, _)| n).collect(),
        };
        want.sort_by(|a, b| a.label.cmp(&b.label));
        if want != nodes {
            return Err(Error::Decode("net fragments: not this net's nodes"));
        }
        Ok(net)
    }

    /// The difference between two fragment versions of a net (E446), by
    /// their roots: a view over the traversal, never stored. Both graphs
    /// are walked a level at a time and the chunks they share are skipped
    /// unread; the nodes of the chunks left are compared, and only the
    /// fragments of nodes that differ are read.
    pub fn net_delta(&self, a: &Id, b: &Id) -> Result<NetDelta, Error> {
        self.net_delta_with(a, b, true)
    }

    /// `net_delta`, skipping shared subtrees only when `prune` says so
    /// (without, every chunk of both is read; the delta is the same).
    pub fn net_delta_with(&self, a: &Id, b: &Id, prune: bool) -> Result<NetDelta, Error> {
        let (changes, chunks_read) = self.graph_diff_counted(a, b, prune)?;
        let mut d = NetDelta {
            chunks_read,
            ..Default::default()
        };
        for (label, x, y) in &changes {
            // A node whose fragment is the same on both sides (only its
            // links changed) has its fragment read once.
            let same_target = matches!((x, y), (Some(x), Some(y)) if x.target == y.target);
            let ry = y.as_ref().map(|n| self.read_node(n)).transpose()?;
            let rx = match (same_target, x, &ry) {
                (true, _, Some(r)) => Some(Read {
                    element: r.element.clone(),
                    links: BTreeMap::new(),
                }),
                (_, x, _) => x.as_ref().map(|n| self.read_node(n)).transpose()?,
            };
            d.fragments_read += match same_target {
                true => 1,
                false => x.is_some() as usize + y.is_some() as usize,
            };
            if let Some(r) = &ry {
                let transitions = label.starts_with("t/");
                let own = match &r.element {
                    Element::Head(..) => None,
                    Element::Place(p) => Some(p.name.clone()),
                    Element::Transition(t) => Some(t.name.clone()),
                };
                let strip = |l: &String| l[2..].to_string();
                match &r.element {
                    Element::Head(..) => {
                        for (edge, list) in [(FIRST_PLACE, false), (FIRST_TRANSITION, true)] {
                            d.links.push(Link {
                                transitions: list,
                                from: None,
                                to: r.links.get(edge).map(strip),
                            });
                        }
                    }
                    _ => d.links.push(Link {
                        transitions,
                        from: own,
                        to: r.links.get(NEXT).map(strip),
                    }),
                }
            }
            match (rx.map(|r| r.element), ry.map(|r| r.element)) {
                (Some(Element::Head(n0, v0)), Some(Element::Head(n1, v1))) => {
                    if (&n0, v0) != (&n1, v1) {
                        d.head = Some(((n0, v0), (n1, v1)));
                    }
                }
                (None, Some(Element::Place(p))) => d.places.push(Change::Added(p)),
                (Some(Element::Place(p)), None) => d.places.push(Change::Removed(p)),
                (Some(Element::Place(before)), Some(Element::Place(after))) => {
                    match before == after {
                        true => d.relinked.push(after.name),
                        false => d.places.push(Change::Changed { before, after }),
                    }
                }
                (None, Some(Element::Transition(t))) => d.transitions.push(Change::Added(t)),
                (Some(Element::Transition(t)), None) => d.transitions.push(Change::Removed(t)),
                (Some(Element::Transition(before)), Some(Element::Transition(after))) => {
                    match before == after {
                        true => d.relinked.push(after.name),
                        false => {
                            d.arcs.extend(arc_changes(&before, &after));
                            d.transitions.push(Change::Changed { before, after });
                        }
                    }
                }
                _ => return Err(bad()),
            }
        }
        Ok(d)
    }
}
