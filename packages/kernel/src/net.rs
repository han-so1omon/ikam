//! Petri nets of work states (E269, opt-in, library only).
//!
//! The kernel plan lists scheduling as "Petri nets over derivations, for
//! approvals, budgets and retries", and the graph-objects plan notes that a
//! net fits a graph object as a bipartite graph with markings as content.
//! The Python-era engine (`modelado/plans/engine.py`, removed in a992b77)
//! enabled a transition by its input tokens and recorded each firing with
//! hashes of the marking before and after. This module carries the same
//! model on the database log:
//!
//! - A [`Net`] is places and transitions; arcs carry weights. It is stored
//!   with [`Repo::put_net`] as a graph object (one node per place and per
//!   transition, edges `in` and `out` from each transition to its places)
//!   and read back with [`Repo::net_at`].
//! - Tokens are named. A *colored* place holds named tokens (one per work
//!   item, such as an experiment id); a plain place holds anonymous ones
//!   (`_`), such as a budget or a slot. A firing binds at most one color: it
//!   takes the bound token from its colored inputs and puts it in its
//!   colored outputs, and takes and puts anonymous tokens elsewhere.
//! - The marking lives in a database ([`Db`]) under `net/<name>/m/<place>/
//!   <token>` as decimal counts. A firing is one `db_append`: the changed
//!   counts plus a record at `net/<name>/fire/<key>` holding the transition,
//!   the bound token and the marking hashes before and after. `key` is the
//!   caller's idempotency key, so a retried firing (the same key) changes
//!   nothing, and reusing a key for another firing is refused.
//! - [`Repo::net_replay`] recomputes the marking from the log's records
//!   alone; it must equal the stored one.
//! - Transitions have a [`Kind`]: `pure`, `cmd` and `agent` may be fired by
//!   a scheduler; `human` ones never are ([`Repo::net_enabled`] with
//!   `auto` leaves them out), so an owner's approval stays the owner's.
//!
//! Nothing in the default paths calls this module.

mod fragments;
mod table;
mod text;

pub use fragments::{ArcChange, Change, InlineCodec, Layout, Link, NetDelta};
pub use table::Table;

use std::collections::BTreeMap;

use crate::db::{Db, Event};
use crate::object::{Arg, Edge, Node, Target, node_key};
use crate::{Error, Id, Repo, Store};

/// The anonymous token of a plain place.
pub const ANON: &str = "_";

/// Who may fire a transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Deterministic: recomputed on replay (ranking, a keep rule).
    Pure,
    /// A command from the net's allowlist; its output is recorded.
    Cmd,
    /// An agent's work; its output is recorded, never regenerated.
    Agent,
    /// The owner's: never fired by a scheduler.
    Human,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Pure => "pure",
            Kind::Cmd => "cmd",
            Kind::Agent => "agent",
            Kind::Human => "human",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "pure" => Kind::Pure,
            "cmd" => Kind::Cmd,
            "agent" => Kind::Agent,
            "human" => Kind::Human,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    /// Holds named tokens (one per work item) rather than anonymous ones.
    pub colored: bool,
    /// Anonymous tokens at initialization (plain places only).
    pub initial: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arc {
    pub place: String,
    pub weight: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    pub name: String,
    pub inputs: Vec<Arc>,
    pub outputs: Vec<Arc>,
    pub kind: Kind,
    /// The capability a scheduler resolves to do the work (`bench`,
    /// `propose`); none for bookkeeping transitions and human ones.
    pub capability: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Net {
    pub name: String,
    pub version: u32,
    pub places: Vec<Place>,
    pub transitions: Vec<Transition>,
}

/// Tokens per place: place, then token, then count (no zero counts).
pub type Marking = BTreeMap<String, BTreeMap<String, u64>>;

/// One recorded firing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Firing {
    pub key: String,
    pub transition: String,
    pub token: Option<String>,
    pub before: Id,
    pub after: Id,
    /// The work's output, when the firing was made with `net_fire_output`
    /// (E271).
    pub output: Option<Id>,
    /// The step that made the output, when the firing was made with
    /// `net_fire_step` (E441): a derivation for a pure transition, a
    /// recorded step for a command or an agent.
    pub step: Option<Id>,
    /// The version of the net the firing ran under (E422): the id of the
    /// net's canonical text ([`Net::version_id`]), on a net whose database
    /// tracks versions ([`net_track_versions`](Repo::net_track_versions)).
    pub net: Option<Id>,
}

/// The work a firing runs (E441, `net_fire_step`).
#[derive(Clone, Debug)]
pub enum Step<'a> {
    /// For a `pure` transition: `func(args)`, applied as a derivation.
    Pure { func: Id, args: Vec<Arg> },
    /// For a `cmd` or `agent` transition: a recorded step. `output` is the
    /// work's bytes when it ran outside the kernel; without it the kernel
    /// runs `func` (a WASM module, floats pinned).
    Recorded {
        func: Id,
        args: Vec<Arg>,
        output: Option<&'a [u8]>,
        check: Option<crate::Check>,
    },
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
}

fn invalid(what: &str, name: &str) -> Error {
    Error::InvalidName(format!("{what} {name}"))
}

impl Net {
    pub fn place(&self, name: &str) -> Option<&Place> {
        self.places.iter().find(|p| p.name == name)
    }

    /// The id this version of the net goes by (E422): the content id of
    /// its canonical text ([`to_text`](Net::to_text)). Two nets with the
    /// same places and transitions have the same id, whatever comments or
    /// spacing a file holding them has.
    pub fn version_id(&self) -> Id {
        Id::of_content(self.to_text().as_bytes())
    }

    pub fn transition(&self, name: &str) -> Option<&Transition> {
        self.transitions.iter().find(|t| t.name == name)
    }

    /// Check the net: names valid and unique across places and transitions,
    /// every arc's place defined, weights at least 1, no colored initial
    /// tokens, every transition with an arc, and a human transition with no
    /// capability (the owner does the work). Arcs only join a transition to
    /// a place, so the net is bipartite by construction.
    pub fn check(&self) -> Result<(), Error> {
        if !valid_name(&self.name) {
            return Err(invalid("net", &self.name));
        }
        let mut seen = std::collections::BTreeSet::new();
        // Place names, so an arc's place is found in constant time (E446).
        let mut places = std::collections::HashSet::new();
        for p in &self.places {
            if !valid_name(&p.name) || !seen.insert(p.name.as_str()) {
                return Err(invalid("place", &p.name));
            }
            places.insert(p.name.as_str());
            if p.colored && p.initial > 0 {
                return Err(invalid("colored place with initial tokens", &p.name));
            }
        }
        for t in &self.transitions {
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

    /// Whether `t` binds a color: it touches a colored place.
    pub fn binds_color(&self, t: &Transition) -> bool {
        (t.inputs.iter().chain(&t.outputs)).any(|a| self.place(&a.place).is_some_and(|p| p.colored))
    }

    /// The initial marking: each plain place's initial tokens.
    pub fn initial_marking(&self) -> Marking {
        let mut m = Marking::new();
        for p in self.places.iter().filter(|p| p.initial > 0) {
            m.entry(p.name.clone())
                .or_default()
                .insert(ANON.into(), p.initial as u64);
        }
        m
    }

    /// Whether `t` may fire in `m` with `token` bound (None for a transition
    /// that binds no color; a source transition with colored outputs takes
    /// a new token).
    pub fn is_enabled(&self, m: &Marking, t: &Transition, token: Option<&str>) -> bool {
        if self.binds_color(t) != token.is_some() {
            return false;
        }
        t.inputs.iter().all(|a| {
            let colored = self.place(&a.place).is_some_and(|p| p.colored);
            let tok = if colored { token.unwrap_or(ANON) } else { ANON };
            count(m, &a.place, tok) >= a.weight as u64
        })
    }

    /// Fire `t` on `m` in memory: the new marking, or None when not enabled.
    pub fn fire(&self, m: &Marking, t: &Transition, token: Option<&str>) -> Option<Marking> {
        if !self.is_enabled(m, t, token) {
            return None;
        }
        let mut out = m.clone();
        let tok_for = |place: &str| {
            if self.place(place).is_some_and(|p| p.colored) {
                token.unwrap_or(ANON).to_string()
            } else {
                ANON.to_string()
            }
        };
        for a in &t.inputs {
            let tok = tok_for(&a.place);
            let e = out.get_mut(&a.place).unwrap();
            let c = e.get_mut(&tok).unwrap();
            *c -= a.weight as u64;
            if *c == 0 {
                e.remove(&tok);
                if e.is_empty() {
                    out.remove(&a.place);
                }
            }
        }
        for a in &t.outputs {
            let tok = tok_for(&a.place);
            *out.entry(a.place.clone())
                .or_default()
                .entry(tok)
                .or_default() += a.weight as u64;
        }
        Some(out)
    }
}

fn count(m: &Marking, place: &str, token: &str) -> u64 {
    m.get(place)
        .and_then(|p| p.get(token))
        .copied()
        .unwrap_or(0)
}

/// The canonical bytes of a marking (`place token count` lines in order),
/// whose hash names it in a firing record.
pub fn marking_bytes(m: &Marking) -> Vec<u8> {
    let mut out = Vec::new();
    for (p, toks) in m {
        for (t, c) in toks {
            out.extend_from_slice(format!("{p} {t} {c}\n").as_bytes());
        }
    }
    out
}

pub fn marking_id(m: &Marking) -> Id {
    Id::of_content(&marking_bytes(m))
}

/// One `(place, token, count)` term of [`sum_hash`], as four lanes.
fn term(place: &str, token: &str, count: u64) -> [u64; 4] {
    let h = blake3::hash(format!("ikam net term\0{place}\0{token}\0{count}").as_bytes());
    let b = h.as_bytes();
    std::array::from_fn(|i| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap()))
}

fn lanes(id: &Id) -> [u64; 4] {
    let b = id.as_bytes();
    std::array::from_fn(|i| u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap()))
}

fn from_lanes(l: [u64; 4]) -> Id {
    let mut b = [0u8; 32];
    for (i, x) in l.iter().enumerate() {
        b[i * 8..i * 8 + 8].copy_from_slice(&x.to_le_bytes());
    }
    Id::from_bytes(b)
}

/// The marking's hash as a sum (E279): each `(place, token, count)` term
/// hashed, the terms added lane by lane modulo 2^64. A firing changes it by
/// subtracting the old terms of the keys it touches and adding the new
/// ones, so it never needs the whole marking. It detects accidental damage
/// and replay errors; unlike `marking_id` it is not meant to resist a
/// forger choosing markings to collide.
pub fn sum_hash(m: &Marking) -> Id {
    let mut acc = [0u64; 4];
    for (p, toks) in m {
        for (t, c) in toks {
            let x = term(p, t, *c);
            for i in 0..4 {
                acc[i] = acc[i].wrapping_add(x[i]);
            }
        }
    }
    from_lanes(acc)
}

const LT_LANES: usize = 1024;

/// One term of [`lt_hash`]: 1,024 lanes of 16 bits from BLAKE3's
/// extendable output.
fn lt_term(place: &str, token: &str, count: u64) -> Vec<u16> {
    let mut h = blake3::Hasher::new();
    h.update(format!("ikam net lt term\0{place}\0{token}\0{count}").as_bytes());
    let mut b = vec![0u8; LT_LANES * 2];
    h.finalize_xof().fill(&mut b);
    b.chunks(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

fn lt_digest(state: &[u16]) -> Id {
    let mut h = blake3::Hasher::new();
    h.update(b"ikam net lthash\0");
    for x in state {
        h.update(&x.to_le_bytes());
    }
    Id::from_bytes(*h.finalize().as_bytes())
}

/// The marking's hash in the LtHash construction (E284): each term
/// expanded to 1,024 lanes of 16 bits and the terms added lane by lane
/// modulo 2^16; the record carries the BLAKE3 digest of that 2 KB state.
/// It updates as [`sum_hash`] does, and its collision resistance rests on
/// a lattice problem (Bellare and Micciancio's construction, as Facebook's
/// LtHash parameters it) rather than on nobody looking for collisions.
pub fn lt_hash(m: &Marking) -> Id {
    let mut acc = Acc::Lt(vec![0; LT_LANES]);
    for (p, toks) in m {
        for (t, c) in toks {
            acc.add(p, t, *c);
        }
    }
    acc.digest()
}

/// A running marking hash kept beside the marking.
enum Acc {
    Sum([u64; 4]),
    Lt(Vec<u16>),
}

impl Acc {
    fn add(&mut self, place: &str, tok: &str, c: u64) {
        match self {
            Acc::Sum(a) => {
                let x = term(place, tok, c);
                for i in 0..4 {
                    a[i] = a[i].wrapping_add(x[i]);
                }
            }
            Acc::Lt(a) => {
                for (y, x) in a.iter_mut().zip(lt_term(place, tok, c)) {
                    *y = y.wrapping_add(x);
                }
            }
        }
    }

    fn sub(&mut self, place: &str, tok: &str, c: u64) {
        match self {
            Acc::Sum(a) => {
                let x = term(place, tok, c);
                for i in 0..4 {
                    a[i] = a[i].wrapping_sub(x[i]);
                }
            }
            Acc::Lt(a) => {
                for (y, x) in a.iter_mut().zip(lt_term(place, tok, c)) {
                    *y = y.wrapping_sub(x);
                }
            }
        }
    }

    fn digest(&self) -> Id {
        match self {
            Acc::Sum(a) => from_lanes(*a),
            Acc::Lt(a) => lt_digest(a),
        }
    }

    /// What is stored at the hash key.
    fn encode(&self) -> Vec<u8> {
        match self {
            Acc::Sum(a) => from_lanes(*a).to_string().into_bytes(),
            Acc::Lt(a) => a.iter().flat_map(|x| x.to_le_bytes()).collect(),
        }
    }
}

fn prefix(net: &str) -> String {
    format!("net/{net}/")
}

fn arcs_text(arcs: &[Arc]) -> String {
    let v: Vec<String> = arcs
        .iter()
        .map(|a| format!("{} {}", a.weight, a.place))
        .collect();
    v.join(",")
}

fn parse_arcs(s: &str) -> Result<Vec<Arc>, Error> {
    if s.is_empty() {
        return Ok(vec![]);
    }
    s.split(',')
        .map(|a| {
            let (w, p) = a.split_once(' ').ok_or(Error::Decode("net arc"))?;
            Ok(Arc {
                place: p.into(),
                weight: w.parse().map_err(|_| Error::Decode("net arc"))?,
            })
        })
        .collect()
}

fn encode_record(f: &Firing) -> Vec<u8> {
    let mut s = format!(
        "{}\n{}\n{}\n{}\n",
        f.transition,
        f.token.as_deref().unwrap_or(""),
        f.before,
        f.after
    );
    // A fifth line only when there is an output, so records without one
    // keep E269's bytes.
    if let Some(o) = &f.output {
        s += &format!("{o}\n");
        // A sixth, the step, only with an output (E441).
        if let Some(st) = &f.step {
            s += &format!("{st}\n");
        }
    }
    // Last, and only on a net that tracks versions (E422), so every other
    // record keeps its bytes: `net <id>`, which no positional line starts
    // with.
    if let Some(v) = &f.net {
        s += &format!("net {v}\n");
    }
    s.into_bytes()
}

fn decode_record(key: &str, bytes: &[u8]) -> Result<Firing, Error> {
    let s = std::str::from_utf8(bytes).map_err(|_| Error::Decode("net firing"))?;
    let mut l: Vec<&str> = s.lines().collect();
    let id = |s: &str| s.parse::<Id>();
    let net = match l.last().and_then(|x| x.strip_prefix("net ")) {
        Some(v) => {
            let v = id(v)?;
            l.pop();
            Some(v)
        }
        None => None,
    };
    if !(4..=6).contains(&l.len()) {
        return Err(Error::Decode("net firing"));
    }
    Ok(Firing {
        key: key.into(),
        transition: l[0].into(),
        token: (!l[1].is_empty()).then(|| l[1].to_string()),
        before: id(l[2])?,
        after: id(l[3])?,
        output: l.get(4).map(|o| id(o)).transpose()?,
        step: l.get(5).map(|o| id(o)).transpose()?,
        net,
    })
}

impl<S: Store> Repo<S> {
    /// Store a checked net as a graph object: a node `net` (name and
    /// version as content), one node per place (`place/<name>`, its
    /// attributes as content) and per transition (`transition/<name>`, kind
    /// and capability as content, edges `in <weight>` and `out <weight>` to
    /// its places). Returns the graph's id.
    pub fn put_net(&mut self, net: &Net) -> Result<Id, Error> {
        net.check()?;
        let mut nodes = Vec::new();
        let content = |r: &mut Self, s: String| -> Result<Target, Error> {
            Ok(Target::Content(Arg::Whole(r.put_content(s.as_bytes())?.id)))
        };
        let head = content(self, format!("{}\n{}\n", net.name, net.version))?;
        nodes.push(Node {
            label: "net".into(),
            target: head,
            edges: vec![],
        });
        for p in &net.places {
            let t = content(self, format!("{}\n{}\n", p.colored as u8, p.initial))?;
            nodes.push(Node {
                label: format!("place/{}", p.name),
                target: t,
                edges: vec![],
            });
        }
        for (i, t) in net.transitions.iter().enumerate() {
            // Arc order and weights are kept in the content; edges make the
            // structure walkable as a graph.
            let body = format!(
                "{}\n{}\n{}\n{}\n{}\n",
                i,
                t.kind.name(),
                t.capability.as_deref().unwrap_or(""),
                arcs_text(&t.inputs),
                arcs_text(&t.outputs)
            );
            let target = content(self, body)?;
            let mut edges: Vec<Edge> = Vec::new();
            for (dir, arcs) in [("in", &t.inputs), ("out", &t.outputs)] {
                for a in arcs.iter() {
                    edges.push(Edge {
                        to: node_key(&format!("place/{}", a.place)),
                        label: format!("{dir} {}", a.weight),
                    });
                }
            }
            nodes.push(Node {
                label: format!("transition/{}", t.name),
                target,
                edges,
            });
        }
        self.put_graph(nodes)
    }

    /// Read a net stored by `put_net`.
    pub fn net_at(&self, graph: &Id) -> Result<Net, Error> {
        let bad = || Error::Decode("net graph");
        let text = |r: &Self, n: &Node| -> Result<String, Error> {
            match &n.target {
                Target::Content(a) => {
                    String::from_utf8(r.read_content(&a.id())?).map_err(|_| bad())
                }
                _ => Err(bad()),
            }
        };
        let mut net = Net {
            name: String::new(),
            version: 0,
            places: vec![],
            transitions: vec![],
        };
        let mut transitions: Vec<(usize, Transition)> = Vec::new();
        for n in self.graph_nodes(graph)? {
            let body = text(self, &n)?;
            let l: Vec<&str> = body.split('\n').collect();
            if n.label == "net" {
                net.name = l[0].into();
                net.version = l.get(1).and_then(|v| v.parse().ok()).ok_or_else(bad)?;
            } else if let Some(name) = n.label.strip_prefix("place/") {
                net.places.push(Place {
                    name: name.into(),
                    colored: l.first() == Some(&"1"),
                    initial: l.get(1).and_then(|v| v.parse().ok()).ok_or_else(bad)?,
                });
            } else if let Some(name) = n.label.strip_prefix("transition/") {
                if l.len() < 5 {
                    return Err(bad());
                }
                transitions.push((
                    l[0].parse().map_err(|_| bad())?,
                    Transition {
                        name: name.into(),
                        kind: Kind::parse(l[1]).ok_or_else(bad)?,
                        capability: (!l[2].is_empty()).then(|| l[2].to_string()),
                        inputs: parse_arcs(l[3])?,
                        outputs: parse_arcs(l[4])?,
                    },
                ));
            }
        }
        // Places come back in label order; transitions in their stored order.
        transitions.sort_by_key(|(i, _)| *i);
        net.transitions = transitions.into_iter().map(|(_, t)| t).collect();
        net.check()?;
        Ok(net)
    }

    /// Write the net's initial marking into `db` once (idempotency key
    /// `net/<name>/init`). Fails if a marking is already there.
    pub fn net_init(&mut self, db: &Db, net: &Net) -> Result<Db, Error> {
        net.check()?;
        let p = prefix(&net.name);
        if self.db_get(db, &format!("{p}init"))?.is_some() {
            return Ok(*db);
        }
        if !self.db_prefix(db, &format!("{p}m/"), 1, false)?.is_empty() {
            return Err(invalid("net already has a marking", &net.name));
        }
        let m = net.initial_marking();
        let idem = format!("{p}init");
        let mut events = vec![Event::upsert(&idem, &marking_bytes(&m), &idem)];
        for (place, toks) in &m {
            for (tok, c) in toks {
                let key = format!("{p}m/{place}/{tok}");
                events.push(Event::upsert(
                    &key,
                    c.to_string().as_bytes(),
                    &format!("{idem}/{key}"),
                ));
            }
        }
        self.db_append(db, &events)
    }

    /// As `net_init`, and keep the marking's [`sum_hash`] at
    /// `net/<name>/hash` (E279). Firings on such a database then read and
    /// write only the keys their arcs touch, and their records carry sum
    /// hashes, so a firing's cost no longer grows with the tokens held.
    pub fn net_init_sum(&mut self, db: &Db, net: &Net) -> Result<Db, Error> {
        let p = prefix(&net.name);
        let key = format!("{p}hash");
        if self.db_get(db, &key)?.is_some() {
            return Ok(*db);
        }
        if self.db_get(db, &format!("{p}init"))?.is_some() {
            return Err(invalid(
                "net already initialized without a sum hash",
                &net.name,
            ));
        }
        let db = self.net_init(db, net)?;
        let h = sum_hash(&net.initial_marking());
        self.db_append(&db, &[Event::upsert(&key, h.to_string().as_bytes(), &key)])
    }

    /// As [`net_init_sum`](Repo::net_init_sum) with the [`lt_hash`]
    /// construction (E284): its 2 KB state at `net/<name>/lthash`, records
    /// carrying its digest. Refused on a net set up any other way.
    pub fn net_init_lthash(&mut self, db: &Db, net: &Net) -> Result<Db, Error> {
        let p = prefix(&net.name);
        let key = format!("{p}lthash");
        if self.db_get(db, &key)?.is_some() {
            return Ok(*db);
        }
        if self.db_get(db, &format!("{p}init"))?.is_some() {
            return Err(invalid(
                "net already initialized without an lthash",
                &net.name,
            ));
        }
        let db = self.net_init(db, net)?;
        let mut acc = Acc::Lt(vec![0; LT_LANES]);
        for (pl, toks) in net.initial_marking() {
            for (t, c) in toks {
                acc.add(&pl, &t, c);
            }
        }
        self.db_append(&db, &[Event::upsert(&key, &acc.encode(), &key)])
    }

    /// The running hash a net was set up with, if any.
    fn hash_state(&self, db: &Db, net: &str) -> Result<Option<Acc>, Error> {
        let p = prefix(net);
        if let Some(v) = self.db_get(db, &format!("{p}hash"))? {
            let id: Id = std::str::from_utf8(&v)
                .map_err(|_| Error::Decode("net hash"))?
                .parse()?;
            return Ok(Some(Acc::Sum(lanes(&id))));
        }
        if let Some(v) = self.db_get(db, &format!("{p}lthash"))? {
            if v.len() != LT_LANES * 2 {
                return Err(Error::Decode("net lthash"));
            }
            return Ok(Some(Acc::Lt(
                v.chunks(2)
                    .map(|c| u16::from_le_bytes([c[0], c[1]]))
                    .collect(),
            )));
        }
        Ok(None)
    }

    #[cfg(test)]
    fn sum_mode(&self, db: &Db, net: &str) -> Result<Option<Id>, Error> {
        Ok(self.hash_state(db, net)?.map(|a| a.digest()))
    }

    /// The net's current marking, read from `db`.
    pub fn net_marking(&self, db: &Db, net: &str) -> Result<Marking, Error> {
        let p = format!("{}m/", prefix(net));
        let mut m = Marking::new();
        for (key, value) in self.db_prefix(db, &p, usize::MAX, false)? {
            let rest = &key[p.len()..];
            let (place, tok) = rest.split_once('/').ok_or(Error::Decode("net marking"))?;
            let c: u64 = std::str::from_utf8(&value)
                .ok()
                .and_then(|s| s.parse().ok())
                .ok_or(Error::Decode("net marking"))?;
            m.entry(place.into()).or_default().insert(tok.into(), c);
        }
        Ok(m)
    }

    /// Transitions enabled now, with the token each would bind: every
    /// colored token present in a transition's colored inputs, or none for
    /// one that binds no color. A colored source transition (no inputs)
    /// needs a new token from its caller and is not listed. With `auto`,
    /// human transitions are left out: a scheduler never fires them.
    pub fn net_enabled(
        &self,
        db: &Db,
        net: &Net,
        auto: bool,
    ) -> Result<Vec<(String, Option<String>)>, Error> {
        let m = self.net_marking(db, &net.name)?;
        let mut out = Vec::new();
        for t in &net.transitions {
            if auto && t.kind == Kind::Human {
                continue;
            }
            if !net.binds_color(t) {
                if net.is_enabled(&m, t, None) {
                    out.push((t.name.clone(), None));
                }
                continue;
            }
            let colored_in = t
                .inputs
                .iter()
                .find(|a| net.place(&a.place).is_some_and(|p| p.colored));
            let Some(first) = colored_in else { continue };
            for tok in m.get(&first.place).into_iter().flat_map(|p| p.keys()) {
                if net.is_enabled(&m, t, Some(tok)) {
                    out.push((t.name.clone(), Some(tok.clone())));
                }
            }
        }
        Ok(out)
    }

    /// The first `limit` entries of [`net_enabled`](Repo::net_enabled), in
    /// the same order, reading only what it needs (E280): plain inputs by
    /// key, and a colored transition's first colored input place by prefix,
    /// in pages that double until `limit` are found or the place is read to
    /// the end. Its cost follows `limit` and the net, not the tokens held,
    /// unless most held tokens are not enabled.
    pub fn net_enabled_limit(
        &self,
        db: &Db,
        net: &Net,
        auto: bool,
        limit: usize,
    ) -> Result<Vec<(String, Option<String>)>, Error> {
        let p = prefix(&net.name);
        let count = |key: &str| -> Result<u64, Error> {
            match self.db_get(db, key)? {
                Some(v) => std::str::from_utf8(&v)
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .ok_or(Error::Decode("net marking")),
                None => Ok(0),
            }
        };
        let colored = |place: &str| net.place(place).is_some_and(|pl| pl.colored);
        let mut out = Vec::new();
        for t in &net.transitions {
            if out.len() >= limit {
                break;
            }
            if auto && t.kind == Kind::Human {
                continue;
            }
            // Arcs on plain places, summed by place.
            let mut plain: BTreeMap<&str, u64> = BTreeMap::new();
            let mut col: BTreeMap<&str, u64> = BTreeMap::new();
            for a in &t.inputs {
                let e = if colored(&a.place) {
                    &mut col
                } else {
                    &mut plain
                };
                *e.entry(a.place.as_str()).or_default() += a.weight as u64;
            }
            let mut ok = true;
            for (place, w) in &plain {
                if count(&format!("{p}m/{place}/{ANON}"))? < *w {
                    ok = false;
                    break;
                }
            }
            if !ok {
                continue;
            }
            if !net.binds_color(t) {
                out.push((t.name.clone(), None));
                continue;
            }
            let Some(first) = t.inputs.iter().find(|a| colored(&a.place)) else {
                continue;
            };
            let pre = format!("{p}m/{}/", first.place);
            let mut page = (limit - out.len()).max(8);
            let mut seen = 0;
            loop {
                let rows = self.db_prefix(db, &pre, page, false)?;
                let done = rows.len() < page;
                for (key, _) in rows.iter().skip(seen) {
                    let tok = &key[pre.len()..];
                    let mut all = true;
                    for (place, w) in &col {
                        if count(&format!("{p}m/{place}/{tok}"))? < *w {
                            all = false;
                            break;
                        }
                    }
                    if all {
                        out.push((t.name.clone(), Some(tok.to_string())));
                        if out.len() >= limit {
                            return Ok(out);
                        }
                    }
                }
                seen = rows.len();
                if done {
                    break;
                }
                page *= 2;
            }
        }
        Ok(out)
    }

    /// Fire `transition` with `token` bound, as one append: the changed
    /// counts and the firing record under `key`. A retry with the same key
    /// returns the recorded firing and changes nothing; a key already used
    /// for another firing is refused (`Error::IdempotencyConflict`), as is a
    /// transition that is not enabled. Firing a human transition is the
    /// caller's decision to make; `net_enabled` with `auto` never offers one.
    pub fn net_fire(
        &mut self,
        db: &Db,
        net: &Net,
        transition: &str,
        token: Option<&str>,
        key: &str,
    ) -> Result<(Db, Firing), Error> {
        self.fire_inner(db, net, transition, token, key, None, None)
    }

    /// Fire as `net_fire`, recording the work's output in the same append
    /// (E271): the bytes as content, their id at `net/<name>/out/<key>`
    /// and in the firing record. A retry with the same key and the same
    /// bytes changes nothing; the same key with other bytes is refused
    /// (`Error::IdempotencyConflict`), which is how a rerun of work that is
    /// not deterministic shows up. See also `net_verify`.
    pub fn net_fire_output(
        &mut self,
        db: &Db,
        net: &Net,
        transition: &str,
        token: Option<&str>,
        key: &str,
        output: &[u8],
    ) -> Result<(Db, Firing), Error> {
        self.fire_inner(db, net, transition, token, key, Some(output), None)
    }

    /// Whether `output` (the work recomputed) matches what the firing under
    /// `key` recorded. None when nothing was recorded under `key`.
    pub fn net_verify(
        &self,
        db: &Db,
        net: &str,
        key: &str,
        output: &[u8],
    ) -> Result<Option<bool>, Error> {
        Ok(self
            .net_output(db, net, key)?
            .map(|(id, _)| id == Id::of_content(output)))
    }

    /// Fire `transition` through an executable step (E441), so a net's work
    /// and a stored function share one model: a `pure` transition applies
    /// `func(args)` as a derivation (re-evaluated by fsck and
    /// [`net_verify_steps`](Repo::net_verify_steps)); a `cmd` or `agent`
    /// transition records its output as a recorded step (kept, never
    /// rebuilt; a rerun is a check). A `human` transition runs no step. The
    /// firing record names the output and the step. A retry with the same
    /// key and the same step changes nothing.
    pub fn net_fire_step(
        &mut self,
        db: &Db,
        net: &Net,
        transition: &str,
        token: Option<&str>,
        key: &str,
        step: Step<'_>,
    ) -> Result<(Db, Firing), Error> {
        let t = net
            .transition(transition)
            .ok_or_else(|| invalid("transition", transition))?;
        let (output, record) = match (t.kind, step) {
            (Kind::Pure, Step::Pure { func, args }) => {
                let output = self.apply(func, args.clone())?;
                let d = crate::Derivation { output, func, args };
                (output, crate::Object::Derivation(d).id())
            }
            (
                Kind::Cmd | Kind::Agent,
                Step::Recorded {
                    func,
                    args,
                    output,
                    check,
                },
            ) => self.record_step(func, args, output, check, crate::step::Design::Run)?,
            (kind, _) => {
                return Err(Error::Exec(format!(
                    "transition {transition} is {}: no step of that kind",
                    kind.name()
                )));
            }
        };
        let bytes = self.read_content(&output)?;
        self.fire_inner(db, net, transition, token, key, Some(&bytes), Some(record))
    }

    /// Check every firing made through a step (E441), in log order, by the
    /// same code that reconstructs files: a pure step is evaluated again
    /// and must give its output; a recorded step's output must still be
    /// here as bytes, and a step the kernel can run (a WASM function) is
    /// rerun and must give the same bits. Returns each firing's key and
    /// whether it passed.
    pub fn net_verify_steps(&mut self, db: &Db, net: &str) -> Result<Vec<(String, bool)>, Error> {
        let mut out = Vec::new();
        for f in self.net_firings(db, net)? {
            let (Some(step), Some(output)) = (f.step, f.output) else {
                continue;
            };
            let ok = match self.get(&step)? {
                crate::Object::Derivation(d) if d.func != crate::func::recorded() => {
                    let (bytes, _) = self.evaluate(&d.func, &d.args)?;
                    d.output == output && Id::of_content(&bytes) == output
                }
                _ => {
                    let kept = self.read_content(&output).is_ok();
                    let runs = self.rerun(&step, None);
                    kept && match runs {
                        Ok(r) => r.first == output && r.same,
                        // Work done outside the kernel: kept is the check.
                        Err(Error::Exec(_)) => true,
                        Err(e) => return Err(e),
                    }
                }
            };
            out.push((f.key, ok));
        }
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn fire_inner(
        &mut self,
        db: &Db,
        net: &Net,
        transition: &str,
        token: Option<&str>,
        key: &str,
        output: Option<&[u8]>,
        step: Option<Id>,
    ) -> Result<(Db, Firing), Error> {
        if !valid_name(key) {
            return Err(invalid("firing key", key));
        }
        if let Some(t) = token
            && (!valid_name(t) || t == ANON)
        {
            return Err(invalid("token", t));
        }
        let p = prefix(&net.name);
        let rec_key = format!("{p}fire/{key}");
        let version = self.tracked_version(db, net)?;
        if let Some(bytes) = self.db_get(db, &rec_key)? {
            let f = decode_record(key, &bytes)?;
            let same_output = match output {
                None => true,
                Some(o) => f.output == Some(Id::of_content(o)),
            };
            if f.transition != transition
                || f.token.as_deref() != token
                || !same_output
                || f.step != step
                || f.net != version
            {
                return Err(Error::IdempotencyConflict(rec_key));
            }
            return Ok((*db, f));
        }
        let t = net
            .transition(transition)
            .ok_or_else(|| invalid("transition", transition))?;
        if let Some(h) = self.hash_state(db, &net.name)? {
            return self.fire_sum(db, net, t, token, key, output, step, version, h);
        }
        let before = self.net_marking(db, &net.name)?;
        let after = net
            .fire(&before, t, token)
            .ok_or_else(|| Error::Exec(format!("transition {transition} is not enabled")))?;
        let out_key = format!("{p}out/{key}");
        let output = match output {
            Some(o) => Some(self.put_content(o)?.id),
            None => None,
        };
        let f = Firing {
            key: key.into(),
            transition: transition.into(),
            token: token.map(String::from),
            before: marking_id(&before),
            after: marking_id(&after),
            output,
            step,
            net: version,
        };
        let mut events = Vec::new();
        let places: std::collections::BTreeSet<&String> =
            before.keys().chain(after.keys()).collect();
        for place in places {
            let empty = BTreeMap::new();
            let (b, a) = (
                before.get(place).unwrap_or(&empty),
                after.get(place).unwrap_or(&empty),
            );
            let toks: std::collections::BTreeSet<&String> = b.keys().chain(a.keys()).collect();
            for tok in toks {
                let mk = format!("{p}m/{place}/{tok}");
                let idem = format!("{rec_key}/{place}/{tok}");
                match (b.get(tok), a.get(tok)) {
                    (x, Some(c)) if x != Some(c) => {
                        events.push(Event::upsert(&mk, c.to_string().as_bytes(), &idem))
                    }
                    (Some(_), None) => events.push(Event::remove(&mk, false, &idem)),
                    _ => {}
                }
            }
        }
        if let Some(o) = &f.output {
            events.push(Event::upsert(&out_key, o.to_string().as_bytes(), &out_key));
        }
        events.push(Event::upsert(&rec_key, &encode_record(&f), &rec_key));
        Ok((self.db_append(db, &events)?, f))
    }

    /// The output recorded under `key`, if any: its id and bytes.
    pub fn net_output(
        &self,
        db: &Db,
        net: &str,
        key: &str,
    ) -> Result<Option<(Id, Vec<u8>)>, Error> {
        let Some(v) = self.db_get(db, &format!("{}out/{key}", prefix(net)))? else {
            return Ok(None);
        };
        let id: Id = std::str::from_utf8(&v)
            .map_err(|_| Error::Decode("net output"))?
            .parse()?;
        Ok(Some((id, self.read_content(&id)?)))
    }

    /// `fire_inner` on a database with a sum hash: reads and writes only
    /// the keys `t`'s arcs touch.
    #[allow(clippy::too_many_arguments)]
    fn fire_sum(
        &mut self,
        db: &Db,
        net: &Net,
        t: &Transition,
        token: Option<&str>,
        key: &str,
        output: Option<&[u8]>,
        step: Option<Id>,
        version: Option<Id>,
        mut acc: Acc,
    ) -> Result<(Db, Firing), Error> {
        let before = acc.digest();
        if net.binds_color(t) != token.is_some() {
            return Err(Error::Exec(format!("transition {} is not enabled", t.name)));
        }
        let p = prefix(&net.name);
        let rec_key = format!("{p}fire/{key}");
        // Net change per (place, token) key: taken and added.
        let mut delta: BTreeMap<(String, String), (u64, u64)> = BTreeMap::new();
        let tok_for = |place: &str| {
            if net.place(place).is_some_and(|p| p.colored) {
                token.unwrap_or(ANON).to_string()
            } else {
                ANON.to_string()
            }
        };
        for a in &t.inputs {
            delta
                .entry((a.place.clone(), tok_for(&a.place)))
                .or_default()
                .0 += a.weight as u64;
        }
        for a in &t.outputs {
            delta
                .entry((a.place.clone(), tok_for(&a.place)))
                .or_default()
                .1 += a.weight as u64;
        }
        let mut events = Vec::new();
        for ((place, tok), (take, add)) in &delta {
            let mk = format!("{p}m/{place}/{tok}");
            let old: u64 = match self.db_get(db, &mk)? {
                Some(v) => std::str::from_utf8(&v)
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .ok_or(Error::Decode("net marking"))?,
                None => 0,
            };
            if old < *take {
                return Err(Error::Exec(format!("transition {} is not enabled", t.name)));
            }
            let new = old - take + add;
            if new == old {
                continue;
            }
            if old > 0 {
                acc.sub(place, tok, old);
            }
            let idem = format!("{rec_key}/{place}/{tok}");
            if new > 0 {
                acc.add(place, tok, new);
                events.push(Event::upsert(&mk, new.to_string().as_bytes(), &idem));
            } else {
                events.push(Event::remove(&mk, false, &idem));
            }
        }
        let after = acc.digest();
        let out_key = format!("{p}out/{key}");
        let output = match output {
            Some(o) => Some(self.put_content(o)?.id),
            None => None,
        };
        let f = Firing {
            key: key.into(),
            transition: t.name.clone(),
            token: token.map(String::from),
            before,
            after,
            output,
            step,
            net: version,
        };
        let hk = match acc {
            Acc::Sum(_) => format!("{p}hash"),
            Acc::Lt(_) => format!("{p}lthash"),
        };
        events.push(Event::upsert(
            &hk,
            &acc.encode(),
            &format!("{rec_key}/hash"),
        ));
        if let Some(o) = &f.output {
            events.push(Event::upsert(&out_key, o.to_string().as_bytes(), &out_key));
        }
        events.push(Event::upsert(&rec_key, &encode_record(&f), &rec_key));
        Ok((self.db_append(db, &events)?, f))
    }

    /// Save `bytes` as the progress of the work that the firing under `key`
    /// will finish (E281): the bytes as content, their id at
    /// `net/<name>/ckpt/<key>`, replacing any earlier checkpoint. A runner
    /// that crashes resumes from the last one with
    /// [`net_resume`](Repo::net_resume) instead of from the start. Saving
    /// bytes already saved once under `key` changes nothing, so progress
    /// should name its step.
    pub fn net_checkpoint(
        &mut self,
        db: &Db,
        net: &str,
        key: &str,
        bytes: &[u8],
    ) -> Result<Db, Error> {
        let id = self.put_content(bytes)?.id;
        let k = format!("{}ckpt/{key}", prefix(net));
        let idem = format!("{k}/{id}");
        self.db_append(db, &[Event::upsert(&k, id.to_string().as_bytes(), &idem)])
    }

    /// The last checkpoint saved under `key`, if any.
    pub fn net_resume(&self, db: &Db, net: &str, key: &str) -> Result<Option<Vec<u8>>, Error> {
        match self.db_get(db, &format!("{}ckpt/{key}", prefix(net)))? {
            Some(v) => {
                let id: Id = std::str::from_utf8(&v)
                    .map_err(|_| Error::Decode("net checkpoint"))?
                    .parse()?;
                Ok(Some(self.read_content(&id)?))
            }
            None => Ok(None),
        }
    }

    /// Store `net` as a file (E422): its canonical text at `path` in tree
    /// `root` (None: an empty tree), as any other file is stored, so it is
    /// versioned by commits, diffed and fragmented as text, and a tree may
    /// hold any number of nets. Returns the new tree and the net's
    /// [`version_id`](Net::version_id).
    pub fn put_net_file(
        &mut self,
        root: Option<Id>,
        path: &str,
        net: &Net,
    ) -> Result<(Id, Id), Error> {
        net.check()?;
        let id = self.put_content(net.to_text().as_bytes())?.id;
        let edit = crate::worktree::Edit::Put {
            path: path.into(),
            id,
        };
        Ok((self.edit_tree(root, &[edit])?, id))
    }

    /// The net held as a file at `path` in `rev` (a commit or a tree): the
    /// version that commit holds. The file may be written by hand, comments
    /// and all; it is parsed with [`Net::parse`].
    pub fn net_file(&self, rev: &Id, path: &str) -> Result<Net, Error> {
        let mut tree = crate::snapshot::tree_of(self, rev)?;
        let parts = crate::worktree::split_path(path)?;
        let (last, dirs) = parts.split_last().expect("split_path is never empty");
        let entry = |r: &Self, t: &Id, name: &str, kind: crate::Kind| -> Result<Id, Error> {
            let crate::Object::Tree(entries) = r.get(t)? else {
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
        let id = entry(self, &tree, last, crate::Kind::File)?;
        let bytes = self.read_content(&id)?;
        Net::parse(std::str::from_utf8(&bytes).map_err(|_| Error::Decode("net text"))?)
    }

    /// Every net held as a file in `rev` (paths ending `.net`), in path
    /// order, with the file's content id.
    pub fn net_files(&self, rev: &Id) -> Result<Vec<(String, Id)>, Error> {
        let mut out = Vec::new();
        let mut stack = vec![(String::new(), crate::snapshot::tree_of(self, rev)?)];
        while let Some((dir, t)) = stack.pop() {
            let crate::Object::Tree(entries) = self.get(&t)? else {
                return Err(Error::WrongKind(t));
            };
            for e in entries {
                let path = match dir.is_empty() {
                    true => e.name.clone(),
                    false => format!("{dir}/{}", e.name),
                };
                match e.kind {
                    crate::Kind::Tree => stack.push((path, e.id)),
                    crate::Kind::File if e.name.ends_with(".net") => out.push((path, e.id)),
                    _ => {}
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// The net whose [`version_id`](Net::version_id) is `id`, read back
    /// from the store; or, when `id` is no net's text, the net whose
    /// fragment root it is (E446, [`put_net_fragments`](Repo::put_net_fragments)).
    pub fn net_version(&self, id: &Id) -> Result<Net, Error> {
        match self.net_text_version(id) {
            Ok(net) => Ok(net),
            Err(e) => match self.get(id) {
                Ok(crate::Object::Graph(_)) => self.net_fragments(id),
                _ => Err(e),
            },
        }
    }

    fn net_text_version(&self, id: &Id) -> Result<Net, Error> {
        let bytes = self.read_content(id)?;
        let net = Net::parse(std::str::from_utf8(&bytes).map_err(|_| Error::Decode("net text"))?)?;
        if net.version_id() != *id {
            return Err(Error::Decode("net version: not a net's canonical text"));
        }
        Ok(net)
    }

    /// Track versions on `net` (E422; off unless called): from here on each
    /// firing records the version of the net it was fired with (one more
    /// line on its record), and that version's text is kept, so the net can
    /// be edited while it runs and every firing still replays against the
    /// version it ran under ([`net_replay_versions`](Repo::net_replay_versions)).
    /// Call after `net_init` (or a hashed init) of this same version, before
    /// the first firing. An edited version fires on the same marking: its
    /// new places start empty, and initial tokens count only at init.
    pub fn net_track_versions(&mut self, db: &Db, net: &Net) -> Result<Db, Error> {
        net.check()?;
        let id = self.put_content(net.to_text().as_bytes())?.id;
        self.track_from(db, net, id.to_string())
    }

    /// [`net_track_versions`](Repo::net_track_versions) with each version
    /// named by its fragment root (E446): every firing stores `net` as
    /// fragments laid out as `layout` says (what an earlier version wrote
    /// is found, not written again) and names the root. Replay reads each
    /// root back with [`net_version`](Repo::net_version). The tracking key
    /// holds the first root and the layout, so a firing knows the form.
    pub fn net_track_fragments(&mut self, db: &Db, net: &Net, layout: Layout) -> Result<Db, Error> {
        let id = self.put_net_fragments(net, layout)?;
        self.track_from(db, net, format!("{id} {}", layout.name()))
    }

    fn track_from(&mut self, db: &Db, net: &Net, value: String) -> Result<Db, Error> {
        let p = prefix(&net.name);
        let key = format!("{p}ver");
        if let Some(v) = self.db_get(db, &key)? {
            if v == value.as_bytes() {
                return Ok(*db);
            }
            return Err(invalid("net tracks versions from another", &net.name));
        }
        match self.db_get(db, &format!("{p}init"))? {
            Some(m) if m == marking_bytes(&net.initial_marking()) => {}
            Some(_) => return Err(invalid("net was initialized by another version", &net.name)),
            None => return Err(invalid("net is not initialized", &net.name)),
        }
        if !self
            .db_prefix(db, &format!("{p}fire/"), 1, false)?
            .is_empty()
        {
            return Err(invalid("net fired before tracking versions", &net.name));
        }
        self.db_append(db, &[Event::upsert(&key, value.as_bytes(), &key)])
    }

    /// The tracking key's first version and, for fragments, layout.
    fn tracking(&self, db: &Db, net: &str) -> Result<Option<(Id, Option<Layout>)>, Error> {
        let Some(v) = self.db_get(db, &format!("{}ver", prefix(net)))? else {
            return Ok(None);
        };
        let bad = || Error::Decode("net version");
        let v = std::str::from_utf8(&v).map_err(|_| bad())?;
        Ok(Some(match v.split_once(' ') {
            None => (v.parse()?, None),
            Some((id, layout)) => (id.parse()?, Some(Layout::parse(layout).ok_or_else(bad)?)),
        }))
    }

    /// The version of the net that tracking started from, if it tracks
    /// versions.
    pub fn net_first_version(&self, db: &Db, net: &str) -> Result<Option<Id>, Error> {
        Ok(self.tracking(db, net)?.map(|(id, _)| id))
    }

    /// On a net that tracks versions, the version `net` is, its text kept
    /// so replay can read it back; None on any other net.
    fn tracked_version(&mut self, db: &Db, net: &Net) -> Result<Option<Id>, Error> {
        match self.tracking(db, &net.name)? {
            None => Ok(None),
            Some((_, None)) => Ok(Some(self.put_content(net.to_text().as_bytes())?.id)),
            Some((_, Some(layout))) => Ok(Some(self.put_net_fragments(net, layout)?)),
        }
    }

    /// Recompute the marking of a net that tracks versions from the
    /// recorded firings alone, each against the version of the net it
    /// names (read back from the store), checking every record's before
    /// and after hashes. It must equal `net_marking`, however often the
    /// net was edited between firings.
    pub fn net_replay_versions(&self, db: &Db, name: &str) -> Result<Marking, Error> {
        let first = self
            .net_first_version(db, name)?
            .ok_or_else(|| invalid("net does not track versions", name))?;
        let hash: fn(&Marking) -> Id = match self.hash_state(db, name)? {
            Some(Acc::Sum(_)) => sum_hash,
            Some(Acc::Lt(_)) => lt_hash,
            None => marking_id,
        };
        let mut nets: std::collections::HashMap<Id, Net> = Default::default();
        let mut m = self.net_version(&first)?.initial_marking();
        for f in self.net_firings(db, name)? {
            let v = f
                .net
                .ok_or(Error::Decode("net replay: firing names no version"))?;
            if let std::collections::hash_map::Entry::Vacant(e) = nets.entry(v) {
                let net = self.net_version(&v)?;
                if net.name != name {
                    return Err(Error::Decode("net replay: version of another net"));
                }
                e.insert(net);
            }
            let net = &nets[&v];
            let t = net
                .transition(&f.transition)
                .ok_or_else(|| invalid("recorded transition", &f.transition))?;
            if hash(&m) != f.before {
                return Err(Error::Decode("net replay: marking before differs"));
            }
            m = net
                .fire(&m, t, f.token.as_deref())
                .ok_or(Error::Decode("net replay: recorded firing not enabled"))?;
            if hash(&m) != f.after {
                return Err(Error::Decode("net replay: marking after differs"));
            }
            if let Some(o) = &f.output {
                self.read_content(o)?;
            }
        }
        Ok(m)
    }

    /// The firings recorded for `net`, in log order.
    pub fn net_firings(&self, db: &Db, net: &str) -> Result<Vec<Firing>, Error> {
        let p = format!("{}fire/", prefix(net));
        let mut out = Vec::new();
        for (_, e) in self.db_events(db)? {
            if let (Some(key), crate::db::Op::Upsert(v)) = (e.key.strip_prefix(&p), &e.op)
                && !key.contains('/')
            {
                out.push(decode_record(key, v)?);
            }
        }
        Ok(out)
    }

    /// Recompute the marking from the initial one and the recorded firings
    /// alone, checking each record's before and after hashes on the way.
    /// It must equal `net_marking`.
    pub fn net_replay(&self, db: &Db, net: &Net) -> Result<Marking, Error> {
        let hash: fn(&Marking) -> Id = match self.hash_state(db, &net.name)? {
            Some(Acc::Sum(_)) => sum_hash,
            Some(Acc::Lt(_)) => lt_hash,
            None => marking_id,
        };
        let mut m = net.initial_marking();
        for f in self.net_firings(db, &net.name)? {
            let t = net
                .transition(&f.transition)
                .ok_or_else(|| invalid("recorded transition", &f.transition))?;
            if hash(&m) != f.before {
                return Err(Error::Decode("net replay: marking before differs"));
            }
            m = net
                .fire(&m, t, f.token.as_deref())
                .ok_or(Error::Decode("net replay: recorded firing not enabled"))?;
            if hash(&m) != f.after {
                return Err(Error::Decode("net replay: marking after differs"));
            }
            if let Some(o) = &f.output {
                self.read_content(o)?;
            }
        }
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemStore;

    fn arc(p: &str) -> Arc {
        Arc {
            place: p.into(),
            weight: 1,
        }
    }

    fn tr(name: &str, i: &[&str], o: &[&str], kind: Kind) -> Transition {
        Transition {
            name: name.into(),
            inputs: i.iter().map(|p| arc(p)).collect(),
            outputs: o.iter().map(|p| arc(p)).collect(),
            kind,
            capability: None,
        }
    }

    /// A small experiment loop: candidates queue, take a cpu slot, run,
    /// free it, then wait for the owner.
    pub(crate) fn loop_net() -> Net {
        let place = |n: &str, colored: bool, initial: u32| Place {
            name: n.into(),
            colored,
            initial,
        };
        Net {
            name: "loop".into(),
            version: 1,
            places: vec![
                place("cpu", false, 1),
                place("queued", true, 0),
                place("running", true, 0),
                place("measured", true, 0),
                place("waiting", true, 0),
            ],
            transitions: vec![
                tr("propose", &[], &["queued"], Kind::Agent),
                tr("start", &["queued", "cpu"], &["running"], Kind::Cmd),
                tr("finish", &["running"], &["measured", "cpu"], Kind::Pure),
                tr("ask", &["measured"], &["waiting"], Kind::Pure),
                tr("merge", &["waiting"], &[], Kind::Human),
            ],
        }
    }

    #[test]
    fn stores_and_reads_back() {
        let mut r = Repo::new(MemStore::default());
        let net = loop_net();
        let g = r.put_net(&net).unwrap();
        let back = r.net_at(&g).unwrap();
        assert_eq!(back.transitions, net.transitions);
        let mut a = back.places.clone();
        let mut b = net.places.clone();
        a.sort_by(|x, y| x.name.cmp(&y.name));
        b.sort_by(|x, y| x.name.cmp(&y.name));
        assert_eq!(a, b);
        assert_eq!(r.put_net(&net).unwrap(), g, "same net, same graph");
    }

    #[test]
    fn fires_retries_and_replays() {
        let mut r = Repo::new(MemStore::default());
        let net = loop_net();
        let db = r.db_empty().unwrap();
        let db = r.net_init(&db, &net).unwrap();
        let (db, _) = r.net_fire(&db, &net, "propose", Some("E1"), "p1").unwrap();
        let (db, _) = r.net_fire(&db, &net, "propose", Some("E2"), "p2").unwrap();
        let (db, f) = r.net_fire(&db, &net, "start", Some("E1"), "s1").unwrap();
        // Retry: same key, nothing changes.
        let (db2, f2) = r.net_fire(&db, &net, "start", Some("E1"), "s1").unwrap();
        assert_eq!(db2, db);
        assert_eq!(f2, f);
        // Reusing a key for another firing is refused.
        assert!(matches!(
            r.net_fire(&db, &net, "start", Some("E2"), "s1"),
            Err(Error::IdempotencyConflict(_))
        ));
        // The cpu slot is taken: E2 cannot start.
        assert!(r.net_fire(&db, &net, "start", Some("E2"), "s2").is_err());
        let auto = r.net_enabled(&db, &net, true).unwrap();
        assert_eq!(auto, vec![("finish".to_string(), Some("E1".to_string()))]);
        let (db, _) = r.net_fire(&db, &net, "finish", Some("E1"), "f1").unwrap();
        let (db, _) = r.net_fire(&db, &net, "ask", Some("E1"), "a1").unwrap();
        // The owner's transition is offered only without `auto`.
        let auto = r.net_enabled(&db, &net, true).unwrap();
        assert!(auto.iter().all(|(t, _)| t != "merge"));
        let all = r.net_enabled(&db, &net, false).unwrap();
        assert!(all.contains(&("merge".to_string(), Some("E1".to_string()))));
        let m = r.net_marking(&db, "loop").unwrap();
        assert_eq!(r.net_replay(&db, &net).unwrap(), m);
        assert_eq!(count(&m, "cpu", ANON), 1);
        assert_eq!(count(&m, "queued", "E2"), 1);
        assert_eq!(count(&m, "waiting", "E1"), 1);
        assert_eq!(r.net_firings(&db, "loop").unwrap().len(), 5);
        // The database folds to the same views from scratch.
        assert_eq!(r.db_rebuild(&db.log).unwrap().kv, db.kv);
    }

    #[test]
    fn records_outputs_once() {
        let mut r = Repo::new(MemStore::default());
        let net = loop_net();
        let db = r.db_empty().unwrap();
        let db = r.net_init(&db, &net).unwrap();
        let (db, _) = r.net_fire(&db, &net, "propose", Some("E1"), "p1").unwrap();
        let (db, _) = r.net_fire(&db, &net, "start", Some("E1"), "s1").unwrap();
        assert!(r.net_output(&db, "loop", "f1").unwrap().is_none());
        let (db, f) = r
            .net_fire_output(&db, &net, "finish", Some("E1"), "f1", b"cost 0.31")
            .unwrap();
        assert_eq!(f.output, Some(Id::of_content(b"cost 0.31")));
        // A retry with the same bytes: no change. Other bytes: refused.
        let again = r.net_fire_output(&db, &net, "finish", Some("E1"), "f1", b"cost 0.31");
        assert_eq!(again.unwrap().0, db);
        assert!(matches!(
            r.net_fire_output(&db, &net, "finish", Some("E1"), "f1", b"cost 0.30"),
            Err(Error::IdempotencyConflict(_))
        ));
        assert_eq!(
            r.net_output(&db, "loop", "f1").unwrap().unwrap().1,
            b"cost 0.31"
        );
        assert_eq!(
            r.net_verify(&db, "loop", "f1", b"cost 0.31").unwrap(),
            Some(true)
        );
        assert_eq!(
            r.net_verify(&db, "loop", "f1", b"cost 0.30").unwrap(),
            Some(false)
        );
        assert_eq!(r.net_verify(&db, "loop", "s1", b"x").unwrap(), None);
        let fs = r.net_firings(&db, "loop").unwrap();
        assert_eq!(fs.last().unwrap().output, f.output);
        assert_eq!(fs[0].output, None);
        assert_eq!(
            r.net_replay(&db, &net).unwrap(),
            r.net_marking(&db, "loop").unwrap()
        );
        assert_eq!(r.db_rebuild(&db.log).unwrap().kv, db.kv);
    }

    #[test]
    fn sum_hash_follows_the_marking() {
        let mut r = Repo::new(MemStore::default());
        let net = loop_net();
        let db = r.db_empty().unwrap();
        let db = r.net_init_sum(&db, &net).unwrap();
        assert_eq!(r.net_init_sum(&db, &net).unwrap(), db);
        let mut db = db;
        for i in 0..5 {
            let tok = format!("E{i}");
            for t in ["propose", "start", "finish", "ask"] {
                db = r
                    .net_fire(&db, &net, t, Some(&tok), &format!("{t}-{tok}"))
                    .unwrap()
                    .0;
            }
        }
        // Not enabled, retry, reused key: as in the classic path.
        assert!(r.net_fire(&db, &net, "start", Some("E9"), "x").is_err());
        assert_eq!(
            r.net_fire(&db, &net, "ask", Some("E0"), "ask-E0")
                .unwrap()
                .0,
            db
        );
        assert!(matches!(
            r.net_fire(&db, &net, "merge", Some("E1"), "ask-E0"),
            Err(Error::IdempotencyConflict(_))
        ));
        let (db, _) = r
            .net_fire(&db, &net, "merge", Some("E1"), "merge-E1")
            .unwrap();
        let m = r.net_marking(&db, "loop").unwrap();
        assert_eq!(r.sum_mode(&db, "loop").unwrap(), Some(sum_hash(&m)));
        assert_eq!(r.net_replay(&db, &net).unwrap(), m);
        assert_eq!(r.db_rebuild(&db.log).unwrap().kv, db.kv);
        // The sum hash is order-free and differs from marking_id.
        assert_ne!(sum_hash(&m), marking_id(&m));
        let plain = r.db_empty().unwrap();
        let plain = r.net_init(&plain, &net).unwrap();
        assert!(r.net_init_sum(&plain, &net).is_err());
    }

    #[test]
    fn enabled_limit_is_a_prefix_of_enabled() {
        let mut r = Repo::new(MemStore::default());
        let net = loop_net();
        let db = r.db_empty().unwrap();
        let mut db = r.net_init(&db, &net).unwrap();
        let check = |r: &Repo<MemStore>, db: &Db| {
            for auto in [false, true] {
                let all = r.net_enabled(db, &net, auto).unwrap();
                for k in [0, 1, 2, 5, 40, 1000] {
                    let some = r.net_enabled_limit(db, &net, auto, k).unwrap();
                    assert_eq!(some, all[..k.min(all.len())], "limit {k}, auto {auto}");
                }
            }
        };
        check(&r, &db);
        for i in 0..30 {
            let tok = format!("E{i:02}");
            for t in ["propose", "start", "finish", "ask"] {
                db = r
                    .net_fire(&db, &net, t, Some(&tok), &format!("{t}-{tok}"))
                    .unwrap()
                    .0;
                if i % 7 == 0 {
                    check(&r, &db);
                }
            }
            if i % 3 == 0 {
                db = r
                    .net_fire(&db, &net, "merge", Some(&tok), &format!("merge-{tok}"))
                    .unwrap()
                    .0;
            }
        }
        // Some queued, one running: start is blocked on the cpu.
        for i in 30..45 {
            let tok = format!("E{i:02}");
            db = r
                .net_fire(&db, &net, "propose", Some(&tok), &format!("propose-{tok}"))
                .unwrap()
                .0;
        }
        db = r
            .net_fire(&db, &net, "start", Some("E31"), "start-E31")
            .unwrap()
            .0;
        check(&r, &db);
    }

    #[test]
    fn checkpoints_replace_and_resume() {
        let mut r = Repo::new(MemStore::default());
        let db = r.db_empty().unwrap();
        assert_eq!(r.net_resume(&db, "loop", "finish-E1").unwrap(), None);
        let db = r
            .net_checkpoint(&db, "loop", "finish-E1", b"step 1")
            .unwrap();
        let again = r
            .net_checkpoint(&db, "loop", "finish-E1", b"step 1")
            .unwrap();
        assert_eq!(again, db, "a retried checkpoint changes nothing");
        let db = r
            .net_checkpoint(&db, "loop", "finish-E1", b"step 2")
            .unwrap();
        assert_eq!(
            r.net_resume(&db, "loop", "finish-E1").unwrap().as_deref(),
            Some(&b"step 2"[..])
        );
        assert_eq!(r.net_resume(&db, "loop", "finish-E2").unwrap(), None);
    }

    #[test]
    fn lthash_follows_the_marking() {
        let mut r = Repo::new(MemStore::default());
        let net = loop_net();
        let db = r.db_empty().unwrap();
        let mut db = r.net_init_lthash(&db, &net).unwrap();
        for i in 0..5 {
            let tok = format!("E{i}");
            for t in ["propose", "start", "finish", "ask"] {
                db = r
                    .net_fire(&db, &net, t, Some(&tok), &format!("{t}-{tok}"))
                    .unwrap()
                    .0;
            }
        }
        assert_eq!(
            r.net_fire(&db, &net, "ask", Some("E0"), "ask-E0")
                .unwrap()
                .0,
            db
        );
        let (db, _) = r
            .net_fire(&db, &net, "merge", Some("E1"), "merge-E1")
            .unwrap();
        let m = r.net_marking(&db, "loop").unwrap();
        assert_eq!(r.sum_mode(&db, "loop").unwrap(), Some(lt_hash(&m)));
        assert_eq!(r.net_replay(&db, &net).unwrap(), m);
        assert_eq!(r.db_rebuild(&db.log).unwrap().kv, db.kv);
        assert_ne!(lt_hash(&m), sum_hash(&m));
        let sum = r.db_empty().unwrap();
        let sum = r.net_init_sum(&sum, &net).unwrap();
        assert!(r.net_init_lthash(&sum, &net).is_err());
    }

    #[test]
    fn check_refuses_bad_nets() {
        let mut net = loop_net();
        net.transitions[1].inputs[0].place = "nowhere".into();
        assert!(net.check().is_err());
        let mut net = loop_net();
        net.transitions.push(tr("cpu", &["cpu"], &[], Kind::Pure));
        assert!(
            net.check().is_err(),
            "a name shared by a place and a transition"
        );
        let mut net = loop_net();
        net.transitions[4].capability = Some("merge".into());
        assert!(net.check().is_err(), "a human transition has no capability");
        let mut net = loop_net();
        net.places[1].initial = 2;
        assert!(net.check().is_err(), "colored places start empty");
        let mut net = loop_net();
        net.transitions[1].inputs[1].weight = 0;
        assert!(net.check().is_err());
    }
}
