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
//! With [`MergeOptions::lines`] (E417, off by default), a file changed on
//! both sides is merged line by line ([`merge_lines`]): changes to
//! different lines of the base are both kept, and changes to the same
//! lines are a [`Conflict::Lines`]. Without it, as before, any file changed
//! on both sides is a conflict.
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
use std::ops::Range;

use similar::{Algorithm, DiffOp, capture_diff_slices};

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
    /// E417 (`MergeOptions::lines`): a file whose two sides changed the
    /// same lines of the base: each range is base lines (from 0, end
    /// excluded) both sides changed.
    Lines {
        path: String,
        lines: Vec<Range<usize>>,
    },
}

/// How `Repo::merge_with` merges. The default is `Repo::merge`'s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MergeOptions {
    /// E417: merge a file changed on both sides line by line.
    pub lines: bool,
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
        self.merge_with(into, from, message, MergeOptions::default())
    }

    /// `merge` with options (E417).
    pub fn merge_with(
        &mut self,
        into: &str,
        from: &str,
        message: &str,
        opts: MergeOptions,
    ) -> Result<Merged, Error> {
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
        let files = self.merge_entry("", b, o, t, opts, &mut conflicts)?;
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
        opts: MergeOptions,
        conflicts: &mut Vec<Conflict>,
    ) -> Result<Entry, Error> {
        if o == t || b == t {
            return Ok(o);
        }
        if b == o {
            return Ok(t);
        }
        if let (true, Some((Kind::File, bi)), Some((Kind::File, oi)), Some((Kind::File, ti))) =
            (opts.lines, b, o, t)
        {
            let (bb, ob, tb) = (
                self.read_content(&bi)?,
                self.read_content(&oi)?,
                self.read_content(&ti)?,
            );
            return match merge_lines(&bb, &ob, &tb) {
                Some(Ok(bytes)) => Ok(Some((Kind::File, self.put_content(&bytes)?.id))),
                Some(Err(lines)) => {
                    conflicts.push(Conflict::Lines {
                        path: path.into(),
                        lines,
                    });
                    Ok(o)
                }
                None => {
                    conflicts.push(Conflict::Path(path.into()));
                    Ok(o)
                }
            };
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
                self.merge_entry(&sub, get(&bm), get(&om), get(&tm), opts, conflicts)?
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

/// `bytes` as lines, each with its `\n` (the last may have none).
fn lines(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split_inclusive(|&b| b == b'\n').collect()
}

/// One side's change to the base: base lines `base` replaced by `new`.
struct Hunk<'a> {
    base: Range<usize>,
    new: &'a [&'a [u8]],
}

/// The changes from `base` to `side`, in order, each between equal runs
/// (so two hunks of one side never touch).
fn hunks<'a>(base: &[&[u8]], side: &'a [&'a [u8]]) -> Vec<Hunk<'a>> {
    let mut out = Vec::new();
    let (mut b, mut s) = (0, 0);
    let mut push = |b0: usize, b1: usize, s0: usize, s1: usize| {
        if b0 < b1 || s0 < s1 {
            out.push(Hunk {
                base: b0..b1,
                new: &side[s0..s1],
            });
        }
    };
    for op in capture_diff_slices(Algorithm::Myers, base, side) {
        if let DiffOp::Equal {
            old_index,
            new_index,
            len,
        } = op
        {
            push(b, old_index, s, new_index);
            (b, s) = (old_index + len, new_index + len);
        }
    }
    push(b, base.len(), s, side.len());
    out
}

/// Do two changes from different sides touch the same base lines? Two
/// replacements clash when their ranges overlap; an insertion clashes with
/// a replacement it falls strictly inside, and with another insertion at
/// the same place (their order would be a guess). Changes to adjacent
/// lines do not clash.
fn clash(a: &Range<usize>, b: &Range<usize>) -> bool {
    match (a.is_empty(), b.is_empty()) {
        (false, false) => a.start < b.end && b.start < a.end,
        (true, false) => b.start < a.start && a.start < b.end,
        (false, true) => a.start < b.start && b.start < a.end,
        (true, true) => a.start == b.start,
    }
}

/// E417: a three-way merge of one file by lines. `Some(Ok(bytes))` holds
/// both sides' changes to `base` applied together; `Some(Err(ranges))`
/// names the base lines both sides changed differently (a change made
/// identically on both sides is kept once). `None`: a side holds a NUL
/// byte, so it is not treated as text.
pub fn merge_lines(
    base: &[u8],
    ours: &[u8],
    theirs: &[u8],
) -> Option<Result<Vec<u8>, Vec<Range<usize>>>> {
    if [base, ours, theirs].iter().any(|b| b.contains(&0)) {
        return None;
    }
    let (bl, ol, tl) = (lines(base), lines(ours), lines(theirs));
    let (ho, ht) = (hunks(&bl, &ol), hunks(&bl, &tl));
    let mut clashes: Vec<Range<usize>> = Vec::new();
    let mut same = vec![false; ht.len()];
    let mut from = 0;
    for a in &ho {
        while from < ht.len() && ht[from].base.end < a.base.start {
            from += 1;
        }
        for (j, b) in ht.iter().enumerate().skip(from) {
            if b.base.start > a.base.end {
                break;
            }
            if a.base == b.base && a.new == b.new {
                same[j] = true;
            } else if clash(&a.base, &b.base) {
                clashes.push(a.base.start.min(b.base.start)..a.base.end.max(b.base.end));
            }
        }
    }
    if !clashes.is_empty() {
        clashes.sort_by_key(|r| (r.start, r.end));
        let mut joined: Vec<Range<usize>> = Vec::new();
        for r in clashes {
            match joined.last_mut() {
                Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
                _ => joined.push(r),
            }
        }
        return Some(Err(joined));
    }
    let mut all: Vec<&Hunk> = ho.iter().collect();
    all.extend(ht.iter().zip(&same).filter(|(_, s)| !**s).map(|(h, _)| h));
    all.sort_by_key(|h| (h.base.start, h.base.end));
    let mut out = Vec::with_capacity(ours.len().max(theirs.len()));
    let mut at = 0;
    for h in all {
        for l in &bl[at..h.base.start] {
            out.extend_from_slice(l);
        }
        for l in h.new {
            out.extend_from_slice(l);
        }
        at = h.base.end;
    }
    for l in &bl[at..] {
        out.extend_from_slice(l);
    }
    Some(Ok(out))
}
