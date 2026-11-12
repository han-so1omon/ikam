//! A graph of named derived views over the database (E242), after dbt's
//! models: each view names the views it reads (or `kv`, the database's
//! key-value view), so the views form a dependency graph that is built in
//! order. Each view has a materialization:
//!
//! - `OnRead`: nothing stored; computed from its inputs when read.
//! - `Stored`: stored as a graph and rebuilt from its inputs on update.
//! - `Incremental`: stored as a graph and updated from its inputs'
//!   changes only (a delta), reading just the rows the changes touch.
//!
//! A view is a `Map` (each input row to at most one output row, output
//! keys distinct) or a `GroupSum` (rows grouped by a key, keeping a count
//! and a sum). A view may carry a row check, run on every row after a
//! build and on every changed row after an update, as dbt's tests run
//! after each model. An update gives the same rows as a full build
//! (tests/views.rs), so incremental views are a cost choice, not a
//! different answer. Each build or update stores its rows' values as one
//! content that the rows point into, so roots tell where values were
//! written as well as what they are: a rebuild gives a build's roots, an
//! incremental update the same rows under other roots.
//!
//! Opt-in: nothing in the kernel defines or updates views unless a caller
//! does.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Arg, Db, Error, Id, Node, Repo, Store, Target};

/// Rows: key to value.
pub type Rows = BTreeMap<String, Vec<u8>>;
/// Changed rows: key to (before, after).
type Delta = BTreeMap<String, (Option<Vec<u8>>, Option<Vec<u8>>)>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    OnRead,
    Stored,
    Incremental,
}

#[derive(Clone, Copy)]
pub enum ViewOp {
    /// An input row to at most one output row. Distinct input keys must
    /// give distinct output keys.
    Map(MapFn),
    /// An input row to a group and a measure; the view keeps
    /// `count,sum` per group, and drops a group whose count falls to 0.
    GroupSum(GroupFn),
}

/// A row to its output row, if any.
pub type MapFn = fn(&str, &[u8]) -> Option<(String, Vec<u8>)>;
/// A row to its group and measure, if any.
pub type GroupFn = fn(&str, &[u8]) -> Option<(String, i64)>;

/// A row check: an error message for a bad row.
pub type Check = fn(&str, &[u8]) -> Result<(), String>;

#[derive(Clone)]
pub struct View {
    pub name: String,
    /// The view read: `kv` or another view's name.
    pub input: String,
    pub mode: Mode,
    pub op: ViewOp,
    pub check: Option<Check>,
}

/// Views in an order where each comes after its input.
pub struct ViewGraph {
    views: Vec<View>,
}

/// The stored views' roots, and the database they reflect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewState {
    pub roots: BTreeMap<String, Id>,
    pub db: Db,
}

impl ViewGraph {
    /// Order `views` by their inputs. Fails on an unknown input, a
    /// repeated name, a cycle, or a stored view reading an `OnRead` one.
    pub fn new(mut views: Vec<View>) -> Result<ViewGraph, Error> {
        let bad = |n: &str| Err(Error::InvalidName(n.to_string()));
        let mut done: BTreeMap<String, Mode> = BTreeMap::new();
        let mut order = Vec::new();
        while !views.is_empty() {
            let ready = views
                .iter()
                .position(|v| v.input == "kv" || done.contains_key(&v.input));
            let Some(i) = ready else {
                return bad(&views[0].name);
            };
            let v = views.remove(i);
            let input_mode = done.get(&v.input).copied();
            if v.name == "kv" || done.contains_key(&v.name) {
                return bad(&v.name);
            }
            if v.mode != Mode::OnRead && input_mode == Some(Mode::OnRead) {
                return bad(&v.name);
            }
            done.insert(v.name.clone(), v.mode);
            order.push(v);
        }
        Ok(ViewGraph { views: order })
    }

    fn get(&self, name: &str) -> Result<&View, Error> {
        self.views
            .iter()
            .find(|v| v.name == name)
            .ok_or(Error::InvalidName(name.into()))
    }
}

fn group_value(count: i64, sum: i64) -> Vec<u8> {
    format!("{count},{sum}").into_bytes()
}

fn parse_group(v: &[u8]) -> Result<(i64, i64), Error> {
    let bad = || Error::Decode("view group");
    let s = std::str::from_utf8(v).map_err(|_| bad())?;
    let (c, s) = s.split_once(',').ok_or_else(bad)?;
    Ok((c.parse().map_err(|_| bad())?, s.parse().map_err(|_| bad())?))
}

/// A view's rows from its input's rows.
fn compute(op: ViewOp, input: &Rows) -> Result<Rows, Error> {
    let mut out = Rows::new();
    match op {
        ViewOp::Map(f) => {
            for (k, v) in input {
                if let Some((key, value)) = f(k, v)
                    && out.insert(key.clone(), value).is_some()
                {
                    return Err(Error::InvalidName(key));
                }
            }
        }
        ViewOp::GroupSum(f) => {
            let mut groups: BTreeMap<String, (i64, i64)> = BTreeMap::new();
            for (k, v) in input {
                if let Some((g, m)) = f(k, v) {
                    let e = groups.entry(g).or_default();
                    *e = (e.0 + 1, e.1 + m);
                }
            }
            out.extend(groups.into_iter().map(|(g, (c, s))| (g, group_value(c, s))));
        }
    }
    Ok(out)
}

fn run_check<'a>(
    v: &View,
    rows: impl Iterator<Item = (&'a String, &'a Vec<u8>)>,
) -> Result<(), Error> {
    let Some(check) = v.check else { return Ok(()) };
    for (k, val) in rows {
        check(k, val).map_err(|m| Error::Exec(format!("view {} row {k}: {m}", v.name)))?;
    }
    Ok(())
}

impl<S: Store> Repo<S> {
    fn node_value(&self, n: &Node) -> Result<Vec<u8>, Error> {
        match &n.target {
            Target::Content(Arg::Whole(id)) => self.read_content(id),
            Target::Content(Arg::Range { id, start, len }) => {
                let bytes = self.read_content(id)?;
                Ok(crate::repo::select(&bytes, *start, *len)
                    .ok_or(Error::Corrupt(*id))?
                    .to_vec())
            }
            _ => Err(Error::Decode("view row")),
        }
    }

    /// Store derived bytes as plain content: a view can be rebuilt, so no
    /// plan is searched for its values (the search reads stored contents).
    fn put_derived(&mut self, bytes: &[u8]) -> Result<Id, Error> {
        let id = Id::of_content(bytes);
        if !self.has_content(&id)? {
            let encoded = self.encode_plain(bytes);
            self.materialize(id, &encoded, bytes)?;
        }
        Ok(id)
    }

    fn graph_rows(&self, root: &Id) -> Result<Rows, Error> {
        let mut out = Rows::new();
        for n in self.graph_nodes(root)? {
            out.insert(n.label.clone(), self.node_value(&n)?);
        }
        Ok(out)
    }

    /// Nodes for `rows`, their values stored as one content and each node
    /// a range of it (as the database's `kv` view points into events), so
    /// a build or an update stores one content, not one per row.
    fn row_nodes<'a>(
        &mut self,
        rows: impl Iterator<Item = (&'a String, &'a Vec<u8>)>,
    ) -> Result<Vec<Node>, Error> {
        let (mut blob, mut spans) = (Vec::new(), Vec::new());
        for (k, v) in rows {
            spans.push((k.clone(), blob.len(), v.len()));
            blob.extend_from_slice(v);
        }
        let id = self.put_derived(&blob)?;
        let empty = Id::of_content(&[]);
        if spans.iter().any(|s| s.2 == 0) {
            self.put_derived(&[])?;
        }
        let node = |(label, start, len): (String, usize, usize)| Node {
            label,
            target: Target::Content(match len {
                0 => Arg::Whole(empty),
                _ => Arg::Range {
                    id,
                    start: start as u64,
                    len: len as u64,
                },
            }),
            edges: Vec::new(),
        };
        Ok(spans.into_iter().map(node).collect())
    }

    fn put_rows(&mut self, rows: &Rows) -> Result<Id, Error> {
        let nodes = self.row_nodes(rows.iter())?;
        self.put_graph(nodes)
    }

    /// A view's rows at `state`: read from its graph, or computed.
    pub fn view_rows(&self, g: &ViewGraph, state: &ViewState, name: &str) -> Result<Rows, Error> {
        if name == "kv" {
            return Ok(self.db_scan(&state.db)?.into_iter().collect());
        }
        let v = g.get(name)?;
        match state.roots.get(name) {
            Some(root) => self.graph_rows(root),
            None => compute(v.op, &self.view_rows(g, state, &v.input)?),
        }
    }

    /// Build every stored and incremental view from `db`, in order, and
    /// run each view's check on all its rows.
    pub fn views_build(&mut self, g: &ViewGraph, db: &Db) -> Result<ViewState, Error> {
        let mut state = ViewState {
            roots: BTreeMap::new(),
            db: *db,
        };
        for v in &g.views {
            let rows = compute(v.op, &self.view_rows(g, &state, &v.input)?)?;
            run_check(v, rows.iter())?;
            if v.mode != Mode::OnRead {
                let root = self.put_rows(&rows)?;
                state.roots.insert(v.name.clone(), root);
            }
        }
        Ok(state)
    }

    /// Bring `state` up to `db`. Stored views are rebuilt; incremental ones
    /// apply their input's changes. Checks run on changed rows.
    pub fn views_update(
        &mut self,
        g: &ViewGraph,
        state: &ViewState,
        db: &Db,
    ) -> Result<ViewState, Error> {
        let mut deltas: BTreeMap<String, Delta> = BTreeMap::new();
        deltas.insert("kv".into(), self.kv_delta(&state.db.kv, &db.kv)?);
        let mut next = ViewState {
            roots: state.roots.clone(),
            db: *db,
        };
        for v in &g.views {
            if v.mode == Mode::OnRead {
                continue;
            }
            let old = state.roots[&v.name];
            let input = &deltas[&v.input];
            let (root, delta) = if input.is_empty() {
                (old, Delta::new())
            } else if v.mode == Mode::Stored {
                let rows = compute(v.op, &self.view_rows(g, &next, &v.input)?)?;
                let root = self.put_rows(&rows)?;
                (root, self.kv_delta(&old, &root)?)
            } else {
                let delta = self.view_delta(v, &old, input)?;
                let set = delta
                    .iter()
                    .filter_map(|(k, (_, a))| a.as_ref().map(|a| (k, a)));
                let mut edits: BTreeMap<String, Option<Node>> = self
                    .row_nodes(set)?
                    .into_iter()
                    .map(|n| (n.label.clone(), Some(n)))
                    .collect();
                for (k, (_, a)) in &delta {
                    if a.is_none() {
                        edits.insert(k.clone(), None);
                    }
                }
                (self.graph_edit(&old, edits)?, delta)
            };
            run_check(
                v,
                delta
                    .iter()
                    .filter_map(|(k, (_, a))| a.as_ref().map(|a| (k, a))),
            )?;
            next.roots.insert(v.name.clone(), root);
            deltas.insert(v.name.clone(), delta);
        }
        Ok(next)
    }

    /// Rows that differ between two graphs of rows, with their values.
    fn kv_delta(&self, a: &Id, b: &Id) -> Result<Delta, Error> {
        let mut out = Delta::new();
        for (label, before, after) in self.graph_diff(a, b)? {
            let before = before.map(|n| self.node_value(&n)).transpose()?;
            let after = after.map(|n| self.node_value(&n)).transpose()?;
            if before != after {
                out.insert(label, (before, after));
            }
        }
        Ok(out)
    }

    /// A view's changes from its input's, reading only the groups touched.
    fn view_delta(&self, v: &View, old: &Id, input: &Delta) -> Result<Delta, Error> {
        let mut out = Delta::new();
        match v.op {
            ViewOp::Map(f) => {
                for (k, (before, after)) in input {
                    if let Some((key, val)) = before.as_ref().and_then(|b| f(k, b)) {
                        out.entry(key).or_insert((Some(val), None));
                    }
                    if let Some((key, val)) = after.as_ref().and_then(|a| f(k, a)) {
                        out.entry(key).or_insert((None, None)).1 = Some(val);
                    }
                }
            }
            ViewOp::GroupSum(f) => {
                let mut moves: BTreeMap<String, (i64, i64)> = BTreeMap::new();
                for (k, (before, after)) in input {
                    for (row, sign) in [(before, -1), (after, 1)] {
                        if let Some((g, m)) = row.as_ref().and_then(|r| f(k, r)) {
                            let e = moves.entry(g).or_default();
                            *e = (e.0 + sign, e.1 + sign * m);
                        }
                    }
                }
                let groups: BTreeSet<String> = moves.keys().cloned().collect();
                let mut current = BTreeMap::new();
                for n in self.graph_nodes_in(old, &groups)? {
                    current.insert(n.label.clone(), self.node_value(&n)?);
                }
                for (g, (dc, ds)) in moves {
                    let before = current.remove(&g);
                    let (c, s) = before
                        .as_deref()
                        .map(parse_group)
                        .transpose()?
                        .unwrap_or((0, 0));
                    let after = (c + dc > 0).then(|| group_value(c + dc, s + ds));
                    out.insert(g, (before, after));
                }
            }
        }
        out.retain(|_, (b, a)| b != a);
        Ok(out)
    }
}
