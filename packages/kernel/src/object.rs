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

/// A selector: all of a content's bytes, or a byte range of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Arg {
    Whole(Id),
    Range { id: Id, start: u64, len: u64 },
}

impl Arg {
    pub fn id(&self) -> Id {
        match self {
            Arg::Whole(id) | Arg::Range { id, .. } => *id,
        }
    }
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
                    Target::None => None,
                    Target::Content(a) => Some((label(&n.label, a), a.id())),
                    Target::Object(id) => Some((n.label.clone(), *id)),
                })
                .collect(),
            Object::Graph(Graph::Children(_, children)) => children.clone(),
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

fn encode_graph(out: &mut Vec<u8>, g: &Graph) {
    out.push(b'G');
    match g {
        Graph::Nodes(nodes) => {
            out.push(0);
            put_u32(out, nodes.len());
            for n in nodes {
                put_str(out, &n.label);
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
                }
                put_u32(out, n.edges.len());
                for e in &n.edges {
                    out.extend_from_slice(&e.to);
                    put_str(out, &e.label);
                }
            }
        }
        Graph::Children(level, children) => {
            out.push(*level);
            put_u32(out, children.len());
            for (first, id) in children {
                put_str(out, first);
                out.extend_from_slice(id.as_bytes());
            }
        }
    }
}

fn decode_graph(r: &mut Reader) -> Result<Graph, Error> {
    let g = match r.take(1)?[0] {
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
    let ascending = match &g {
        Graph::Nodes(n) => n.windows(2).all(|w| w[0].label < w[1].label),
        Graph::Children(_, c) => c.windows(2).all(|w| w[0].0 < w[1].0),
    };
    if !ascending {
        return Err(Error::Decode("graph labels not strictly ascending"));
    }
    Ok(g)
}

/// Edge label for a selector, keeping range offsets visible.
fn label(name: &str, a: &Arg) -> String {
    match a {
        Arg::Whole(_) => name.to_string(),
        Arg::Range { start, len, .. } => format!("{name}[{start}..{}]", start + len),
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
