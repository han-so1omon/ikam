//! Fragments kept inline (E447, opt-in: [`Layout::Inline`] and
//! [`Repo::put_net_inline`]; nothing in the default paths calls this).
//!
//! E446 stores each element of a net as a content of its own and names it
//! from a graph node by a 32-byte id, with its order kept by `next` edges
//! (an 8-byte key and a label each). At 1,000 transitions that made the
//! first version 14 times the bytes of the text: the element's own bytes
//! were a third of a node, ids and keys (which no compressor shrinks) most
//! of the rest, and every element was an object besides.
//!
//! Here the element's bytes are the node's target ([`Target::Inline`]), so
//! they sit in the leaf chunk beside their neighbours and the chunk, stored
//! as content like every graph chunk, is compressed and sliced against its
//! earlier versions as a whole. The order link moves into the payload as
//! the next element's name (text that compresses like the rest), and the
//! edges go: nothing in a node is a hash. Nodes, labels and the prolly
//! tree are E446's, so an edit still rewrites only the chunks it falls in,
//! the same net always has the same root, and [`Repo::net_delta`] walks
//! these roots as it walks E446's.
//!
//! Three codecs ([`InlineCodec`]), to measure each step alone: `Lines`
//! keeps E446's canonical line and its edges and only inlines the line;
//! `LinesNext` drops the edges for a next name after the line; `Compact`
//! (what [`Layout::Inline`] uses) also drops what the label already says
//! (the word and the name) and writes the kind as one letter. Each payload
//! rebuilds its element's canonical line exactly, so the text form, and
//! E422's id of it, is always one `to_text` away.

use std::collections::BTreeMap;

use super::{Element, FIRST_PLACE, FIRST_TRANSITION, HEAD, NEXT, Read, bad, fragment_nodes};
use super::{parse_fragment, place_label, transition_label};
use crate::net::text::{place_line, transition_line};
use crate::net::{Arc, Kind, Net, Place, Transition};
use crate::{Error, Node, Target};

/// How an inline node's payload is written (see the module notes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineCodec {
    /// E446's canonical line inline, E446's order edges kept.
    Lines,
    /// The canonical line, then the next element's name; no edges.
    LinesNext,
    /// A short form the label completes, then the next name; no edges.
    Compact,
}

/// Marks a compact head payload: never the start of a canonical line.
const COMPACT_HEAD: u8 = b'@';

fn kind_letter(k: Kind) -> char {
    match k {
        Kind::Pure => 'p',
        Kind::Cmd => 'c',
        Kind::Agent => 'a',
        Kind::Human => 'h',
    }
}

fn letter_kind(c: u8) -> Option<Kind> {
    Some(match c {
        b'p' => Kind::Pure,
        b'c' => Kind::Cmd,
        b'a' => Kind::Agent,
        b'h' => Kind::Human,
        _ => return None,
    })
}

fn side(arcs: &[Arc]) -> String {
    let v: Vec<String> = arcs
        .iter()
        .map(|a| match a.weight {
            1 => a.place.clone(),
            w => format!("{w}*{}", a.place),
        })
        .collect();
    v.join(",")
}

fn compact_place(p: &Place) -> String {
    match (p.colored, p.initial) {
        (true, _) => "c".into(),
        (false, 0) => String::new(),
        (false, k) => k.to_string(),
    }
}

fn compact_transition(t: &Transition) -> String {
    let cap = t
        .capability
        .as_ref()
        .map(|c| format!(" {c}"))
        .unwrap_or_default();
    format!(
        "{}{cap}:{}>{}",
        kind_letter(t.kind),
        side(&t.inputs),
        side(&t.outputs)
    )
}

/// The nodes of `net`'s inline form under `codec`, in no particular order.
pub(super) fn inline_nodes(net: &Net, codec: InlineCodec) -> Vec<Node> {
    if codec == InlineCodec::Lines {
        return fragment_nodes(net)
            .into_iter()
            .map(|(mut n, text)| {
                n.target = Target::Inline(text.into_bytes());
                n
            })
            .collect();
    }
    let node = |label: String, payload: String| Node {
        label,
        target: Target::Inline(payload.into_bytes()),
        edges: vec![],
    };
    let name_of = |n: Option<&String>| n.cloned().unwrap_or_default();
    let first_p = name_of(net.places.first().map(|p| &p.name));
    let first_t = name_of(net.transitions.first().map(|t| &t.name));
    let head = head_payload(&net.name, net.version, &first_p, &first_t, codec);
    let mut out = vec![node(HEAD.into(), head)];
    for (i, p) in net.places.iter().enumerate() {
        let next = name_of(net.places.get(i + 1).map(|p| &p.name));
        out.push(node(place_label(&p.name), place_payload(p, &next, codec)));
    }
    for (i, t) in net.transitions.iter().enumerate() {
        let next = name_of(net.transitions.get(i + 1).map(|t| &t.name));
        out.push(node(
            transition_label(&t.name),
            transition_payload(t, &next, codec),
        ));
    }
    out
}

/// The head payload `inline_nodes` writes under `codec` (not `Lines`,
/// whose head is E446's line with edges): the name and version, then the
/// first place's and first transition's names ("" for none).
pub(in crate::net) fn head_payload(
    name: &str,
    version: u32,
    first_p: &str,
    first_t: &str,
    codec: InlineCodec,
) -> String {
    match codec {
        InlineCodec::Compact => format!(
            "{}{name} {version}\n{first_p}\n{first_t}",
            COMPACT_HEAD as char
        ),
        _ => format!("net {name} {version}\n{first_p}\n{first_t}"),
    }
}

/// The payload `inline_nodes` writes for place `p` followed by the place
/// named `next` ("" for none), under `codec` (not `Lines`).
pub(in crate::net) fn place_payload(p: &Place, next: &str, codec: InlineCodec) -> String {
    match codec {
        InlineCodec::Compact => format!("{}\n{next}", compact_place(p)),
        _ => format!("{}{next}", place_line(p)),
    }
}

/// The payload `inline_nodes` writes for transition `t` followed by the
/// transition named `next` ("" for none), under `codec` (not `Lines`).
pub(in crate::net) fn transition_payload(t: &Transition, next: &str, codec: InlineCodec) -> String {
    match codec {
        InlineCodec::Compact => format!("{}\n{next}", compact_transition(t)),
        _ => format!("{}{next}", transition_line(t)),
    }
}

/// The codec a head node's payload was written with.
pub(super) fn codec_of(head: &[u8]) -> Result<InlineCodec, Error> {
    if head.first() == Some(&COMPACT_HEAD) {
        return Ok(InlineCodec::Compact);
    }
    let end = head.iter().position(|&b| b == b'\n').ok_or_else(bad)?;
    Ok(match end + 1 == head.len() {
        true => InlineCodec::Lines,
        false => InlineCodec::LinesNext,
    })
}

fn parse_arcs(s: &str) -> Result<Vec<Arc>, Error> {
    if s.is_empty() {
        return Ok(vec![]);
    }
    s.split(',')
        .map(|a| {
            let (weight, place) = match a.split_once('*') {
                Some((w, p)) => (w.parse::<u32>().map_err(|_| bad())?, p),
                None => (1, a),
            };
            Ok(Arc {
                place: place.into(),
                weight,
            })
        })
        .collect()
}

/// What an inline node declares and links to. `net_fragments` writes the
/// net it reads back and refuses nodes it would not write, so one element
/// has one payload.
pub(in crate::net) fn read_inline(n: &Node, bytes: &[u8]) -> Result<Read, Error> {
    let text = std::str::from_utf8(bytes).map_err(|_| bad())?;
    let label = n.label.as_str();
    let mut links = BTreeMap::new();
    let mut link = |edge: &str, prefix: &str, name: &str| {
        if !name.is_empty() {
            links.insert(edge.to_string(), format!("{prefix}{name}"));
        }
    };
    // E446's line with its edges: read as E446 reads it.
    if !n.edges.is_empty() || (label == HEAD && codec_of(bytes)? == InlineCodec::Lines) {
        for e in &n.edges {
            let (l, to) = e.label.split_once(' ').ok_or_else(bad)?;
            if crate::node_key(to) != e.to || links.insert(l.into(), to.into()).is_some() {
                return Err(bad());
            }
        }
        return Ok(Read {
            element: parse_fragment(label, text)?,
            links,
        });
    }
    let element = if label == HEAD {
        let (first, rest) = text.split_once('\n').ok_or_else(bad)?;
        let (fp, ft) = rest.split_once('\n').ok_or_else(bad)?;
        link(FIRST_PLACE, "p/", fp);
        link(FIRST_TRANSITION, "t/", ft);
        match first.strip_prefix(COMPACT_HEAD as char) {
            Some(h) => {
                let (name, version) = h.split_once(' ').ok_or_else(bad)?;
                Element::Head(name.into(), version.parse().map_err(|_| bad())?)
            }
            None => parse_fragment(label, &format!("{first}\n"))?,
        }
    } else {
        let (body, next) = text.rsplit_once('\n').ok_or_else(bad)?;
        let (prefix, name) = label.split_at_checked(2).ok_or_else(bad)?;
        link(NEXT, prefix, next);
        let compact = !body.starts_with("place ") && !body.starts_with("transition ");
        match (compact, prefix) {
            (false, _) => parse_fragment(label, &format!("{body}\n"))?,
            (true, "p/") => Element::Place(Place {
                name: name.into(),
                colored: body == "c",
                initial: match body {
                    "" | "c" => 0,
                    k => k.parse().map_err(|_| bad())?,
                },
            }),
            (true, "t/") => {
                let (head, arcs) = body.split_once(':').ok_or_else(bad)?;
                let (kind, cap) = match head.split_once(' ') {
                    Some((k, c)) => (k, Some(c.to_string())),
                    None => (head, None),
                };
                let [k] = kind.as_bytes() else {
                    return Err(bad());
                };
                let (ins, outs) = arcs.split_once('>').ok_or_else(bad)?;
                Element::Transition(Transition {
                    name: name.into(),
                    inputs: parse_arcs(ins)?,
                    outputs: parse_arcs(outs)?,
                    kind: letter_kind(*k).ok_or_else(bad)?,
                    capability: cap,
                })
            }
            _ => return Err(bad()),
        }
    };
    Ok(Read { element, links })
}
