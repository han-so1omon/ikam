//! Fragment versions of a net read and written at the speed of what
//! changed (E449; options, off by default).
//!
//! E446's read of a fragment version reads every node and every fragment
//! one at a time, parses each line, checks the whole net, then builds every
//! node again (formatting and hashing each line) to compare with the ones
//! read. Its write builds, hashes and puts every node and chunk even when
//! one line changed. Here:
//!
//! - **Read** ([`Repo::net_fast_read`]): each leaf chunk is fetched once,
//!   its fragments read together ([`Repo::read_contents`]) and parsed by a
//!   strict parser of the canonical line, which accepts exactly the lines
//!   [`Net::to_text`] prints, so a line that parses is its element's
//!   canonical line and its id is the one the node names; edge keys are
//!   checked as they are read. The decoded leaf is kept by
//!   [`Repo::graph_cache`] under its chunk id, so reading version N+1
//!   after N decodes only the leaves the edit wrote. The net is then put
//!   together by following the order links, with the checks
//!   [`Net::check`] makes that span elements (names unique across places
//!   and transitions, every arc's place defined). The full check, building
//!   every node again, stays in [`Repo::net_fragments_full`].
//! - **Write** ([`Repo::net_delta_write`]): the last version written of
//!   each net is kept with its root; a new version is compared with it
//!   element by element, only the nodes that changed (and the neighbours
//!   whose link changed) are built and their fragments put, and the graph
//!   is edited in place (`graph_edit`, which rewrites only the chunks the
//!   edits fall in). The root is the one `put_graph` gives. A version equal
//!   to the last one costs a comparison: a firing on a net that tracks
//!   fragments ([`Repo::net_track_fragments`]) no longer writes the net.
//! - **Partial** ([`Repo::net_fragments_part`], [`Repo::net_fire_at`]):
//!   one transition and its places read by key, for a firing that names
//!   the version by its root without the whole net in memory.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use super::fragments::{
    Element, FIRST_PLACE, FIRST_TRANSITION, HEAD, Layout, NEXT, bad, head_node, place_label,
    place_node, transition_label, transition_node,
};
use super::{Arc as NetArc, Firing, Kind, Net, Place, Transition, invalid, valid_name};
use crate::db::Db;
use crate::{Arg, Error, Graph, Id, Node, Repo, Store, Target, node_key};

/// FxHash (as `ir.rs` uses for its short keys): labels are short, and a
/// read hashes every one of them twice.
#[derive(Clone, Copy, Default)]
struct Fx(u64);

impl std::hash::Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        const K: u64 = 0x517c_c1b7_2722_0a95;
        let mut c = bytes.chunks_exact(8);
        for w in &mut c {
            let w = u64::from_le_bytes(w.try_into().unwrap());
            self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(K);
        }
        for &x in c.remainder() {
            self.0 = (self.0.rotate_left(5) ^ u64::from(x)).wrapping_mul(K);
        }
    }

    fn write_u8(&mut self, n: u8) {
        self.0 = (self.0.rotate_left(5) ^ u64::from(n)).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

type FxMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<Fx>>;
type FxSet<K> = HashSet<K, std::hash::BuildHasherDefault<Fx>>;

/// The kind a decoded leaf of a fragment net is kept under.
const NET_LEAF: &str = "net-leaf";

/// A node of a fragment version as the fast read decodes it.
#[derive(Debug)]
struct Item {
    label: String,
    element: Element,
    /// Target labels of the order edges: `next`, or the head's first place
    /// and first transition.
    next: Option<String>,
    first_place: Option<String>,
    first_transition: Option<String>,
}

/// The last fragment version written of a net (`net_delta_write`).
struct Last {
    net: Net,
    root: Id,
    layout: Layout,
}

/// A canonical decimal: digits, no leading zero.
fn canonical_u32(s: &str) -> Result<u32, Error> {
    let ok =
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'));
    if !ok {
        return Err(bad());
    }
    s.parse().map_err(|_| bad())
}

fn name_ok(s: &str) -> Result<&str, Error> {
    match valid_name(s) {
        true => Ok(s),
        false => Err(bad()),
    }
}

/// One side of a canonical transition line: `w*p` for weights above 1,
/// `p` for 1, joined by `, `.
fn canonical_side(s: &str) -> Result<Vec<NetArc>, Error> {
    let mut out = Vec::with_capacity(2);
    for (i, a) in s.split(',').enumerate() {
        let a = match i {
            0 => a,
            _ => a.strip_prefix(' ').ok_or_else(bad)?,
        };
        let (weight, place) = match a.find('*') {
            Some(at) => match canonical_u32(&a[..at])? {
                w if w >= 2 => (w, &a[at + 1..]),
                _ => return Err(bad()),
            },
            None => (1, a),
        };
        out.push(NetArc {
            place: name_ok(place)?.into(),
            weight,
        });
    }
    Ok(out)
}

/// The element of a node labelled `label` whose fragment is `text`, when
/// `text` is exactly the canonical line of an element of that label (the
/// line `Net::to_text` prints for it); refused otherwise. Element-level
/// rules of `Net::check` are applied here.
fn parse_canonical(label: &str, text: &str) -> Result<Element, Error> {
    let line = text.strip_suffix('\n').ok_or_else(bad)?;
    if label == HEAD {
        let rest = line.strip_prefix("net ").ok_or_else(bad)?;
        let (name, version) = rest.split_once(' ').ok_or_else(bad)?;
        return Ok(Element::Head(
            name_ok(name)?.into(),
            canonical_u32(version)?,
        ));
    }
    if let Some(name) = label.strip_prefix("p/") {
        let rest = line.strip_prefix("place ").ok_or_else(bad)?;
        let tail = rest.strip_prefix(name_ok(name)?).ok_or_else(bad)?;
        let (colored, initial) = match tail {
            "" => (false, 0),
            " colored" => (true, 0),
            t => match canonical_u32(t.strip_prefix(' ').ok_or_else(bad)?)? {
                0 => return Err(bad()),
                k => (false, k),
            },
        };
        return Ok(Element::Place(Place {
            name: name.into(),
            colored,
            initial,
        }));
    }
    let name = label.strip_prefix("t/").ok_or_else(bad)?;
    let rest = line.strip_prefix("transition ").ok_or_else(bad)?;
    let rest = rest.strip_prefix(name_ok(name)?).ok_or_else(bad)?;
    let rest = rest.strip_prefix(' ').ok_or_else(bad)?;
    let (head, arrow) = rest.split_once(':').ok_or_else(bad)?;
    let arrow = arrow.strip_prefix(' ').ok_or_else(bad)?;
    let (kind, capability) = match head.split_once(' ') {
        None => (head, None),
        Some((k, c)) => (k, Some(name_ok(c)?.to_string())),
    };
    let kind = Kind::parse(kind).ok_or_else(bad)?;
    if kind == Kind::Human && capability.is_some() {
        return Err(bad());
    }
    let (ins, outs) = if arrow == "->" {
        (vec![], vec![])
    } else if let Some(outs) = arrow.strip_prefix("-> ") {
        (vec![], canonical_side(outs)?)
    } else if let Some(ins) = arrow.strip_suffix(" ->") {
        (canonical_side(ins)?, vec![])
    } else {
        let at = arrow.find('>').ok_or_else(bad)?;
        let i = arrow[..at].strip_suffix(" -").ok_or_else(bad)?;
        let o = arrow[at + 1..].strip_prefix(' ').ok_or_else(bad)?;
        (canonical_side(i)?, canonical_side(o)?)
    };
    if ins.is_empty() && outs.is_empty() {
        return Err(invalid("transition with no arcs", name));
    }
    Ok(Element::Transition(Transition {
        name: name.into(),
        inputs: ins,
        outputs: outs,
        kind,
        capability,
    }))
}

/// A node and its fragment as the fast read decodes them: the strict
/// parse, and each edge's label and key checked.
fn decode_item(n: Node, text: &[u8]) -> Result<Item, Error> {
    let text = std::str::from_utf8(text).map_err(|_| bad())?;
    let element = parse_canonical(&n.label, text)?;
    let (mut next, mut first_place, mut first_transition) = (None, None, None);
    let head = n.label == HEAD;
    for e in &n.edges {
        let (label, to) = e.label.split_once(' ').ok_or_else(bad)?;
        if node_key(to) != e.to {
            return Err(bad());
        }
        let slot = match (head, label) {
            (true, FIRST_PLACE) => &mut first_place,
            (true, FIRST_TRANSITION) => &mut first_transition,
            (false, NEXT) => &mut next,
            _ => return Err(bad()),
        };
        if slot.replace(to.to_string()).is_some() {
            return Err(bad());
        }
    }
    Ok(Item {
        label: n.label,
        element,
        next,
        first_place,
        first_transition,
    })
}

/// A fragment version put together from its decoded leaves, in label
/// order: the order links followed from the head, and the checks of
/// `Net::check` that span elements.
fn assemble(leaves: &[Arc<Vec<Item>>]) -> Result<Net, Error> {
    let total = leaves.iter().map(|l| l.len()).sum();
    let mut by: FxMap<&str, &Item> = FxMap::with_capacity_and_hasher(total, Default::default());
    for item in leaves.iter().flat_map(|l| l.iter()) {
        if by.insert(&item.label, item).is_some() {
            return Err(bad());
        }
    }
    let find = |label: &str| -> Result<&Item, Error> { by.get(label).copied().ok_or_else(bad) };
    let head = find(HEAD)?;
    let Element::Head(name, version) = &head.element else {
        return Err(bad());
    };
    let mut net = Net {
        name: name.clone(),
        version: *version,
        places: Vec::new(),
        transitions: Vec::new(),
    };
    let mut at = head.first_place.as_deref();
    while let Some(label) = at {
        let item = find(label)?;
        let Element::Place(p) = &item.element else {
            return Err(bad());
        };
        net.places.push(p.clone());
        if net.places.len() > total {
            return Err(bad());
        }
        at = item.next.as_deref();
    }
    let mut at = head.first_transition.as_deref();
    while let Some(label) = at {
        let item = find(label)?;
        let Element::Transition(t) = &item.element else {
            return Err(bad());
        };
        net.transitions.push(t.clone());
        if net.transitions.len() > total {
            return Err(bad());
        }
        at = item.next.as_deref();
    }
    if 1 + net.places.len() + net.transitions.len() != total {
        return Err(bad());
    }
    // What `Net::check` refuses across elements (each element passed its
    // own rules when decoded; labels are unique within a graph).
    let mut places: FxSet<&str> =
        FxSet::with_capacity_and_hasher(net.places.len(), Default::default());
    places.extend(net.places.iter().map(|p| p.name.as_str()));
    for t in &net.transitions {
        if places.contains(t.name.as_str()) {
            return Err(invalid("transition", &t.name));
        }
        for a in t.inputs.iter().chain(&t.outputs) {
            if !places.contains(a.place.as_str()) {
                return Err(invalid("arc to an undefined place", &a.place));
            }
        }
    }
    Ok(net)
}

/// `Net::check`, its name sets hashed: the same result and the same first
/// error, in a third of the time at 10,000 transitions (E449; the delta
/// write checks every new version with it).
fn check_hashed(net: &Net) -> Result<(), Error> {
    if !valid_name(&net.name) {
        return Err(invalid("net", &net.name));
    }
    let mut seen = HashSet::with_capacity(net.places.len() + net.transitions.len());
    let mut places = HashSet::with_capacity(net.places.len());
    for p in &net.places {
        if !valid_name(&p.name) || !seen.insert(p.name.as_str()) {
            return Err(invalid("place", &p.name));
        }
        places.insert(p.name.as_str());
        if p.colored && p.initial > 0 {
            return Err(invalid("colored place with initial tokens", &p.name));
        }
    }
    for t in &net.transitions {
        if !valid_name(&t.name) || !seen.insert(t.name.as_str()) {
            return Err(invalid("transition", &t.name));
        }
        if t.inputs.is_empty() && t.outputs.is_empty() {
            return Err(invalid("transition with no arcs", &t.name));
        }
        for a in t.inputs.iter().chain(&t.outputs) {
            if !places.contains(a.place.as_str()) {
                return Err(invalid("arc to an undefined place", &a.place));
            }
            if a.weight == 0 {
                return Err(invalid("arc of weight 0 to", &a.place));
            }
        }
        if let Some(c) = &t.capability
            && (t.kind == Kind::Human || !valid_name(c))
        {
            return Err(invalid("capability", c));
        }
    }
    Ok(())
}

/// The edits that turn the fragment form of `old` into that of `new`: a
/// node for each element added or changed and for each whose `next`
/// changed, a removal for each element gone. Returns also the fragment
/// texts that are new.
fn list_edits<T: PartialEq>(
    old: &[T],
    new: &[T],
    name: fn(&T) -> &str,
    label: fn(&str) -> String,
    node: fn(&T, Option<&T>) -> (Node, String),
    edits: &mut Vec<(String, Option<Node>)>,
    texts: &mut Vec<String>,
) {
    // Most of two versions line up by position; only where they do not is
    // an element looked up by name.
    let aligned = |i: usize, x: &T| old.get(i).is_some_and(|o| name(o) == name(x));
    let mut at: Option<HashMap<&str, usize>> = None;
    for (i, x) in new.iter().enumerate() {
        let next = new.get(i + 1);
        let j = match aligned(i, x) {
            true => Some(i),
            false => at
                .get_or_insert_with(|| old.iter().enumerate().map(|(j, o)| (name(o), j)).collect())
                .get(name(x))
                .copied(),
        };
        let (same_element, same_next) = match j {
            Some(j) => (old[j] == *x, old.get(j + 1).map(name) == next.map(name)),
            None => (false, false),
        };
        if same_element && same_next {
            continue;
        }
        let (n, text) = node(x, next);
        if !same_element {
            texts.push(text);
        }
        edits.push((n.label.clone(), Some(n)));
    }
    if old.len() != new.len() || old.iter().zip(new).any(|(o, n)| name(o) != name(n)) {
        let kept: HashSet<&str> = new.iter().map(name).collect();
        for o in old.iter().filter(|o| !kept.contains(name(o))) {
            edits.push((label(name(o)), None));
        }
    }
}

impl<S: Store> Repo<S> {
    /// A leaf of a fragment version, decoded by the fast read.
    fn net_leaf(&self, leaf: &Id) -> Result<Vec<Item>, Error> {
        let Graph::Nodes(nodes) = self.graph_chunk(leaf)? else {
            return Err(Error::WrongKind(*leaf));
        };
        let mut out = Vec::with_capacity(nodes.len());
        for n in nodes {
            let Target::Content(Arg::Whole(id)) = n.target else {
                return Err(bad());
            };
            let text = self.read_content_direct(&id)?;
            out.push(decode_item(n, &text)?);
        }
        Ok(out)
    }

    /// The net a fragment root holds, read as `net_fast_read` reads it
    /// (whether the option is on or not): each leaf decoded once and kept
    /// by `graph_cache`. Gives what [`net_fragments_full`](Repo::net_fragments_full)
    /// gives, and refuses what it refuses.
    pub fn net_fragments_fast(&self, root: &Id) -> Result<Net, Error> {
        let leaves = (self.graph_leaf_ids(root)?.iter())
            .map(|l| self.graph_decoded(l, NET_LEAF, || self.net_leaf(l)))
            .collect::<Result<Vec<_>, Error>>()?;
        assemble(&leaves)
    }

    /// Only the transitions `transitions` of the fragment version at
    /// `root`, with the places their arcs name and the net's name and
    /// version, read by key: one path of chunks per label, kept by
    /// `graph_cache`. The places come in the order the arcs name them. A
    /// transition the version lacks is refused.
    pub fn net_fragments_part(&self, root: &Id, transitions: &[&str]) -> Result<Net, Error> {
        let read = |labels: &BTreeSet<String>| -> Result<Vec<Item>, Error> {
            let nodes = self.graph_nodes_in_cached(root, labels)?;
            if nodes.len() != labels.len() {
                return Err(invalid("transition", "not in this version"));
            }
            let ids: Vec<Id> = nodes
                .iter()
                .map(|n| match &n.target {
                    Target::Content(Arg::Whole(id)) => Ok(*id),
                    _ => Err(bad()),
                })
                .collect::<Result<_, _>>()?;
            let texts = self.read_contents(&ids)?;
            (nodes.into_iter().zip(&texts))
                .map(|(n, t)| decode_item(n, t))
                .collect()
        };
        let mut labels: BTreeSet<String> =
            transitions.iter().map(|t| transition_label(t)).collect();
        labels.insert(HEAD.into());
        let mut net = Net {
            name: String::new(),
            version: 0,
            places: vec![],
            transitions: vec![],
        };
        let mut by_label = HashMap::new();
        for item in read(&labels)? {
            by_label.insert(item.label, item.element);
        }
        match by_label.remove(HEAD) {
            Some(Element::Head(name, version)) => (net.name, net.version) = (name, version),
            _ => return Err(bad()),
        }
        let mut wanted = Vec::new();
        for t in transitions {
            let Some(Element::Transition(t)) = by_label.get(&transition_label(t)) else {
                return Err(bad());
            };
            for a in t.inputs.iter().chain(&t.outputs) {
                if !wanted.contains(&a.place) {
                    wanted.push(a.place.clone());
                }
            }
            net.transitions.push(t.clone());
        }
        let labels: BTreeSet<String> = wanted.iter().map(|p| place_label(p)).collect();
        let mut places = HashMap::new();
        for item in read(&labels)? {
            if let Element::Place(p) = item.element {
                places.insert(p.name.clone(), p);
            }
        }
        for p in &wanted {
            net.places.push(places.remove(p).ok_or_else(bad)?);
        }
        net.check()?;
        Ok(net)
    }

    /// Fire `transition` of the fragment version at `root` as
    /// [`net_fire`](Repo::net_fire) fires it on the whole net, reading only
    /// that transition and its places ([`net_fragments_part`](Repo::net_fragments_part)).
    /// The net must track fragment versions (`net_track_fragments`); the
    /// firing names `root`.
    pub fn net_fire_at(
        &mut self,
        db: &Db,
        root: &Id,
        transition: &str,
        token: Option<&str>,
        key: &str,
    ) -> Result<(Db, Firing), Error> {
        let part = self.net_fragments_part(root, &[transition])?;
        if !matches!(self.tracking(db, &part.name)?, Some((_, Some(_)))) {
            return Err(invalid("net does not track fragment versions", &part.name));
        }
        self.fire_inner(db, &part, transition, token, key, None, None, Some(*root))
    }

    /// `put_net_fragments` with `net_delta_write`: against the last
    /// version this handle wrote of the same net, in the tree layout, only
    /// what changed is built and put and the graph is edited in place. The
    /// root equals the whole write's.
    pub(super) fn put_net_fragments_delta(
        &mut self,
        net: &Net,
        layout: Layout,
    ) -> Result<Id, Error> {
        let key = format!("net/{}", net.name);
        let last = self.graph_named::<Last>(&key);
        let id = match last {
            Some(l) if l.layout == layout && l.net == *net && self.store.has(&l.root) => {
                return Ok(l.root);
            }
            Some(l)
                if l.layout == Layout::Tree
                    && layout == Layout::Tree
                    && self.store.has(&l.root) =>
            {
                check_hashed(net)?;
                self.fragment_edit(&l.net, &l.root, net)?
            }
            _ => self.put_net_fragments_whole(net, layout)?,
        };
        self.graph_name(
            key,
            Arc::new(Last {
                net: net.clone(),
                root: id,
                layout,
            }),
        );
        Ok(id)
    }

    /// The tree-layout root of `new` from that of `old` at `root`.
    fn fragment_edit(&mut self, old: &Net, root: &Id, new: &Net) -> Result<Id, Error> {
        let (mut edits, mut texts) = (Vec::new(), Vec::new());
        let first = |n: &Net| {
            (
                n.places.first().map(|p| p.name.clone()),
                n.transitions.first().map(|t| t.name.clone()),
            )
        };
        if (&old.name, old.version, first(old)) != (&new.name, new.version, first(new)) {
            let (n, text) = head_node(new);
            texts.push(text);
            edits.push((n.label.clone(), Some(n)));
        }
        list_edits(
            &old.places,
            &new.places,
            |p| &p.name,
            place_label,
            place_node,
            &mut edits,
            &mut texts,
        );
        list_edits(
            &old.transitions,
            &new.transitions,
            |t| &t.name,
            transition_label,
            transition_node,
            &mut edits,
            &mut texts,
        );
        for t in &texts {
            self.put_content(t.as_bytes())?;
        }
        self.graph_edit(root, edits.into_iter().collect())
    }
}

impl<S: Store + Sync> Repo<S> {
    /// [`net_fragments_fast`](Repo::net_fragments_fast), the leaves the
    /// cache does not keep decoded on up to `read_threads` threads.
    pub fn net_fragments_par(&self, root: &Id) -> Result<Net, Error> {
        let leaves = self.graph_leaf_ids(root)?;
        let decoded = self.graph_decoded_par(&leaves, NET_LEAF, |l| self.net_leaf(l))?;
        assemble(&decoded)
    }
}
