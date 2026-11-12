//! Joins kept up to date as both sides change (E243), after Differential
//! Dataflow's shared arrangements (McSherry et al., from what is known of
//! them): a table is indexed by its join key once, as an *arrangement*,
//! and every join that reads it shares that index. A batch of changes
//! updates each arrangement once, then each join only at the join keys the
//! changes touch, pairing the changed rows with the other side's rows
//! there (a delta join: a change on one side reads only the other side).
//!
//! - An arrangement of table `t` (keys `t/...` in the database's `kv`
//!   view) is a graph with a node `jk US rowkey` per row whose join key is
//!   `jk` (US is the unit separator, U+001F), pointing at the same bytes
//!   as the row's `kv` node: no value is copied.
//! - A join's output is a graph with a node `jk US left US right` per
//!   matching pair, its value `combine(left value, right value)`; an
//!   update stores its new values as one content the nodes point into.
//! - `join_recompute` builds the same rows from scratch, the reference an
//!   update must equal (tests/join.rs).
//!
//! Opt-in: nothing in the kernel arranges or joins unless a caller asks.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::{Arg, Db, Error, Id, Node, Repo, Store, Target};

const US: char = '\u{1f}';

/// Rows: key to value.
pub type Rows = BTreeMap<String, Vec<u8>>;

#[derive(Clone)]
pub struct Arrangement {
    pub name: String,
    /// The table: rows are the `kv` keys starting with `table/`.
    pub table: String,
    /// A row's join key, if it has one. May not contain U+001F.
    pub key: fn(&str, &[u8]) -> Option<String>,
}

#[derive(Clone)]
pub struct Join {
    pub name: String,
    /// Arrangement names.
    pub left: String,
    pub right: String,
    pub combine: fn(&[u8], &[u8]) -> Vec<u8>,
}

/// Roots of the arrangements and joins, and the database they reflect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinState {
    pub arranged: BTreeMap<String, Id>,
    pub joins: BTreeMap<String, Id>,
    pub db: Db,
}

fn bad_key(k: &str) -> Error {
    Error::InvalidName(k.to_string())
}

/// Contents read during one update, by id: the old and new products at a
/// key read mostly the same rows.
type Seen = HashMap<Id, Vec<u8>>;
/// Changed rows at one join key: row key to (value before, value after),
/// `None` where the row was not at that key.
type Pairs = BTreeMap<String, (Option<Vec<u8>>, Option<Vec<u8>>)>;
/// An arrangement's changes: join key to its changed rows.
type Touched = BTreeMap<String, Pairs>;
/// One side's rows grouped by join key.
type Sides = BTreeMap<String, Vec<(String, Vec<u8>)>>;

impl<S: Store> Repo<S> {
    fn value_of(&self, target: &Target) -> Result<Vec<u8>, Error> {
        self.value_seen(target, &mut Seen::new())
    }

    fn value_seen(&self, target: &Target, seen: &mut Seen) -> Result<Vec<u8>, Error> {
        let Target::Content(arg) = target else {
            return Err(Error::Decode("join row"));
        };
        let id = arg.id();
        if let std::collections::hash_map::Entry::Vacant(e) = seen.entry(id) {
            e.insert(self.read_content(&id)?);
        }
        let bytes = &seen[&id];
        match arg {
            Arg::Whole(_) => Ok(bytes.clone()),
            Arg::Range { start, len, .. } => Ok(crate::repo::select(bytes, *start, *len)
                .ok_or(Error::Corrupt(id))?
                .to_vec()),
        }
    }

    /// Store derived bytes as plain content: they can be rebuilt, so no
    /// plan is searched for them (the search reads stored contents).
    fn put_derived(&mut self, bytes: &[u8]) -> Result<Id, Error> {
        let id = Id::of_content(bytes);
        if !self.has_content(&id)? {
            let encoded = self.encode_plain(bytes);
            self.materialize(id, &encoded, bytes)?;
        }
        Ok(id)
    }

    /// An arrangement's node for a `kv` node, if the row has a join key.
    fn arranged_node(
        &self,
        a: &Arrangement,
        n: &Node,
        seen: &mut Seen,
    ) -> Result<Option<Node>, Error> {
        let value = self.value_seen(&n.target, seen)?;
        let Some(jk) = (a.key)(&n.label, &value) else {
            return Ok(None);
        };
        if jk.contains(US) || n.label.contains(US) {
            return Err(bad_key(&n.label));
        }
        Ok(Some(Node {
            label: format!("{jk}{US}{}", n.label),
            target: n.target.clone(),
            edges: Vec::new(),
        }))
    }

    fn arrange(&mut self, a: &Arrangement, db: &Db) -> Result<Id, Error> {
        let mut nodes = Vec::new();
        for n in self.graph_prefix(&db.kv, &format!("{}/", a.table))? {
            nodes.extend(self.arranged_node(a, &n, &mut Seen::new())?);
        }
        self.put_graph_with(nodes, crate::Keys::Any)
    }

    /// Rows of an arrangement at one join key: row key to value.
    fn at_key(
        &self,
        root: &Id,
        jk: &str,
        seen: &mut Seen,
    ) -> Result<Vec<(String, Vec<u8>)>, Error> {
        let mut out = Vec::new();
        for n in self.graph_prefix(root, &format!("{jk}{US}"))? {
            let row = n
                .label
                .split_once(US)
                .ok_or(Error::Decode("arrangement"))?
                .1
                .to_string();
            out.push((row, self.value_seen(&n.target, seen)?));
        }
        Ok(out)
    }

    /// Nodes for `rows`, their values stored as one content (as E242).
    fn rows_nodes(&mut self, rows: &Rows) -> Result<Vec<Node>, Error> {
        let (mut blob, mut spans) = (Vec::new(), Vec::new());
        for (k, v) in rows {
            spans.push((k.clone(), blob.len(), v.len()));
            blob.extend_from_slice(v);
        }
        let id = self.put_derived(&blob)?;
        if spans.iter().any(|s| s.2 == 0) {
            self.put_derived(&[])?;
        }
        Ok(spans
            .into_iter()
            .map(|(label, start, len)| Node {
                label,
                target: Target::Content(match len {
                    0 => Arg::Whole(Id::of_content(&[])),
                    _ => Arg::Range {
                        id,
                        start: start as u64,
                        len: len as u64,
                    },
                }),
                edges: Vec::new(),
            })
            .collect())
    }

    /// Every row of a join, from scratch: both tables read, matched in
    /// memory, and stored as one graph. Returns its root and rows.
    pub fn join_recompute(
        &mut self,
        arrs: &[Arrangement],
        j: &Join,
        db: &Db,
    ) -> Result<(Id, Rows), Error> {
        let get = |n: &str| arrs.iter().find(|a| a.name == n).ok_or_else(|| bad_key(n));
        let side = |r: &Self, a: &Arrangement| -> Result<Sides, Error> {
            let mut by: Sides = BTreeMap::new();
            for n in r.graph_prefix(&db.kv, &format!("{}/", a.table))? {
                let v = r.value_of(&n.target)?;
                if let Some(jk) = (a.key)(&n.label, &v) {
                    by.entry(jk).or_default().push((n.label, v));
                }
            }
            Ok(by)
        };
        let (l, r) = (side(self, get(&j.left)?)?, side(self, get(&j.right)?)?);
        let mut out = Rows::new();
        for (jk, ls) in &l {
            for (rk, rv) in r.get(jk).into_iter().flatten() {
                for (lk, lv) in ls {
                    out.insert(format!("{jk}{US}{lk}{US}{rk}"), (j.combine)(lv, rv));
                }
            }
        }
        let nodes = self.rows_nodes(&out)?;
        Ok((self.put_graph_with(nodes, crate::Keys::Any)?, out))
    }

    /// Arrange every table and build every join from `db`.
    pub fn joins_build(
        &mut self,
        arrs: &[Arrangement],
        joins: &[Join],
        db: &Db,
    ) -> Result<JoinState, Error> {
        let mut state = JoinState {
            arranged: BTreeMap::new(),
            joins: BTreeMap::new(),
            db: *db,
        };
        for a in arrs {
            let root = self.arrange(a, db)?;
            state.arranged.insert(a.name.clone(), root);
        }
        for j in joins {
            let root = self.join_recompute(arrs, j, db)?.0;
            state.joins.insert(j.name.clone(), root);
        }
        Ok(state)
    }

    /// Bring `state` up to `db`. Each arrangement takes its table's
    /// changes once; then, at each join key where rows changed, a join
    /// pairs the changed rows with the other side's rows there (a delta
    /// join): a change on one side reads only the other side at its key.
    pub fn joins_update(
        &mut self,
        arrs: &[Arrangement],
        joins: &[Join],
        state: &JoinState,
        db: &Db,
    ) -> Result<JoinState, Error> {
        let changes = self.graph_diff(&state.db.kv, &db.kv)?;
        let mut next = JoinState {
            arranged: state.arranged.clone(),
            joins: state.joins.clone(),
            db: *db,
        };
        let mut touched: BTreeMap<String, Touched> = BTreeMap::new();
        let mut seen = Seen::new();
        for a in arrs {
            let prefix = format!("{}/", a.table);
            let (mut edits, mut moved) = (BTreeMap::new(), Touched::new());
            for (_, before, after) in changes.iter().filter(|c| c.0.starts_with(&prefix)) {
                for (n, set) in [(before, false), (after, true)] {
                    let Some(n) = n else { continue };
                    let Some(an) = self.arranged_node(a, n, &mut seen)? else {
                        continue;
                    };
                    let jk = an.label.split_once(US).unwrap().0.to_string();
                    let value = self.value_seen(&n.target, &mut seen)?;
                    let row = moved
                        .entry(jk)
                        .or_default()
                        .entry(n.label.clone())
                        .or_default();
                    if set {
                        row.1 = Some(value);
                        edits.insert(an.label.clone(), Some(an));
                    } else {
                        row.0 = Some(value);
                        edits.entry(an.label.clone()).or_insert(None);
                    }
                }
            }
            let root = self.graph_edit_with(&state.arranged[&a.name], edits, crate::Keys::Any)?;
            next.arranged.insert(a.name.clone(), root);
            touched.insert(a.name.clone(), moved);
        }
        for j in joins {
            let none = Touched::new();
            let (tl, tr) = (
                touched.get(&j.left).unwrap_or(&none),
                touched.get(&j.right).unwrap_or(&none),
            );
            let keys: BTreeSet<&String> = tl.keys().chain(tr.keys()).collect();
            let (mut edits, mut fresh) = (BTreeMap::new(), Rows::new());
            for jk in keys {
                let (cl, cr) = (tl.get(jk), tr.get(jk));
                // A side's rows at `jk`: the changed ones as (before, after);
                // the rest are read, only if the other side changed here.
                let side = |r: &Self,
                            root: &Id,
                            mine: Option<&Pairs>,
                            other: Option<&Pairs>,
                            seen: &mut Seen|
                 -> Result<Pairs, Error> {
                    let mut out = Pairs::new();
                    if other.is_some() {
                        for (k, v) in r.at_key(root, jk, seen)? {
                            out.insert(k, (Some(v.clone()), Some(v)));
                        }
                    }
                    for (k, p) in mine.into_iter().flatten() {
                        out.insert(k.clone(), p.clone());
                    }
                    Ok(out)
                };
                let left = side(self, &next.arranged[&j.left], cl, cr, &mut seen)?;
                let right = side(self, &next.arranged[&j.right], cr, cl, &mut seen)?;
                for (lk, (lo, ln)) in &left {
                    for (rk, (ro, rn)) in &right {
                        if !cl.is_some_and(|c| c.contains_key(lk))
                            && !cr.is_some_and(|c| c.contains_key(rk))
                        {
                            continue;
                        }
                        let pair = |l: &Option<Vec<u8>>, r: &Option<Vec<u8>>| {
                            l.as_ref().zip(r.as_ref()).map(|(l, r)| (j.combine)(l, r))
                        };
                        let (old, new) = (pair(lo, ro), pair(ln, rn));
                        let label = format!("{jk}{US}{lk}{US}{rk}");
                        match new {
                            Some(v) if old.as_ref() != Some(&v) => {
                                fresh.insert(label, v);
                            }
                            None if old.is_some() => {
                                edits.insert(label, None);
                            }
                            _ => {}
                        }
                    }
                }
            }
            for n in self.rows_nodes(&fresh)? {
                edits.insert(n.label.clone(), Some(n));
            }
            let root = self.graph_edit_with(&state.joins[&j.name], edits, crate::Keys::Any)?;
            next.joins.insert(j.name.clone(), root);
        }
        Ok(next)
    }

    /// A join's rows at `state`.
    pub fn join_rows(&self, state: &JoinState, name: &str) -> Result<Rows, Error> {
        let mut out = Rows::new();
        for n in self.graph_nodes(&state.joins[name])? {
            out.insert(n.label.clone(), self.value_of(&n.target)?);
        }
        Ok(out)
    }
}
