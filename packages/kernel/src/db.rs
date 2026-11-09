//! An append-only event log with views, as a projection of the file store
//! (docs/plans/2026-11-09-db-log-view.md). It brings the April design's edge
//! event log and its last-write-wins fold into the kernel, built only from
//! objects the kernel already has: no new object kind, no format change.
//!
//! - The log is a graph whose node labels are zero-padded sequence numbers
//!   and whose targets are event contents. Appending never changes an
//!   earlier entry.
//! - The `ids` view maps each idempotency key to its event, so a retried
//!   append is a no-op: the same event under a key already logged changes
//!   no root, and a different event under it is refused.
//! - The `kv` view folds the log by last write wins per key; a remove drops
//!   the key, and with `subtree` every key below it (`key/...`). A value is
//!   a byte range of the event that wrote it (an empty value, the empty
//!   content), so read-back is byte-exact and needs no second copy.
//! - Views are projections: `db_rebuild` folds the log from scratch with
//!   `put_graph` and must give the same roots as the views `db_append`
//!   edits in place (E119), which touch only the chunks an event's keys
//!   fall in. Each view records the log position it reflects.
//!
//! In a commit the database is the tree entry `db`: `db/log`, and
//! `db/views/ids` and `db/views/kv` with their positions in `ids.at` and
//! `kv.at`; files beside it are kept as they are.

use std::collections::BTreeMap;

use crate::{Arg, Edge, Error, Id, Kind, Node, Object, Repo, Store, Target, TreeEntry, node_key};

/// Leading bytes of an event's encoding.
const MAGIC: &[u8] = b"ikam-db-event/1\0";
/// Digits of a log label: `u64::MAX` has 20.
const SEQ_DIGITS: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// Set the key to these bytes.
    Upsert(Vec<u8>),
    /// Drop the key; with `subtree`, every key below it too.
    Remove { subtree: bool },
}

/// One log entry. `key` is the identity the fold groups by: segments joined
/// by `/`, none empty; qualifiers such as an environment belong in the key.
/// `idempotency_key` names this write, so a retry can be recognised; it is
/// separate from `key` because two writes to one key are two events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    pub key: String,
    pub op: Op,
    pub idempotency_key: String,
}

/// The roots of one database version: three graphs, and the log position
/// (the sequence number of the last event folded in) each view reflects,
/// so a view may lag the log and catch up later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Db {
    pub log: Id,
    pub ids: Id,
    pub kv: Id,
    pub ids_at: u64,
    pub kv_at: u64,
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.split('/').all(|s| !s.is_empty())
}

impl Event {
    pub fn upsert(key: &str, value: &[u8], idempotency_key: &str) -> Event {
        Event {
            key: key.into(),
            op: Op::Upsert(value.to_vec()),
            idempotency_key: idempotency_key.into(),
        }
    }

    pub fn remove(key: &str, subtree: bool, idempotency_key: &str) -> Event {
        Event {
            key: key.into(),
            op: Op::Remove { subtree },
            idempotency_key: idempotency_key.into(),
        }
    }

    /// Canonical bytes: magic, idempotency key, key, then `U` and the value
    /// (last, so it is the tail a view points at) or `R` and 0/1 for subtree.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        put_str(&mut out, &self.idempotency_key);
        put_str(&mut out, &self.key);
        match &self.op {
            Op::Upsert(value) => {
                out.push(b'U');
                out.extend_from_slice(value);
            }
            Op::Remove { subtree } => out.extend([b'R', *subtree as u8]),
        }
        out
    }

    /// Strict inverse of `encode`, with the value's offset for an upsert.
    pub fn decode(bytes: &[u8]) -> Result<(Event, usize), Error> {
        let bad = || Error::Decode("db event");
        if !bytes.starts_with(MAGIC) {
            return Err(bad());
        }
        let mut at = MAGIC.len();
        let take_str = |at: &mut usize| -> Result<String, Error> {
            let len = bytes.get(*at..*at + 4).ok_or_else(bad)?;
            let len = u32::from_be_bytes(len.try_into().unwrap()) as usize;
            let s = bytes.get(*at + 4..*at + 4 + len).ok_or_else(bad)?;
            *at += 4 + len;
            String::from_utf8(s.to_vec()).map_err(|_| bad())
        };
        let idempotency_key = take_str(&mut at)?;
        let key = take_str(&mut at)?;
        let op = match (bytes.get(at), bytes.get(at + 1..)) {
            (Some(b'U'), Some(value)) => Op::Upsert(value.to_vec()),
            (Some(b'R'), Some([s @ (0 | 1)])) => Op::Remove { subtree: *s == 1 },
            _ => return Err(bad()),
        };
        if !valid_key(&key) || idempotency_key.is_empty() {
            return Err(bad());
        }
        let e = Event {
            key,
            op,
            idempotency_key,
        };
        Ok((e, at + 1))
    }
}

fn seq_label(seq: u64) -> String {
    format!("{seq:0width$}", width = SEQ_DIGITS)
}

fn target_node(label: &str, target: Target, edges: Vec<Edge>) -> Node {
    Node {
        label: label.into(),
        target,
        edges,
    }
}

/// Changes to the kv view: `Some` sets a key's value range, `None` drops it.
type Changes = BTreeMap<String, Option<Arg>>;

/// Apply one event to `changes`. `below` lists the keys under the event's
/// key already in the view, for a subtree remove.
fn fold(
    changes: &mut Changes,
    below: &[String],
    event: &Event,
    event_id: Id,
    value_at: usize,
    len: usize,
) {
    match &event.op {
        // A range may not be empty, so an empty value is the empty content.
        Op::Upsert(_) if value_at == len => {
            changes.insert(event.key.clone(), Some(Arg::Whole(Id::of_content(&[]))));
        }
        Op::Upsert(_) => {
            let range = Arg::Range {
                id: event_id,
                start: value_at as u64,
                len: (len - value_at) as u64,
            };
            changes.insert(event.key.clone(), Some(range));
        }
        Op::Remove { subtree } => {
            changes.insert(event.key.clone(), None);
            if *subtree {
                let prefix = format!("{}/", event.key);
                let pending: Vec<String> = changes
                    .keys()
                    .filter(|k| k.starts_with(&prefix))
                    .cloned()
                    .collect();
                for k in pending.into_iter().chain(below.iter().cloned()) {
                    changes.insert(k, None);
                }
            }
        }
    }
}

/// An event's log, ids and kv nodes: what an append adds.
struct Entry {
    log: Node,
    id: Node,
}

fn entry(seq: u64, e: &Event, id: Id) -> Entry {
    let label = seq_label(seq);
    let seq_edge = Edge {
        to: node_key(&label),
        label: "seq".into(),
    };
    Entry {
        id: target_node(
            &e.idempotency_key,
            Target::Content(Arg::Whole(id)),
            vec![seq_edge],
        ),
        log: target_node(&label, Target::Content(Arg::Whole(id)), Vec::new()),
    }
}

fn parse_seq(label: &str) -> Result<u64, Error> {
    match label.parse() {
        Ok(seq) if label.len() == SEQ_DIGITS => Ok(seq),
        _ => Err(Error::Decode("db log")),
    }
}

impl<S: Store> Repo<S> {
    /// An empty database: three empty graphs, at log position 0.
    pub fn db_empty(&mut self) -> Result<Db, Error> {
        let empty = self.put_graph(Vec::new())?;
        Ok(Db {
            log: empty,
            ids: empty,
            kv: empty,
            ids_at: 0,
            kv_at: 0,
        })
    }

    /// The database in a commit, or an empty one if it has none. A view
    /// with no recorded position (E118 commits) is at the log's end.
    pub fn db_at(&mut self, commit: &Id) -> Result<Db, Error> {
        let Some(db) = self.tree_entry(&self.commit_tree(commit)?, "db")? else {
            return self.db_empty();
        };
        let views = self.tree_entry(&db, "views")?.ok_or(Error::WrongKind(db))?;
        let get = |r: &Self, t: &Id, n: &str| r.tree_entry(t, n)?.ok_or(Error::WrongKind(*t));
        let log = get(self, &db, "log")?;
        let head = self.db_head(&log)?;
        let at = |r: &Self, name: &str| -> Result<u64, Error> {
            match r.tree_entry(&views, name)? {
                Some(id) => String::from_utf8(r.read_content(&id)?)
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .ok_or(Error::Decode("db view position")),
                None => Ok(head),
            }
        };
        Ok(Db {
            log,
            ids: get(self, &views, "ids")?,
            kv: get(self, &views, "kv")?,
            ids_at: at(self, "ids.at")?,
            kv_at: at(self, "kv.at")?,
        })
    }

    /// The sequence number of the log's last event, 0 if it is empty.
    pub fn db_head(&self, log: &Id) -> Result<u64, Error> {
        self.graph_last(log)?.map_or(Ok(0), |n| parse_seq(&n.label))
    }

    /// Append events in order. An event whose idempotency key is logged
    /// with the same bytes is skipped; with other bytes, nothing is
    /// appended and the call fails. Returns the new roots, which equal the
    /// old ones when every event was a retry. Views are edited in place:
    /// an append reads and writes the chunks its keys fall in, not the
    /// database, and gives the roots `db_rebuild` would.
    pub fn db_append(&mut self, db: &Db, events: &[Event]) -> Result<Db, Error> {
        let head = self.db_head(&db.log)?;
        if db.ids_at != head || db.kv_at != head {
            return Err(Error::Decode("db views are behind the log"));
        }
        // Every idempotency key in the batch, looked up in one pass.
        let keys = events.iter().map(|e| e.idempotency_key.clone()).collect();
        let mut seen: BTreeMap<String, Id> = BTreeMap::new();
        for n in self.graph_nodes_in(&db.ids, &keys)? {
            match n.target {
                Target::Content(a) => seen.insert(n.label, a.id()),
                _ => return Err(Error::Decode("db ids")),
            };
        }
        let mut fresh = Vec::new();
        for e in events {
            if !valid_key(&e.key) || e.idempotency_key.is_empty() {
                return Err(Error::InvalidName(e.key.clone()));
            }
            let bytes = e.encode();
            let id = Id::of_content(&bytes);
            match seen.get(&e.idempotency_key) {
                Some(prev) if *prev == id => continue,
                Some(_) => return Err(Error::IdempotencyConflict(e.idempotency_key.clone())),
                None => {}
            }
            seen.insert(e.idempotency_key.clone(), id);
            fresh.push((e, bytes, id));
        }
        if fresh.is_empty() {
            return Ok(*db);
        }
        let (mut log, mut ids, mut changes) = (BTreeMap::new(), BTreeMap::new(), Changes::new());
        let mut seq = head;
        for (e, bytes, id) in fresh {
            seq += 1;
            self.put_content(&bytes)?;
            if matches!(e.op, Op::Upsert(ref v) if v.is_empty()) {
                self.put_content(&[])?;
            }
            let entry = entry(seq, e, id);
            log.insert(entry.log.label.clone(), Some(entry.log));
            ids.insert(entry.id.label.clone(), Some(entry.id));
            let below = match e.op {
                Op::Remove { subtree: true } => self
                    .graph_prefix(&db.kv, &format!("{}/", e.key))?
                    .into_iter()
                    .map(|n| n.label)
                    .collect(),
                _ => Vec::new(),
            };
            fold(
                &mut changes,
                &below,
                e,
                id,
                Event::decode(&bytes)?.1,
                bytes.len(),
            );
        }
        let kv = changes
            .into_iter()
            .map(|(k, a)| {
                let node = a.map(|a| target_node(&k, Target::Content(a), Vec::new()));
                (k, node)
            })
            .collect();
        Ok(Db {
            log: self.graph_edit(&db.log, log)?,
            ids: self.graph_edit(&db.ids, ids)?,
            kv: self.graph_edit(&db.kv, kv)?,
            ids_at: seq,
            kv_at: seq,
        })
    }

    /// The log's events in order, read back from the store.
    pub fn db_events(&self, db: &Db) -> Result<Vec<(u64, Event)>, Error> {
        let mut out = Vec::new();
        for n in self.graph_nodes(&db.log)? {
            let seq = n.label.parse().map_err(|_| Error::Decode("db log"))?;
            let Target::Content(a) = n.target else {
                return Err(Error::Decode("db log"));
            };
            out.push((seq, Event::decode(&self.read_content(&a.id())?)?.0));
        }
        Ok(out)
    }

    /// The value of `key`, byte for byte as written, if the key is set.
    pub fn db_get(&self, db: &Db, key: &str) -> Result<Option<Vec<u8>>, Error> {
        let Some(node) = self.graph_node(&db.kv, key)? else {
            return Ok(None);
        };
        match node.target {
            Target::Content(Arg::Range { id, start, len }) => {
                let bytes = self.read_content(&id)?;
                let value = crate::repo::select(&bytes, start, len);
                value.map(|v| Some(v.to_vec())).ok_or(Error::Corrupt(id))
            }
            Target::Content(Arg::Whole(id)) => Ok(Some(self.read_content(&id)?)),
            _ => Err(Error::Decode("db kv")),
        }
    }

    /// Every key and its value, in key order.
    pub fn db_scan(&self, db: &Db) -> Result<Vec<(String, Vec<u8>)>, Error> {
        let mut out = Vec::new();
        for n in self.graph_nodes(&db.kv)? {
            let value = self.db_get(db, &n.label)?.ok_or(Error::Decode("db kv"))?;
            out.push((n.label, value));
        }
        Ok(out)
    }

    /// Fold a log from scratch with `put_graph`, the reference the views
    /// maintained in place must equal byte for byte.
    pub fn db_rebuild(&mut self, log: &Id) -> Result<Db, Error> {
        let empty = self.put_graph(Vec::new())?;
        let probe = Db {
            log: *log,
            ids: empty,
            kv: empty,
            ids_at: 0,
            kv_at: 0,
        };
        let (mut log_nodes, mut ids, mut changes) = (Vec::new(), Vec::new(), Changes::new());
        let mut seen = BTreeMap::new();
        for (i, (seq, e)) in self.db_events(&probe)?.into_iter().enumerate() {
            let bytes = e.encode();
            let id = Id::of_content(&bytes);
            if seq != i as u64 + 1 || seen.insert(e.idempotency_key.clone(), id).is_some() {
                return Err(Error::Decode(
                    "db log is not a sequence from 1 with unique keys",
                ));
            }
            let entry = entry(seq, &e, id);
            log_nodes.push(entry.log);
            ids.push(entry.id);
            fold(
                &mut changes,
                &[],
                &e,
                id,
                Event::decode(&bytes)?.1,
                bytes.len(),
            );
        }
        let seq = log_nodes.len() as u64;
        let kv = changes
            .into_iter()
            .filter_map(|(k, a)| a.map(|a| target_node(&k, Target::Content(a), Vec::new())))
            .collect();
        let rebuilt = Db {
            log: self.put_graph(log_nodes)?,
            ids: self.put_graph(ids)?,
            kv: self.put_graph(kv)?,
            ids_at: seq,
            kv_at: seq,
        };
        if rebuilt.log != *log {
            return Err(Error::Decode("db log is not canonical"));
        }
        Ok(rebuilt)
    }

    /// Commit `db` to `name`, replacing the `db` entry of the ref's current
    /// tree and keeping every other entry.
    pub fn db_commit(&mut self, name: &str, db: &Db, message: &str) -> Result<Id, Error> {
        let mut entries = match self.store.get_ref(name)? {
            Some(c) => match self.get(&self.commit_tree(&c)?)? {
                Object::Tree(entries) => entries,
                _ => return Err(Error::WrongKind(c)),
            },
            None => Vec::new(),
        };
        entries.retain(|e| e.name != "db");
        let graph = |name: &str, id: Id| TreeEntry {
            name: name.into(),
            kind: Kind::Graph,
            id,
        };
        let at = |r: &mut Self, name: &str, seq: u64| -> Result<TreeEntry, Error> {
            Ok(TreeEntry {
                name: name.into(),
                kind: Kind::File,
                id: r.put_content(seq.to_string().as_bytes())?.id,
            })
        };
        let (ids_at, kv_at) = (at(self, "ids.at", db.ids_at)?, at(self, "kv.at", db.kv_at)?);
        let views = self.put(&Object::tree(vec![
            graph("ids", db.ids),
            graph("kv", db.kv),
            ids_at,
            kv_at,
        ])?)?;
        let views = TreeEntry {
            name: "views".into(),
            kind: Kind::Tree,
            id: views,
        };
        let dir = self.put(&Object::tree(vec![graph("log", db.log), views])?)?;
        entries.push(TreeEntry {
            name: "db".into(),
            kind: Kind::Tree,
            id: dir,
        });
        let tree = self.put(&Object::tree(entries)?)?;
        self.commit(name, tree, message)
    }

    fn commit_tree(&self, commit: &Id) -> Result<Id, Error> {
        match self.get(commit)? {
            Object::Commit(c) => Ok(c.tree),
            _ => Err(Error::WrongKind(*commit)),
        }
    }

    fn tree_entry(&self, tree: &Id, name: &str) -> Result<Option<Id>, Error> {
        match self.get(tree)? {
            Object::Tree(entries) => Ok(entries.into_iter().find(|e| e.name == name).map(|e| e.id)),
            _ => Err(Error::WrongKind(*tree)),
        }
    }
}
