//! Merging one branch into another (E265, opt-in, library only).
//!
//! A branch is a ref. `Repo::merge` finds a common ancestor of the two
//! heads and merges in two parts:
//!
//! - **Files:** a three-way merge of the trees by path. A subtree with the
//!   same id on two sides is not read, so the work grows with the paths
//!   changed since the fork, not with the tree. A path changed on both
//!   sides to different entries is a conflict.
//! - **Database:** the events the other branch logged since the fork are
//!   appended after this branch's head (`db_append`, so a retry logged on
//!   both sides with the same bytes is kept once). A key written on both
//!   sides by different operations is a conflict, as is an idempotency key
//!   logged with different bytes on each side. Only the events since the
//!   fork are read.
//!
//! Conflicts are reported and nothing is written: no merge resolves one
//! silently. Otherwise the result is a commit with both heads as parents,
//! and the ref moves by compare-and-swap. A merge into an ancestor is a
//! fast-forward; a merge of an ancestor changes nothing.
//!
//! The common ancestor is the first commit found by walking both histories
//! breadth-first in turn; with criss-cross histories it may not be the
//! best one. Both branches' views must be current (as `db_append` needs).

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use crate::{Commit, Db, Error, Event, Id, Kind, Object, Op, Repo, Store, TreeEntry};

/// Why a merge was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conflict {
    /// A file or directory changed on both sides to different entries.
    Path(String),
    /// A database key written on both sides by different operations.
    Key(String),
    /// An idempotency key logged on both sides with different bytes.
    Idempotency(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Merged {
    /// The other head already is in this branch's history.
    UpToDate,
    /// This branch's head was an ancestor; the ref now points at the other.
    FastForward(Id),
    /// A merge commit with both heads as parents.
    Commit(Id),
    /// Nothing written.
    Conflicts(Vec<Conflict>),
}

type Entry = Option<(Kind, Id)>;

/// The keys one side's events write: single keys, and `[lo, hi)` spans.
#[derive(Default)]
struct Touched {
    points: BTreeMap<String, Event>,
    spans: Vec<(String, String, Event)>,
}

fn spans(e: &Event) -> Vec<(String, String)> {
    match &e.op {
        Op::Remove { subtree: true } => vec![(format!("{}/", e.key), format!("{}0", e.key))],
        Op::RemoveRange { end } => vec![(e.key.clone(), end.clone())],
        _ => Vec::new(),
    }
}

impl Touched {
    fn new(events: &[Event]) -> Touched {
        let mut t = Touched::default();
        for e in events {
            if !matches!(e.op, Op::RemoveRange { .. }) {
                t.points.insert(e.key.clone(), e.clone());
            }
            for (lo, hi) in spans(e) {
                t.spans.push((lo, hi, e.clone()));
            }
        }
        t
    }

    /// Does `e` write a key this side wrote with another operation?
    fn clashes(&self, e: &Event) -> bool {
        let other = |o: &Event| o.key != e.key || o.op != e.op;
        let point = !matches!(e.op, Op::RemoveRange { .. });
        if point && self.points.get(&e.key).is_some_and(other) {
            return true;
        }
        if point
            && self
                .spans
                .iter()
                .any(|(lo, hi, o)| *lo <= e.key && e.key < *hi && other(o))
        {
            return true;
        }
        spans(e).iter().any(|(lo, hi)| {
            self.points
                .range(lo.clone()..hi.clone())
                .any(|(_, o)| other(o))
                || self
                    .spans
                    .iter()
                    .any(|(l, h, o)| l < hi && lo < h && other(o))
        })
    }
}

impl<S: Store> Repo<S> {
    /// Create the ref `name` at `commit`: a branch. Fails if it exists.
    pub fn branch(&mut self, name: &str, commit: Id) -> Result<(), Error> {
        self.store.set_ref(name, None, commit)
    }

    /// A common ancestor of two commits, found by walking both histories
    /// breadth-first in turn, or `None` if they share none.
    pub fn merge_base(&self, a: Id, b: Id) -> Result<Option<Id>, Error> {
        let (mut seen, mut queue) = (
            [HashSet::from([a]), HashSet::from([b])],
            [VecDeque::from([a]), VecDeque::from([b])],
        );
        if a == b {
            return Ok(Some(a));
        }
        while !queue[0].is_empty() || !queue[1].is_empty() {
            for side in 0..2 {
                let Some(id) = queue[side].pop_front() else {
                    continue;
                };
                if seen[1 - side].contains(&id) {
                    return Ok(Some(id));
                }
                let Object::Commit(c) = self.get(&id)? else {
                    return Err(Error::WrongKind(id));
                };
                for p in c.parents {
                    if seen[1 - side].contains(&p) {
                        return Ok(Some(p));
                    }
                    if seen[side].insert(p) {
                        queue[side].push_back(p);
                    }
                }
            }
        }
        Ok(None)
    }

    /// Merge the commit `from` (a ref name or id) into the ref `into`.
    pub fn merge(&mut self, into: &str, from: &str, message: &str) -> Result<Merged, Error> {
        let ours = self.resolve(into)?;
        let theirs = self.resolve(from)?;
        let base = self.merge_base(ours, theirs)?;
        if base == Some(theirs) {
            return Ok(Merged::UpToDate);
        }
        if base == Some(ours) {
            self.store.set_ref(into, Some(ours), theirs)?;
            return Ok(Merged::FastForward(theirs));
        }
        let mut conflicts = Vec::new();
        let tree = |r: &Self, c: Option<Id>| -> Result<Entry, Error> {
            match c {
                Some(c) => match r.get(&c)? {
                    Object::Commit(c) => Ok(Some((Kind::Tree, c.tree))),
                    _ => Err(Error::WrongKind(c)),
                },
                None => Ok(None),
            }
        };
        let (b, o, t) = (
            tree(self, base)?,
            tree(self, Some(ours))?,
            tree(self, Some(theirs))?,
        );
        let files = self.merge_entry("", b, o, t, &mut conflicts)?;
        let empty = |r: &mut Self| r.db_empty();
        let db_of = |r: &mut Self, c: Option<Id>| match c {
            Some(c) => r.db_at(&c),
            None => empty(r),
        };
        let (bd, od, td) = (
            db_of(self, base)?,
            db_of(self, Some(ours))?,
            db_of(self, Some(theirs))?,
        );
        let db = self.merge_db(&bd, &od, &td, &mut conflicts)?;
        if !conflicts.is_empty() {
            return Ok(Merged::Conflicts(conflicts));
        }
        let mut entries = match files {
            Some((Kind::Tree, id)) => match self.get(&id)? {
                Object::Tree(e) => e,
                _ => return Err(Error::WrongKind(id)),
            },
            _ => Vec::new(),
        };
        entries.retain(|e| e.name != "db");
        if db != self.db_empty()? {
            entries.push(self.db_entry(&db)?);
        }
        let tree = self.put(&Object::tree(entries)?)?;
        let commit = Commit {
            tree,
            parents: vec![ours, theirs],
            message: message.into(),
        };
        let id = self.put(&Object::Commit(commit))?;
        self.store.set_ref(into, Some(ours), id)?;
        Ok(Merged::Commit(id))
    }

    /// Three-way merge of one tree entry. The top level's `db` entry is
    /// left to `merge_db`.
    fn merge_entry(
        &mut self,
        path: &str,
        b: Entry,
        o: Entry,
        t: Entry,
        conflicts: &mut Vec<Conflict>,
    ) -> Result<Entry, Error> {
        if o == t || b == t {
            return Ok(o);
        }
        if b == o {
            return Ok(t);
        }
        let dir = |e: &Entry| matches!(e, Some((Kind::Tree, _)));
        if !(dir(&o) && dir(&t) && (b.is_none() || dir(&b))) {
            conflicts.push(Conflict::Path(path.into()));
            return Ok(o);
        }
        let load = |r: &Self, e: &Entry| -> Result<BTreeMap<String, (Kind, Id)>, Error> {
            let Some((Kind::Tree, id)) = e else {
                return Ok(BTreeMap::new());
            };
            match r.get(id)? {
                Object::Tree(es) => Ok(es.into_iter().map(|e| (e.name, (e.kind, e.id))).collect()),
                _ => Err(Error::WrongKind(*id)),
            }
        };
        let (bm, om, tm) = (load(self, &b)?, load(self, &o)?, load(self, &t)?);
        let names: BTreeSet<&String> = bm.keys().chain(om.keys()).chain(tm.keys()).collect();
        let mut out = Vec::new();
        for name in names {
            if path.is_empty() && name == "db" {
                continue;
            }
            let sub = if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            };
            let get = |m: &BTreeMap<String, (Kind, Id)>| m.get(name).copied();
            if let Some((kind, id)) =
                self.merge_entry(&sub, get(&bm), get(&om), get(&tm), conflicts)?
            {
                out.push(TreeEntry {
                    name: name.clone(),
                    kind,
                    id,
                });
            }
        }
        Ok(Some((Kind::Tree, self.put(&Object::tree(out)?)?)))
    }

    /// Append `theirs`' events since `base` to `ours`, or report the keys
    /// both sides wrote. Reads only the events since the fork.
    fn merge_db(
        &mut self,
        base: &Db,
        ours: &Db,
        theirs: &Db,
        conflicts: &mut Vec<Conflict>,
    ) -> Result<Db, Error> {
        let at = self.db_head(&base.log)?;
        let (mine, other) = (
            self.db_events_after(ours, base, at)?,
            self.db_events_after(theirs, base, at)?,
        );
        if other.is_empty() {
            return Ok(*ours);
        }
        let idem: BTreeMap<&str, Vec<u8>> = mine
            .iter()
            .map(|e| (e.idempotency_key.as_str(), e.encode()))
            .collect();
        let touched = Touched::new(&mine);
        let before = conflicts.len();
        for e in &other {
            match idem.get(e.idempotency_key.as_str()) {
                Some(bytes) if *bytes == e.encode() => continue,
                Some(_) => conflicts.push(Conflict::Idempotency(e.idempotency_key.clone())),
                None if touched.clashes(e) => conflicts.push(Conflict::Key(e.key.clone())),
                None => {}
            }
        }
        if conflicts.len() > before {
            return Ok(*ours);
        }
        self.db_append(ours, &other)
    }
}
