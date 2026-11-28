//! L0 objects and their canonical encoding.
//!
//! Encoding (u32/u64 are big-endian):
//!   blob       = "B" bytes                     (storage variant "Z" zstd(bytes), same id)
//!   tree       = "T" count:u32 { name_len:u32 name kind("F"|"T") id[32] }   names strictly ascending
//!   commit     = "C" tree[32] nparents:u32 { parent[32] } msg_len:u32 msg
//!   derivation = "D" output[32] func[32] nargs:u32 { arg }
//!   arg        = 0 id[32] | 1 id[32] start:u64 len:u64              (whole content | byte range)
//!   claim      = "L" subject:arg pred_len:u32 predicate object:arg gain_bits:i64 by:(0 | 1 id[32])
//!   graph      = "G" level:u8 count:u32 { node (level 0) | label_len:u32 first_label id[32] (above) }
//!   node       = label_len:u32 label target nedges:u32 { key[8] label_len:u32 label }
//!   target     = 0 | 1 arg | 2 id[32]                  (none | content | another object)
//!              | 3 len:varint bytes                    (inline bytes, E447; compact chunks only)
//!   graph derivation (E452, an option)
//!              = "H" output[32] func[32] nargs:u32 { arg | 2 id[32] }   (output and `2` args are graph roots)
//!
//! A graph is a prolly tree of `G` chunks (see `graph.rs`): nodes sorted by
//! label, each with its outgoing edges, which name their target node by
//! `node_key(label)`, so edges may form cycles. Tree entries of kind "G"
//! name a graph's root.
//!
//! A file's identity is the id of its blob form, `Id::of_content(bytes)`.
//! Trees, commits and derivations are named by `BLAKE3(encoding)`.
//!
//! A derivation is a ledger entry: `output = func(args)`, verified by
//! reproducing the output's exact bytes before it is recorded. One output may
//! have many derivations. Whether an output's bytes are also stored is a
//! separate, cost-driven choice, so fragments can be values (stored bytes),
//! references (byte ranges of other content) or function applications, and
//! all of them name the same bytes.
//! Decoding is strict: `encode(decode(b)) == b` whenever it succeeds.

use crate::group::{put_varint, varint};
use crate::{Error, Id};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    File,
    Tree,
    Graph,
}

/// What a graph node stands for, if anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    None,
    /// Content, or a byte range of it: a semantic chunk.
    Content(Arg),
    /// Another object: a tree, graph, commit or derivation record.
    Object(Id),
    /// Bytes kept in the node itself (E447, opt-in: only a caller that
    /// builds such a node writes one). A small payload costs its bytes in
    /// the chunk, compressed with its neighbours, instead of a content of
    /// its own and a 32-byte id; gc and fsck have nothing to follow.
    Inline(Vec<u8>),
}

/// An edge to the node whose label has key `to` (see `node_key`).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    pub to: [u8; 8],
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub label: String,
    pub target: Target,
    pub edges: Vec<Edge>,
}

/// One chunk of a graph's prolly tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Graph {
    /// Level 0: nodes, strictly ascending by label.
    Nodes(Vec<Node>),
    /// Level > 0: children `(first label, chunk id)`, strictly ascending.
    Children(u8, Vec<(String, Id)>),
}

/// The key edges use to name the node labelled `label`.
pub fn node_key(label: &str) -> [u8; 8] {
    blake3::hash(label.as_bytes()).as_bytes()[..8]
        .try_into()
        .unwrap()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub name: String,
    pub kind: Kind,
    pub id: Id,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub tree: Id,
    pub parents: Vec<Id>,
    pub message: String,
}

/// A selector: all of a content's bytes, or a byte range of them; or
/// (E452, an option) a graph root, an object, which only a
/// [`GraphDerivation`] may take: content records (derivations, runs,
/// claims, node targets) refuse it, so their encodings are unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Arg {
    Whole(Id),
    Range {
        id: Id,
        start: u64,
        len: u64,
    },
    /// A graph's root (E452): gc keeps the whole graph through it.
    Graph(Id),
}

impl Arg {
    pub fn id(&self) -> Id {
        match self {
            Arg::Whole(id) | Arg::Range { id, .. } | Arg::Graph(id) => *id,
        }
    }

    /// True for a graph root, false for content.
    pub fn is_graph(&self) -> bool {
        matches!(self, Arg::Graph(_))
    }
}

/// The error for a graph root where content is needed (E452).
pub(crate) fn graph_not_content(id: Id) -> Error {
    Error::Exec(format!("{id}: a graph argument is not content"))
}

/// E452 (an option): `output = func(args)` where `output` is a graph's root
/// and args may be graph roots ([`Arg::Graph`]) as well as content. It is
/// kept outside the content ledger, so a graph root is never taken for
/// content; it is a memo (the key is `Derivation::key(func, args)`) and a
/// fact a rebuild checks by recomputing in full and comparing roots. gc
/// keeps a record while its output graph is live, and with it every graph
/// and content it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphDerivation {
    pub output: Id,
    pub func: Id,
    pub args: Vec<Arg>,
}

/// `output = func(args)`, recorded only after it has been verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Derivation {
    pub output: Id,
    pub func: Id,
    pub args: Vec<Arg>,
}

impl Derivation {
    /// Identity of the computation alone (func and args, not output): the
    /// memo key for pure functions.
    pub fn key(func: &Id, args: &[Arg]) -> Id {
        let mut out = vec![b'K'];
        out.extend_from_slice(func.as_bytes());
        put_args(&mut out, args);
        Id::of(&out)
    }
}

/// A recorded step (E441): `output` was made by `func(args)` and its bytes
/// are kept. Unlike a [`Derivation`], it is never a way to rebuild the
/// output: the step may use floats freely, a seed, or work outside the
/// kernel (a command, an agent), so a rerun is a check that reports same or
/// different, and the first output stays the one of record. `check`, when
/// set, names the step that judges a rerun that differs:
/// `verdict = check.func(first, now, check.args...)`, itself a derivation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub output: Id,
    pub func: Id,
    pub args: Vec<Arg>,
    pub check: Option<Check>,
}

/// The step a rerun's output is judged by (see [`Run`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub func: Id,
    pub args: Vec<Arg>,
}

/// An unverified semantic relation between two selections of content:
/// `subject predicate object`, e.g. "bytes 0..400 of report mentions
/// <entity>". The predicate is free text. `gain_bits` is measured by the
/// kernel, not asserted: how many bits knowing the object saves when
/// compressing the subject. `by` names who asserted it (e.g. an extractor
/// function), if known. Claims never take part in reconstruction; one can
/// be promoted to a derivation once a verified function reproduces it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    pub subject: Arg,
    pub predicate: String,
    pub object: Arg,
    pub gain_bits: i64,
    pub by: Option<Id>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Object {
    Blob(Vec<u8>),
    Tree(Vec<TreeEntry>),
    Commit(Commit),
    Derivation(Derivation),
    Claim(Claim),
    Graph(Graph),
    Run(Run),
    /// E452, an option: see [`GraphDerivation`].
    GraphDerivation(GraphDerivation),
}

impl Object {
    /// Build a tree, sorting entries; fails on duplicate or invalid names.
    pub fn tree(mut entries: Vec<TreeEntry>) -> Result<Object, Error> {
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let tree = Object::Tree(entries);
        Object::decode(&tree.encode())?;
        Ok(tree)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Object::Blob(bytes) => {
                out.push(b'B');
                out.extend_from_slice(bytes);
            }
            Object::Tree(entries) => {
                out.push(b'T');
                put_u32(&mut out, entries.len());
                for e in entries {
                    put_str(&mut out, &e.name);
                    out.push(match e.kind {
                        Kind::File => b'F',
                        Kind::Tree => b'T',
                        Kind::Graph => b'G',
                    });
                    out.extend_from_slice(e.id.as_bytes());
                }
            }
            Object::Commit(c) => {
                out.push(b'C');
                out.extend_from_slice(c.tree.as_bytes());
                put_u32(&mut out, c.parents.len());
                c.parents
                    .iter()
                    .for_each(|p| out.extend_from_slice(p.as_bytes()));
                put_str(&mut out, &c.message);
            }
            Object::Derivation(d) => {
                out.push(b'D');
                out.extend_from_slice(d.output.as_bytes());
                out.extend_from_slice(d.func.as_bytes());
                put_args(&mut out, &d.args);
            }
            Object::Claim(c) => {
                out.push(b'L');
                put_arg(&mut out, &c.subject);
                put_str(&mut out, &c.predicate);
                put_arg(&mut out, &c.object);
                out.extend_from_slice(&c.gain_bits.to_be_bytes());
                match c.by {
                    None => out.push(0),
                    Some(by) => {
                        out.push(1);
                        out.extend_from_slice(by.as_bytes());
                    }
                }
            }
            Object::Graph(g) => encode_graph(&mut out, g),
            Object::GraphDerivation(d) => {
                out.push(b'H');
                out.extend_from_slice(d.output.as_bytes());
                out.extend_from_slice(d.func.as_bytes());
                put_args(&mut out, &d.args);
            }
            Object::Run(r) => {
                out.push(b'E');
                out.extend_from_slice(r.output.as_bytes());
                out.extend_from_slice(r.func.as_bytes());
                put_args(&mut out, &r.args);
                match &r.check {
                    None => out.push(0),
                    Some(c) => {
                        out.push(1);
                        out.extend_from_slice(c.func.as_bytes());
                        put_args(&mut out, &c.args);
                    }
                }
            }
        }
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Object, Error> {
        let mut r = Reader(bytes);
        let obj = match r.take(1)?[0] {
            b'B' => Object::Blob(r.rest().to_vec()),
            b'T' => decode_tree(&mut r)?,
            b'C' => Object::Commit(Commit {
                tree: r.id()?,
                parents: r.many(Reader::id)?,
                message: r.string()?,
            }),
            b'D' => Object::Derivation(Derivation {
                output: r.id()?,
                func: r.id()?,
                args: r.many(Reader::arg)?,
            }),
            b'L' => Object::Claim(Claim {
                subject: r.arg()?,
                predicate: r.string()?,
                object: r.arg()?,
                gain_bits: r.u64()? as i64,
                by: match r.take(1)?[0] {
                    0 => None,
                    1 => Some(r.id()?),
                    _ => return Err(Error::Decode("unknown claim author tag")),
                },
            }),
            b'G' => Object::Graph(decode_graph(&mut r)?),
            b'H' => Object::GraphDerivation(GraphDerivation {
                output: r.id()?,
                func: r.id()?,
                args: r.many(Reader::arg_or_graph)?,
            }),
            b'E' => Object::Run(Run {
                output: r.id()?,
                func: r.id()?,
                args: r.many(Reader::arg)?,
                check: match r.take(1)?[0] {
                    0 => None,
                    1 => Some(Check {
                        func: r.id()?,
                        args: r.many(Reader::arg)?,
                    }),
                    _ => return Err(Error::Decode("unknown run check tag")),
                },
            }),
            _ => return Err(Error::Decode("unknown object tag")),
        };
        r.finish()?;
        Ok(obj)
    }

    pub fn id(&self) -> Id {
        Id::of(&self.encode())
    }

    /// Outgoing edges `(label, target)`. Range selectors keep their offsets
    /// in the label, so the graph says which part of a source is used.
    pub fn links(&self) -> Vec<(String, Id)> {
        match self {
            Object::Blob(_) => vec![],
            Object::Tree(entries) => entries.iter().map(|e| (e.name.clone(), e.id)).collect(),
            Object::Commit(c) => std::iter::once(("tree".to_string(), c.tree))
                .chain(c.parents.iter().map(|p| ("parent".to_string(), *p)))
                .collect(),
            Object::Derivation(d) => {
                let mut links = vec![
                    ("output".to_string(), d.output),
                    ("func".to_string(), d.func),
                ];
                for (i, a) in d.args.iter().enumerate() {
                    links.push((label(&format!("arg{i}"), a), a.id()));
                }
                links
            }
            Object::Claim(c) => {
                let mut links = vec![
                    (label("subject", &c.subject), c.subject.id()),
                    (
                        label(&format!("object ({})", c.predicate), &c.object),
                        c.object.id(),
                    ),
                ];
                links.extend(c.by.map(|by| ("by".to_string(), by)));
                links
            }
            // Node targets and child chunks; edges are internal (by key).
            Object::Graph(Graph::Nodes(nodes)) => nodes
                .iter()
                .filter_map(|n| match &n.target {
                    Target::None | Target::Inline(_) => None,
                    Target::Content(a) => Some((label(&n.label, a), a.id())),
                    Target::Object(id) => Some((n.label.clone(), *id)),
                })
                .collect(),
            Object::Graph(Graph::Children(_, children)) => children.clone(),
            Object::GraphDerivation(d) => {
                let mut links = vec![
                    ("output".to_string(), d.output),
                    ("func".to_string(), d.func),
                ];
                for (i, a) in d.args.iter().enumerate() {
                    links.push((label(&format!("arg{i}"), a), a.id()));
                }
                links
            }
            Object::Run(r) => {
                let mut links = vec![
                    ("output".to_string(), r.output),
                    ("func".to_string(), r.func),
                ];
                for (i, a) in r.args.iter().enumerate() {
                    links.push((label(&format!("arg{i}"), a), a.id()));
                }
                if let Some(c) = &r.check {
                    links.push(("check".to_string(), c.func));
                    for (i, a) in c.args.iter().enumerate() {
                        links.push((label(&format!("check{i}"), a), a.id()));
                    }
                }
                links
            }
        }
    }
}

fn decode_tree(r: &mut Reader) -> Result<Object, Error> {
    let entries = r.many(|r| {
        let name = r.string()?;
        let kind = match r.take(1)?[0] {
            b'F' => Kind::File,
            b'T' => Kind::Tree,
            b'G' => Kind::Graph,
            _ => return Err(Error::Decode("unknown entry kind")),
        };
        Ok(TreeEntry {
            name,
            kind,
            id: r.id()?,
        })
    })?;
    for e in &entries {
        if e.name.is_empty() || e.name == "." || e.name == ".." || e.name.contains(['/', '\0']) {
            return Err(Error::InvalidName(e.name.clone()));
        }
    }
    if entries.windows(2).any(|w| w[0].name >= w[1].name) {
        return Err(Error::Decode("tree names not strictly ascending"));
    }
    Ok(Object::Tree(entries))
}

/// Graph chunks (since E045) set `COMPACT` in the level byte: counts and
/// lengths are varints, and node labels, child labels and edge labels are
/// front-coded against the previous one in the chunk (shared prefix
/// length, then the rest). Labels are sorted, so neighbours share long
/// prefixes; edge labels repeat. Chunks without the flag (before E045)
/// still decode.
const COMPACT: u8 = 0x80;

fn put_front(out: &mut Vec<u8>, prev: &str, s: &str) {
    let mut shared = prev
        .bytes()
        .zip(s.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !s.is_char_boundary(shared) {
        shared -= 1;
    }
    put_varint(out, shared);
    put_varint(out, s.len() - shared);
    out.extend_from_slice(&s.as_bytes()[shared..]);
}

fn encode_graph(out: &mut Vec<u8>, g: &Graph) {
    out.push(b'G');
    let mut prev_edge = "";
    match g {
        Graph::Nodes(nodes) => {
            out.push(COMPACT);
            put_varint(out, nodes.len());
            for (i, n) in nodes.iter().enumerate() {
                put_front(out, if i == 0 { "" } else { &nodes[i - 1].label }, &n.label);
                match &n.target {
                    Target::None => out.push(0),
                    Target::Content(a) => {
                        out.push(1);
                        put_arg(out, a);
                    }
                    Target::Object(id) => {
                        out.push(2);
                        out.extend_from_slice(id.as_bytes());
                    }
                    Target::Inline(bytes) => {
                        out.push(3);
                        put_varint(out, bytes.len());
                        out.extend_from_slice(bytes);
                    }
                }
                put_varint(out, n.edges.len());
                for e in &n.edges {
                    out.extend_from_slice(&e.to);
                    put_front(out, prev_edge, &e.label);
                    prev_edge = &e.label;
                }
            }
        }
        Graph::Children(level, children) => {
            out.push(COMPACT | level);
            put_varint(out, children.len());
            for (i, (first, id)) in children.iter().enumerate() {
                put_front(out, if i == 0 { "" } else { &children[i - 1].0 }, first);
                out.extend_from_slice(id.as_bytes());
            }
        }
    }
}

fn decode_graph(r: &mut Reader) -> Result<Graph, Error> {
    let tag = r.take(1)?[0];
    if tag & COMPACT != 0 {
        return check_graph(decode_compact(r, tag & !COMPACT)?);
    }
    let g = match tag {
        0 => Graph::Nodes(r.many(|r| {
            let label = r.string()?;
            let target = match r.take(1)?[0] {
                0 => Target::None,
                1 => Target::Content(r.arg()?),
                2 => Target::Object(r.id()?),
                _ => return Err(Error::Decode("unknown graph target")),
            };
            let edges = r.many(|r| {
                let to = r.take(8)?.try_into().unwrap();
                Ok(Edge {
                    to,
                    label: r.string()?,
                })
            })?;
            if edges.windows(2).any(|w| w[0] >= w[1]) {
                return Err(Error::Decode("graph edges not strictly ascending"));
            }
            Ok(Node {
                label,
                target,
                edges,
            })
        })?),
        level => {
            let children = r.many(|r| Ok((r.string()?, r.id()?)))?;
            if children.is_empty() {
                return Err(Error::Decode("empty graph chunk"));
            }
            Graph::Children(level, children)
        }
    };
    check_graph(g)
}

/// Labels strictly ascending, as every chunk keeps them.
fn check_graph(g: Graph) -> Result<Graph, Error> {
    let ascending = match &g {
        Graph::Nodes(n) => n.windows(2).all(|w| w[0].label < w[1].label),
        Graph::Children(_, c) => c.windows(2).all(|w| w[0].0 < w[1].0),
    };
    if !ascending {
        return Err(Error::Decode("graph labels not strictly ascending"));
    }
    Ok(g)
}

fn decode_compact(r: &mut Reader, level: u8) -> Result<Graph, Error> {
    let n = r.varint()?;
    let (mut prev, mut prev_edge) = (String::new(), String::new());
    if level == 0 {
        let mut nodes = Vec::with_capacity(n.min(r.0.len()));
        for _ in 0..n {
            let label = r.front(&prev)?;
            let target = r.target()?;
            let m = r.varint()?;
            let mut edges = Vec::with_capacity(m.min(r.0.len()));
            for _ in 0..m {
                let to = r.take(8)?.try_into().unwrap();
                prev_edge = r.front(&prev_edge)?;
                edges.push(Edge {
                    to,
                    label: prev_edge.clone(),
                });
            }
            if edges.windows(2).any(|w| w[0] >= w[1]) {
                return Err(Error::Decode("graph edges not strictly ascending"));
            }
            prev.clone_from(&label);
            nodes.push(Node {
                label,
                target,
                edges,
            });
        }
        return Ok(Graph::Nodes(nodes));
    }
    let mut children = Vec::with_capacity(n.min(r.0.len()));
    for _ in 0..n {
        let first = r.front(&prev)?;
        prev.clone_from(&first);
        children.push((first, r.id()?));
    }
    if children.is_empty() {
        return Err(Error::Decode("empty graph chunk"));
    }
    Ok(Graph::Children(level, children))
}

/// What `graph_scan` found in one graph chunk.
pub(crate) enum Found {
    /// A leaf chunk: its nodes that `keep` kept.
    Nodes(Vec<Node>),
    /// An inner chunk: the last child whose first label is at most `label`.
    Child(Option<Id>),
}

/// Read a graph chunk's encoding for a lookup (E335): checked as
/// `Object::decode` checks it, the same errors for the same bytes, but
/// only the nodes `keep` keeps are built, and an inner chunk gives the
/// child to follow for `label`. Decoding a whole chunk built every node's
/// label and edges to keep one of them. `None` for anything but a compact
/// graph chunk (before E045, or not a graph); decode those.
pub(crate) fn graph_scan(
    encoded: &[u8],
    keep: &dyn Fn(&str) -> bool,
    label: &str,
) -> Result<Option<Found>, Error> {
    use std::cmp::Ordering::Greater;
    let mut r = Reader(encoded);
    match encoded {
        [b'G', tag, ..] if tag & COMPACT != 0 => r.take(2)?,
        _ => return Ok(None),
    };
    let level = encoded[1] & !COMPACT;
    let n = r.varint()?;
    let mut prev: Vec<u8> = Vec::new();
    let mut sorted = true;
    let found = if level == 0 {
        let (mut prev_edge, mut kept) = (Vec::new(), Vec::new());
        for i in 0..n {
            let ord = r.front_step(&mut prev)?;
            sorted &= i == 0 || ord == Greater;
            let target = r.target()?;
            // Valid UTF-8: `front_step` checked it.
            let name = std::str::from_utf8(&prev).unwrap_or_default();
            let mine = keep(name);
            let m = r.varint()?;
            let mut edges = Vec::new();
            let mut ascending = true;
            let mut prev_to = [0u8; 8];
            for j in 0..m {
                let to: [u8; 8] = r.take(8)?.try_into().unwrap();
                let ord = r.front_step(&mut prev_edge)?;
                ascending &= j == 0 || to.cmp(&prev_to).then(ord) == Greater;
                prev_to = to;
                if mine {
                    edges.push(Edge {
                        to,
                        label: String::from_utf8(prev_edge.clone())
                            .map_err(|_| Error::Decode("string not utf-8"))?,
                    });
                }
            }
            if !ascending {
                return Err(Error::Decode("graph edges not strictly ascending"));
            }
            if mine {
                kept.push(Node {
                    label: name.to_string(),
                    target,
                    edges,
                });
            }
        }
        Found::Nodes(kept)
    } else {
        if n == 0 {
            return Err(Error::Decode("empty graph chunk"));
        }
        let mut child = None;
        for i in 0..n {
            let ord = r.front_step(&mut prev)?;
            sorted &= i == 0 || ord == Greater;
            let id = r.id()?;
            if prev.as_slice() <= label.as_bytes() {
                child = Some(id);
            }
        }
        Found::Child(child)
    };
    if !sorted {
        return Err(Error::Decode("graph labels not strictly ascending"));
    }
    r.finish()?;
    Ok(Some(found))
}

/// Edge label for a selector, keeping range offsets visible.
fn label(name: &str, a: &Arg) -> String {
    match a {
        Arg::Whole(_) => name.to_string(),
        Arg::Range { start, len, .. } => format!("{name}[{start}..{}]", start + len),
        Arg::Graph(_) => format!("{name} (graph)"),
    }
}

fn put_args(out: &mut Vec<u8>, args: &[Arg]) {
    put_u32(out, args.len());
    args.iter().for_each(|a| put_arg(out, a));
}

fn put_arg(out: &mut Vec<u8>, a: &Arg) {
    match a {
        Arg::Whole(id) => {
            out.push(0);
            out.extend_from_slice(id.as_bytes());
        }
        Arg::Range { id, start, len } => {
            out.push(1);
            out.extend_from_slice(id.as_bytes());
            out.extend_from_slice(&start.to_be_bytes());
            out.extend_from_slice(&len.to_be_bytes());
        }
        Arg::Graph(id) => {
            out.push(2);
            out.extend_from_slice(id.as_bytes());
        }
    }
}

fn put_u32(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&u32::try_from(n).expect("length exceeds u32").to_be_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len());
    out.extend_from_slice(s.as_bytes());
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let (head, tail) = self
            .0
            .split_at_checked(n)
            .ok_or(Error::Decode("truncated"))?;
        self.0 = tail;
        Ok(head)
    }

    fn rest(&mut self) -> &'a [u8] {
        std::mem::take(&mut self.0)
    }

    fn u32(&mut self) -> Result<usize, Error> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize)
    }

    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn id(&mut self) -> Result<Id, Error> {
        Ok(Id::from_bytes(self.take(32)?.try_into().unwrap()))
    }

    fn arg(&mut self) -> Result<Arg, Error> {
        match self.take(1)?[0] {
            0 => Ok(Arg::Whole(self.id()?)),
            1 => {
                let (id, start, len) = (self.id()?, self.u64()?, self.u64()?);
                if len == 0 || start.checked_add(len).is_none() {
                    return Err(Error::Decode("empty or overflowing range"));
                }
                Ok(Arg::Range { id, start, len })
            }
            _ => Err(Error::Decode("unknown arg tag")),
        }
    }

    /// An argument of a graph derivation (E452): content, or a graph root.
    fn arg_or_graph(&mut self) -> Result<Arg, Error> {
        match self.0.first() {
            Some(2) => {
                self.take(1)?;
                Ok(Arg::Graph(self.id()?))
            }
            _ => self.arg(),
        }
    }

    fn varint(&mut self) -> Result<usize, Error> {
        varint(&mut self.0).ok_or(Error::Decode("bad varint"))
    }

    /// A compact chunk's node target, inline bytes (E447) included.
    fn target(&mut self) -> Result<Target, Error> {
        Ok(match self.take(1)?[0] {
            0 => Target::None,
            1 => Target::Content(self.arg()?),
            2 => Target::Object(self.id()?),
            3 => {
                let n = self.varint()?;
                Target::Inline(self.take(n)?.to_vec())
            }
            _ => return Err(Error::Decode("unknown graph target")),
        })
    }

    /// A front-coded string (see `put_front`) after `prev`.
    fn front(&mut self, prev: &str) -> Result<String, Error> {
        let shared = self.varint()?;
        let rest = self.varint()?;
        let head = prev
            .as_bytes()
            .get(..shared)
            .ok_or(Error::Decode("bad front coding"))?;
        let bytes = [head, self.take(rest)?].concat();
        String::from_utf8(bytes).map_err(|_| Error::Decode("string not utf-8"))
    }

    /// `front`, in place: the string after `prev` replaces it, and the
    /// result says how the new string compares with the old.
    fn front_step(&mut self, prev: &mut Vec<u8>) -> Result<std::cmp::Ordering, Error> {
        let shared = self.varint()?;
        let rest = self.varint()?;
        if shared > prev.len() {
            return Err(Error::Decode("bad front coding"));
        }
        let rest = self.take(rest)?;
        // Both share `prev[..shared]`, so the rests decide.
        let ord = rest.cmp(&prev[shared..]);
        prev.truncate(shared);
        prev.extend_from_slice(rest);
        std::str::from_utf8(prev).map_err(|_| Error::Decode("string not utf-8"))?;
        Ok(ord)
    }

    fn string(&mut self) -> Result<String, Error> {
        let len = self.u32()?;
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| Error::Decode("string not utf-8"))
    }

    fn many<T>(
        &mut self,
        mut item: impl FnMut(&mut Self) -> Result<T, Error>,
    ) -> Result<Vec<T>, Error> {
        let count = self.u32()?;
        let mut out = Vec::with_capacity(count.min(self.0.len() / 32));
        for _ in 0..count {
            out.push(item(self)?);
        }
        Ok(out)
    }

    fn finish(self) -> Result<(), Error> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(Error::Decode("trailing bytes"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// E335: `graph_scan` keeps the same nodes, follows the same child and
    /// fails with the same error as decoding the whole chunk, on chunks and
    /// on every one-byte change and cut of them.
    #[test]
    fn graph_scan_answers_as_decode() {
        let mut seed = 7u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        let labels: Vec<String> = {
            let mut v: Vec<String> = (0..40)
                .map(|i| format!("k/{:03}é{}", i * 7, i % 3))
                .collect();
            v.sort();
            v
        };
        let nodes: Vec<Node> = labels
            .iter()
            .map(|l| {
                let mut edges: Vec<Edge> = (0..next() % 4)
                    .map(|_| Edge {
                        to: crate::node_key(&labels[next() % labels.len()]),
                        label: ["a", "ab", "b"][next() % 3].to_string(),
                    })
                    .collect();
                edges.sort();
                edges.dedup();
                Node {
                    label: l.clone(),
                    target: match next() % 3 {
                        0 => Target::None,
                        1 => Target::Content(Arg::Whole(Id::of(l.as_bytes()))),
                        _ => Target::Object(Id::of(&[1])),
                    },
                    edges,
                }
            })
            .collect();
        let children: Vec<(String, Id)> = labels
            .iter()
            .step_by(5)
            .map(|l| (l.clone(), Id::of(l.as_bytes())))
            .collect();
        let probes: Vec<String> = labels
            .iter()
            .flat_map(|l| [l.clone(), format!("{l}~"), l[..l.len() - 1].to_string()])
            .chain(["".to_string(), "zzz".to_string()])
            .collect();
        for g in [Graph::Nodes(nodes), Graph::Children(2, children)] {
            let enc = Object::Graph(g.clone()).encode();
            let mut cases = vec![enc.clone()];
            for i in 0..enc.len() {
                let mut e = enc.clone();
                e[i] ^= 1 << (next() % 8);
                cases.push(e);
                cases.push(enc[..i].to_vec());
            }
            for e in &cases {
                let want = Object::decode(e);
                for p in probes.iter().step_by(if e == &enc { 1 } else { 17 }) {
                    let keep = |l: &str| l == p || l.ends_with('1');
                    let Some(got) = graph_scan(e, &keep, p).transpose() else {
                        assert!(!matches!(want, Ok(Object::Graph(_)) if e[1] & COMPACT != 0));
                        continue;
                    };
                    match (&want, got) {
                        (Ok(Object::Graph(Graph::Nodes(all))), Ok(Found::Nodes(kept))) => {
                            let expect: Vec<Node> =
                                all.iter().filter(|n| keep(&n.label)).cloned().collect();
                            assert_eq!(kept, expect);
                        }
                        (Ok(Object::Graph(Graph::Children(_, c))), Ok(Found::Child(got))) => {
                            let expect = c.iter().rev().find(|(f, _)| f.as_str() <= p.as_str());
                            assert_eq!(got, expect.map(|(_, id)| *id));
                        }
                        (Err(a), Err(b)) => assert_eq!(format!("{a:?}"), format!("{b:?}")),
                        (a, b) => panic!("decode {a:?}, scan {:?}", b.is_ok()),
                    }
                }
            }
        }
    }

    /// Graph chunks written before E045 (fixed-width counts and full
    /// labels) still decode to the same graph; the compact form round-trips
    /// and is smaller.
    #[test]
    fn graph_chunks_decode_in_both_layouts() {
        let node = |label: &str, to: &str| Node {
            label: label.into(),
            target: Target::None,
            edges: vec![Edge {
                to: crate::node_key(to),
                label: "imports".into(),
            }],
        };
        let g = Graph::Nodes(vec![
            node("packages/kernel/src/dict.rs", "rs:group"),
            node("packages/kernel/src/group.rs", "rs:dict"),
            node("packages/kernel/src/repack.rs", "rs:group"),
        ]);
        let mut legacy = vec![b'G', 0];
        let Graph::Nodes(nodes) = &g else {
            unreachable!()
        };
        put_u32(&mut legacy, nodes.len());
        for n in nodes {
            put_str(&mut legacy, &n.label);
            legacy.push(0);
            put_u32(&mut legacy, n.edges.len());
            for e in &n.edges {
                legacy.extend_from_slice(&e.to);
                put_str(&mut legacy, &e.label);
            }
        }
        assert_eq!(Object::decode(&legacy).unwrap(), Object::Graph(g.clone()));
        let compact = Object::Graph(g.clone()).encode();
        assert_eq!(Object::decode(&compact).unwrap(), Object::Graph(g));
        assert!(
            compact.len() < legacy.len(),
            "{} >= {}",
            compact.len(),
            legacy.len()
        );
    }
}
