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
//! - Views are projections: `db_rebuild` folds the log from scratch and must
//!   give the same roots as the views maintained on append.
//!
//! In a commit the database is the tree entry `db`: `db/log`, and
//! `db/views/ids` and `db/views/kv`; files beside it are kept as they are.

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

/// The roots of one database version: three graphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Db {
    pub log: Id,
    pub ids: Id,
    pub kv: Id,
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

/// Keys and their value ranges, folded by last write wins.
type Kv = BTreeMap<String, Arg>;

/// Apply one event to the fold.
fn fold(kv: &mut Kv, event: &Event, event_id: Id, value_at: usize, len: usize) {
    match &event.op {
        // A range may not be empty, so an empty value is the empty content.
        Op::Upsert(_) if value_at == len => {
            kv.insert(event.key.clone(), Arg::Whole(Id::of_content(&[])));
        }
        Op::Upsert(_) => {
            let range = Arg::Range {
                id: event_id,
                start: value_at as u64,
                len: (len - value_at) as u64,
            };
            kv.insert(event.key.clone(), range);
        }
        Op::Remove { subtree } => {
            kv.remove(&event.key);
            if *subtree {
                let below = format!("{}/", event.key);
                kv.retain(|k, _| !k.starts_with(&below));
            }
        }
    }
}

impl<S: Store> Repo<S> {
    /// An empty database: three empty graphs.
    pub fn db_empty(&mut self) -> Result<Db, Error> {
        let empty = self.put_graph(Vec::new())?;
        Ok(Db {
            log: empty,
            ids: empty,
            kv: empty,
        })
    }

    /// The database in a commit, or an empty one if it has none.
    pub fn db_at(&mut self, commit: &Id) -> Result<Db, Error> {
        let Some(db) = self.tree_entry(&self.commit_tree(commit)?, "db")? else {
            return self.db_empty();
        };
        let views = self.tree_entry(&db, "views")?.ok_or(Error::WrongKind(db))?;
        let get = |r: &Self, t: &Id, n: &str| r.tree_entry(t, n)?.ok_or(Error::WrongKind(*t));
        Ok(Db {
            log: get(self, &db, "log")?,
            ids: get(self, &views, "ids")?,
            kv: get(self, &views, "kv")?,
        })
    }

    /// Append events in order. An event whose idempotency key is logged
    /// with the same bytes is skipped; with other bytes, nothing is
    /// appended and the call fails. Returns the new roots, which equal the
    /// old ones when every event was a retry.
    pub fn db_append(&mut self, db: &Db, events: &[Event]) -> Result<Db, Error> {
        let mut log = self.graph_nodes(&db.log)?;
        let mut ids = self.graph_nodes(&db.ids)?;
        let mut seen: BTreeMap<String, Id> = ids
            .iter()
            .filter_map(|n| match &n.target {
                Target::Content(a) => Some((n.label.clone(), a.id())),
                _ => None,
            })
            .collect();
        let mut kv = self.db_kv(&db.kv)?;
        let mut next = match log.last() {
            Some(n) => {
                n.label
                    .parse::<u64>()
                    .map_err(|_| Error::Decode("db log"))?
                    + 1
            }
            None => 1,
        };
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
            fresh.push((next, e, bytes, id));
            next += 1;
        }
        if fresh.is_empty() {
            return Ok(*db);
        }
        for (seq, e, bytes, id) in fresh {
            self.put_content(&bytes)?;
            let label = seq_label(seq);
            let seq_edge = Edge {
                to: node_key(&label),
                label: "seq".into(),
            };
            ids.push(target_node(
                &e.idempotency_key,
                Target::Content(Arg::Whole(id)),
                vec![seq_edge],
            ));
            log.push(target_node(
                &label,
                Target::Content(Arg::Whole(id)),
                Vec::new(),
            ));
            let value_at = Event::decode(&bytes)?.1;
            if matches!(e.op, Op::Upsert(ref v) if v.is_empty()) {
                self.put_content(&[])?;
            }
            fold(&mut kv, e, id, value_at, bytes.len());
        }
        Ok(Db {
            log: self.put_graph(log)?,
            ids: self.put_graph(ids)?,
            kv: self.put_kv(kv)?,
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

    /// Fold a log from scratch: the views it implies.
    pub fn db_rebuild(&mut self, log: &Id) -> Result<Db, Error> {
        let empty = self.db_empty()?;
        let events: Vec<Event> = self
            .db_events(&Db { log: *log, ..empty })?
            .into_iter()
            .map(|(_, e)| e)
            .collect();
        let rebuilt = self.db_append(&empty, &events)?;
        if rebuilt.log != *log {
            return Err(Error::Decode("db log is not a sequence from 1"));
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
        let views = self.put(&Object::tree(vec![
            graph("ids", db.ids),
            graph("kv", db.kv),
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

    fn db_kv(&self, root: &Id) -> Result<Kv, Error> {
        let mut kv = Kv::new();
        for n in self.graph_nodes(root)? {
            let Target::Content(a) = n.target else {
                return Err(Error::Decode("db kv"));
            };
            kv.insert(n.label, a);
        }
        Ok(kv)
    }

    fn put_kv(&mut self, kv: Kv) -> Result<Id, Error> {
        let nodes = kv
            .into_iter()
            .map(|(k, a)| target_node(&k, Target::Content(a), Vec::new()))
            .collect();
        self.put_graph(nodes)
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
