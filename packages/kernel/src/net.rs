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

mod text;

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
        for p in &self.places {
            if !valid_name(&p.name) || !seen.insert(p.name.as_str()) {
                return Err(invalid("place", &p.name));
            }
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
                if self.place(&a.place).is_none() {
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
    format!(
        "{}\n{}\n{}\n{}\n",
        f.transition,
        f.token.as_deref().unwrap_or(""),
        f.before,
        f.after
    )
    .into_bytes()
}

fn decode_record(key: &str, bytes: &[u8]) -> Result<Firing, Error> {
    let s = std::str::from_utf8(bytes).map_err(|_| Error::Decode("net firing"))?;
    let l: Vec<&str> = s.lines().collect();
    let id = |s: &str| s.parse::<Id>();
    if l.len() != 4 {
        return Err(Error::Decode("net firing"));
    }
    Ok(Firing {
        key: key.into(),
        transition: l[0].into(),
        token: (!l[1].is_empty()).then(|| l[1].to_string()),
        before: id(l[2])?,
        after: id(l[3])?,
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
        if let Some(bytes) = self.db_get(db, &rec_key)? {
            let f = decode_record(key, &bytes)?;
            if f.transition != transition || f.token.as_deref() != token {
                return Err(Error::IdempotencyConflict(rec_key));
            }
            return Ok((*db, f));
        }
        let t = net
            .transition(transition)
            .ok_or_else(|| invalid("transition", transition))?;
        let before = self.net_marking(db, &net.name)?;
        let after = net
            .fire(&before, t, token)
            .ok_or_else(|| Error::Exec(format!("transition {transition} is not enabled")))?;
        let f = Firing {
            key: key.into(),
            transition: transition.into(),
            token: token.map(String::from),
            before: marking_id(&before),
            after: marking_id(&after),
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
        events.push(Event::upsert(&rec_key, &encode_record(&f), &rec_key));
        Ok((self.db_append(db, &events)?, f))
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
        let mut m = net.initial_marking();
        for f in self.net_firings(db, &net.name)? {
            let t = net
                .transition(&f.transition)
                .ok_or_else(|| invalid("recorded transition", &f.transition))?;
            if marking_id(&m) != f.before {
                return Err(Error::Decode("net replay: marking before differs"));
            }
            m = net
                .fire(&m, t, f.token.as_deref())
                .ok_or(Error::Decode("net replay: recorded firing not enabled"))?;
            if marking_id(&m) != f.after {
                return Err(Error::Decode("net replay: marking after differs"));
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
