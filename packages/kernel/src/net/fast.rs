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
    Element, FIRST_PLACE, FIRST_TRANSITION, HEAD, InlineCodec, Layout, NEXT, bad, head_node,
    head_payload, place_label, place_node, place_payload, read_inline, transition_label,
    transition_node, transition_payload,
};
use super::text::{place_line, transition_line};
use super::{Arc as NetArc, Firing, Kind, Net, Place, Transition, invalid, valid_name};
use crate::db::Db;
use crate::{Arg, ChunkSizes, Error, Graph, Id, Node, Repo, Store, Target, node_key};

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
    /// The stored forms this node is exactly the node of (E451, `FORM_*`):
    /// a version's nodes must share one, as the full read requires.
    forms: u8,
}

/// An E446 node: its fragment a content of its own.
const FORM_CONTENT: u8 = 1;
/// Inline (E447) under [`InlineCodec::Lines`]: E446's line and edges.
const FORM_LINES: u8 = 2;
/// Inline under [`InlineCodec::LinesNext`].
const FORM_LINES_NEXT: u8 = 4;
/// Inline under [`InlineCodec::Compact`] (`Layout::Inline`, `InlineFlat`).
const FORM_COMPACT: u8 = 8;

/// The last fragment version written of a net (`net_delta_write`).
struct Last {
    net: Net,
    root: Id,
    layout: Layout,
    /// The chunk bounds the root was cut with: under other bounds the same
    /// net has another root, so the kept one is not reused (E451).
    chunks: ChunkSizes,
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
        forms: FORM_CONTENT,
    })
}

/// The rules `Net::check` applies to one element (E451; the strict
/// canonical parse of `decode_item` applies them as it parses, an inline
/// payload is checked here).
fn check_element(el: &Element) -> Result<(), Error> {
    match el {
        Element::Head(name, _) => {
            name_ok(name)?;
        }
        Element::Place(p) => {
            name_ok(&p.name)?;
            if p.colored && p.initial > 0 {
                return Err(invalid("colored place with initial tokens", &p.name));
            }
        }
        Element::Transition(t) => {
            name_ok(&t.name)?;
            if t.inputs.is_empty() && t.outputs.is_empty() {
                return Err(invalid("transition with no arcs", &t.name));
            }
            for a in t.inputs.iter().chain(&t.outputs) {
                name_ok(&a.place)?;
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
    }
    Ok(())
}

/// One side of a compact transition payload (E447's codec): `w*p` for
/// weights above 1, `p` for 1, joined by `,`; empty for no arcs.
fn compact_side(s: &str) -> Result<Vec<NetArc>, Error> {
    if s.is_empty() {
        return Ok(vec![]);
    }
    let mut out = Vec::with_capacity(2);
    for a in s.split(',') {
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

/// The label a compact payload's link names: `prefix` and the name, or
/// none for "".
fn compact_link(prefix: &str, name: &str) -> Result<Option<String>, Error> {
    match name {
        "" => Ok(None),
        n => Ok(Some(format!("{prefix}{}", name_ok(n)?))),
    }
}

/// A node in E447's compact codec, parsed strictly (E451): accepted only
/// when `bytes` is exactly the payload `inline_nodes` writes for what is
/// read, with the element rules of `Net::check`, so no payload is printed
/// again to compare. None when the payload is not in the compact codec.
fn decode_compact(label: &str, bytes: &[u8]) -> Result<Option<Item>, Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| bad())?;
    let item = |element, next, first_place, first_transition| Item {
        label: label.to_string(),
        element,
        next,
        first_place,
        first_transition,
        forms: FORM_COMPACT,
    };
    if label == HEAD {
        let Some(rest) = text.strip_prefix('@') else {
            return Ok(None);
        };
        let (line, firsts) = rest.split_once('\n').ok_or_else(bad)?;
        let (fp, ft) = firsts.split_once('\n').ok_or_else(bad)?;
        let (name, version) = line.split_once(' ').ok_or_else(bad)?;
        let head = Element::Head(name_ok(name)?.into(), canonical_u32(version)?);
        let (fp, ft) = (compact_link("p/", fp)?, compact_link("t/", ft)?);
        return Ok(Some(item(head, None, fp, ft)));
    }
    if text.starts_with("place ") || text.starts_with("transition ") {
        return Ok(None);
    }
    let (body, next) = text.split_once('\n').ok_or_else(bad)?;
    if let Some(name) = label.strip_prefix("p/") {
        let (colored, initial) = match body {
            "" => (false, 0),
            "c" => (true, 0),
            k => match canonical_u32(k)? {
                0 => return Err(bad()),
                k => (false, k),
            },
        };
        let place = Element::Place(Place {
            name: name_ok(name)?.into(),
            colored,
            initial,
        });
        return Ok(Some(item(place, compact_link("p/", next)?, None, None)));
    }
    let name = label.strip_prefix("t/").ok_or_else(bad)?;
    let (head, arcs) = body.split_once(':').ok_or_else(bad)?;
    let (kind, capability) = match head.split_once(' ') {
        None => (head, None),
        Some((k, c)) => (k, Some(name_ok(c)?.to_string())),
    };
    let kind = match kind {
        "p" => Kind::Pure,
        "c" => Kind::Cmd,
        "a" => Kind::Agent,
        "h" if capability.is_none() => Kind::Human,
        _ => return Err(bad()),
    };
    let (ins, outs) = arcs.split_once('>').ok_or_else(bad)?;
    let (inputs, outputs) = (compact_side(ins)?, compact_side(outs)?);
    if inputs.is_empty() && outputs.is_empty() {
        return Err(invalid("transition with no arcs", name));
    }
    let t = Element::Transition(Transition {
        name: name_ok(name)?.into(),
        inputs,
        outputs,
        kind,
        capability,
    });
    Ok(Some(item(t, compact_link("t/", next)?, None, None)))
}

/// An inline node (E447, `Target::Inline`) as the fast read decodes it
/// (E451): read as `read_inline` reads it, then refused unless its payload
/// is exactly the one `inline_nodes` writes for what was read under some
/// codec, and its element passes `check_element`. A node with edges is
/// E446's line inline (`InlineCodec::Lines`) and is decoded as E446's.
fn decode_inline(n: Node) -> Result<Item, Error> {
    let Target::Inline(bytes) = &n.target else {
        return Err(bad());
    };
    if !n.edges.is_empty() {
        let bytes = bytes.clone();
        let mut item = decode_item(n, &bytes)?;
        item.forms = FORM_LINES;
        return Ok(item);
    }
    if let Some(item) = decode_compact(&n.label, bytes)? {
        return Ok(item);
    }
    let r = read_inline(&n, bytes)?;
    check_element(&r.element)?;
    // Items hold the labels links name; payloads hold the names.
    let link = |edge: &str| r.links.get(edge).cloned();
    let (next, first_place, first_transition) = match &r.element {
        Element::Head(..) => (None, link(FIRST_PLACE), link(FIRST_TRANSITION)),
        _ => (link(NEXT), None, None),
    };
    let codecs = [
        (InlineCodec::LinesNext, FORM_LINES_NEXT),
        (InlineCodec::Compact, FORM_COMPACT),
    ];
    let mut forms = 0;
    match &r.element {
        Element::Head(name, version) => {
            let (fp, ft) = (name_in(&first_place), name_in(&first_transition));
            for (codec, form) in codecs {
                if head_payload(name, *version, fp, ft, codec).as_bytes() == bytes.as_slice() {
                    forms |= form;
                }
            }
            if fp.is_empty()
                && ft.is_empty()
                && *bytes == format!("net {name} {version}\n").as_bytes()
            {
                forms |= FORM_LINES;
            }
        }
        Element::Place(p) => {
            let nx = name_in(&next);
            for (codec, form) in codecs {
                if place_payload(p, nx, codec).as_bytes() == bytes.as_slice() {
                    forms |= form;
                }
            }
            if nx.is_empty() && place_line(p).as_bytes() == bytes.as_slice() {
                forms |= FORM_LINES;
            }
        }
        Element::Transition(t) => {
            let nx = name_in(&next);
            for (codec, form) in codecs {
                if transition_payload(t, nx, codec).as_bytes() == bytes.as_slice() {
                    forms |= form;
                }
            }
            if nx.is_empty() && transition_line(t).as_bytes() == bytes.as_slice() {
                forms |= FORM_LINES;
            }
        }
    }
    if forms == 0 {
        return Err(bad());
    }
    Ok(Item {
        label: n.label,
        element: r.element,
        next,
        first_place,
        first_transition,
        forms,
    })
}

/// The name a link's target label names ("" for none).
fn name_in(l: &Option<String>) -> &str {
    l.as_deref().and_then(|l| l.get(2..)).unwrap_or("")
}

/// A node of a fragment version, E446's or inline, as the fast read
/// decodes it; `text` is the fragment of an E446 node.
fn decode_node(n: Node, text: Option<&[u8]>) -> Result<Item, Error> {
    match (&n.target, text) {
        (Target::Inline(_), _) => decode_inline(n),
        (Target::Content(Arg::Whole(_)), Some(text)) => decode_item(n, text),
        _ => Err(bad()),
    }
}

/// The ids of the fragments E446 nodes among `nodes` name, in order.
fn fragment_ids(nodes: &[Node]) -> Result<Vec<Id>, Error> {
    let mut out = Vec::new();
    for n in nodes {
        match &n.target {
            Target::Content(Arg::Whole(id)) => out.push(*id),
            Target::Inline(_) => {}
            _ => return Err(bad()),
        }
    }
    Ok(out)
}

/// A fragment version put together from its decoded leaves, in label
/// order: the order links followed from the head, and the checks of
/// `Net::check` that span elements.
fn assemble(leaves: &[Arc<Vec<Item>>]) -> Result<Net, Error> {
    let total = leaves.iter().map(|l| l.len()).sum();
    let mut by: FxMap<&str, &Item> = FxMap::with_capacity_and_hasher(total, Default::default());
    let mut forms = u8::MAX;
    for item in leaves.iter().flat_map(|l| l.iter()) {
        forms &= item.forms;
        if by.insert(&item.label, item).is_some() {
            return Err(bad());
        }
    }
    // E451: one stored form for every node, as the full read builds them.
    if forms == 0 {
        return Err(Error::Decode("net fragments: not this net's nodes"));
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

/// An inline compact node (E447's `Layout::Inline`) with its payload,
/// as `inline_nodes` builds it; the text is empty (nothing is put).
fn compact_node(label: String, payload: String) -> (Node, String) {
    let node = Node {
        label,
        target: Target::Inline(payload.into_bytes()),
        edges: vec![],
    };
    (node, String::new())
}

fn compact_head_node(net: &Net) -> (Node, String) {
    let name = |n: Option<&String>| n.cloned().unwrap_or_default();
    let fp = name(net.places.first().map(|p| &p.name));
    let ft = name(net.transitions.first().map(|t| &t.name));
    let payload = head_payload(&net.name, net.version, &fp, &ft, InlineCodec::Compact);
    compact_node(HEAD.into(), payload)
}

fn compact_place_node(p: &Place, next: Option<&Place>) -> (Node, String) {
    let next = next.map(|n| n.name.as_str()).unwrap_or("");
    compact_node(
        place_label(&p.name),
        place_payload(p, next, InlineCodec::Compact),
    )
}

fn compact_transition_node(t: &Transition, next: Option<&Transition>) -> (Node, String) {
    let next = next.map(|n| n.name.as_str()).unwrap_or("");
    compact_node(
        transition_label(&t.name),
        transition_payload(t, next, InlineCodec::Compact),
    )
}

impl<S: Store> Repo<S> {
    /// A leaf of a fragment version, decoded by the fast read.
    fn net_leaf(&self, leaf: &Id) -> Result<Vec<Item>, Error> {
        let Graph::Nodes(nodes) = self.graph_chunk(leaf)? else {
            return Err(Error::WrongKind(*leaf));
        };
        let mut out = Vec::with_capacity(nodes.len());
        for n in nodes {
            let text = match &n.target {
                Target::Content(Arg::Whole(id)) => Some(self.read_content_direct(id)?),
                _ => None,
            };
            out.push(decode_node(n, text.as_deref())?);
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
            let texts = self.read_contents(&fragment_ids(&nodes)?)?;
            let mut texts = texts.iter();
            (nodes.into_iter())
                .map(|n| match &n.target {
                    Target::Inline(_) => decode_node(n, None),
                    _ => decode_node(n, texts.next().map(|t| t.as_slice())),
                })
                .collect()
        };
        // E451: the nodes read share one stored form.
        let mut forms = u8::MAX;
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
            forms &= item.forms;
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
            forms &= item.forms;
            if let Element::Place(p) = item.element {
                places.insert(p.name.clone(), p);
            }
        }
        if forms == 0 {
            return Err(Error::Decode("net fragments: not this net's nodes"));
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
            Some(l)
                if l.layout == layout
                    && l.chunks == self.graph_chunks
                    && l.net == *net
                    && self.store.has(&l.root) =>
            {
                return Ok(l.root);
            }
            Some(l)
                if l.layout == layout
                    && l.chunks == self.graph_chunks
                    && matches!(layout, Layout::Tree | Layout::Inline)
                    && self.store.has(&l.root) =>
            {
                check_hashed(net)?;
                self.fragment_edit(&l.net, &l.root, net, layout)?
            }
            _ => self.put_net_fragments_whole(net, layout)?,
        };
        self.graph_name(
            key,
            Arc::new(Last {
                net: net.clone(),
                root: id,
                layout,
                chunks: self.graph_chunks,
            }),
        );
        Ok(id)
    }

    /// The root of `new` laid out as `layout` (`Tree`, or E447's `Inline`:
    /// E451) from that of `old` at `root`.
    fn fragment_edit(
        &mut self,
        old: &Net,
        root: &Id,
        new: &Net,
        layout: Layout,
    ) -> Result<Id, Error> {
        let inline = layout == Layout::Inline;
        let (mut edits, mut texts) = (Vec::new(), Vec::new());
        let first = |n: &Net| {
            (
                n.places.first().map(|p| p.name.clone()),
                n.transitions.first().map(|t| t.name.clone()),
            )
        };
        if (&old.name, old.version, first(old)) != (&new.name, new.version, first(new)) {
            let (n, text) = match inline {
                true => compact_head_node(new),
                false => head_node(new),
            };
            texts.push(text);
            edits.push((n.label.clone(), Some(n)));
        }
        list_edits(
            &old.places,
            &new.places,
            |p| &p.name,
            place_label,
            match inline {
                true => compact_place_node,
                false => place_node,
            },
            &mut edits,
            &mut texts,
        );
        list_edits(
            &old.transitions,
            &new.transitions,
            |t| &t.name,
            transition_label,
            match inline {
                true => compact_transition_node,
                false => transition_node,
            },
            &mut edits,
            &mut texts,
        );
        if !inline {
            for t in &texts {
                self.put_content(t.as_bytes())?;
            }
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
