//! Rebuild only what a change reaches (E429, an option).
//!
//! After Pachyderm's per-datum lineage: an output remembers which inputs it
//! came from, so a change reruns only the outputs that read a changed input.
//! Two lineages, from a database's rows to what is built from them:
//!
//! - Rows to a training table (E426). The table is built in *units*
//!   ([`Unit`]): the history is cut into the units' keys ([`History::units`],
//!   only the commits the rule reads, no commit ids), and each unit's table
//!   is the derivation `train-table/1(rule, unit)`. A unit whose keys did
//!   not change has the same bytes, so its derivation is the one already
//!   recorded and is not evaluated again: content addressing is the
//!   lineage. [`changed_keys`](crate::train::changed_keys) gives the same
//!   answer forward, from the two histories, as `propagate` does for
//!   fragments. The units' tables are listed in a manifest, whose id is the
//!   same whether the table was built at once or a unit at a time.
//! - Fragments to sampled batches (E427). A batch is `sample(view, spec)`, a
//!   function of the whole view, so a new view makes every batch a new
//!   derivation. With the view numbered by paths ([`Order::Paths`]) a
//!   batch is a function of the nodes it holds, the spec and the view's
//!   targets alone. A [`BatchIndex`] keeps which batches hold each node;
//!   [`view_delta`] names the nodes whose batch record changed (id, kind,
//!   label, size bits, neighbors); [`Repo::resample`] samples again the
//!   batches that hold one and records every other one as
//!   `sample(new view, spec)` with its old output, unevaluated. fsck
//!   evaluates those records like any other.
//!
//! Nothing in the default paths calls this module.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::sampler::{Batch, Order, Spec, View};
use crate::train::{History, TrainRule};
use crate::{Arg, Commit, Derivation, Error, Id, Kind, Object, Repo, Store, TreeEntry, func};

const UNITS_MAGIC: &[u8] = b"ikam-train-units/1\0";

/// How a table's keys are grouped into units of rebuild.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unit {
    /// One unit a key.
    Row,
    /// Keys sharing their first `n` bytes (a key shorter than that is its
    /// own unit).
    Prefix(usize),
    /// One unit for every key.
    Table,
}

impl Unit {
    /// Canonical text, kept in a units table's manifest.
    pub fn text(&self) -> String {
        match self {
            Unit::Row => "row".into(),
            Unit::Prefix(n) => format!("prefix {n}"),
            Unit::Table => "table".into(),
        }
    }

    pub fn parse(s: &str) -> Result<Unit, Error> {
        let bad = || Error::Exec(format!("unit {s:?}"));
        Ok(match s {
            "row" => Unit::Row,
            "table" => Unit::Table,
            _ => Unit::Prefix(
                s.strip_prefix("prefix ")
                    .ok_or_else(bad)?
                    .parse()
                    .map_err(|_| bad())?,
            ),
        })
    }

    /// The unit `key` belongs to.
    pub fn of<'a>(&self, key: &'a str) -> &'a str {
        match self {
            Unit::Row => key,
            Unit::Prefix(n) => key.get(..*n).unwrap_or(key),
            Unit::Table => "",
        }
    }
}

/// One unit of a units table: its name, its slice of the history and its
/// table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitEntry {
    pub name: String,
    pub shard: Id,
    pub table: Id,
}

/// A units table's manifest: the rule, the unit, and every unit's slice
/// and table in name order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub rule: Id,
    pub unit: String,
    pub units: Vec<UnitEntry>,
}

impl Manifest {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = UNITS_MAGIC.to_vec();
        out.extend_from_slice(self.rule.as_bytes());
        let put = |out: &mut Vec<u8>, s: &str| {
            out.extend_from_slice(&(s.len() as u32).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        };
        put(&mut out, &self.unit);
        out.extend_from_slice(&(self.units.len() as u32).to_le_bytes());
        for u in &self.units {
            put(&mut out, &u.name);
            out.extend_from_slice(u.shard.as_bytes());
            out.extend_from_slice(u.table.as_bytes());
        }
        out
    }

    pub fn decode(b: &[u8]) -> Result<Manifest, Error> {
        fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8], Error> {
            if b.len() < n {
                return Err(Error::Decode("train units"));
            }
            let (h, r) = b.split_at(n);
            *b = r;
            Ok(h)
        }
        fn text(b: &mut &[u8]) -> Result<String, Error> {
            let n = u32::from_le_bytes(take(b, 4)?.try_into().unwrap()) as usize;
            String::from_utf8(take(b, n)?.to_vec()).map_err(|_| Error::Decode("train units"))
        }
        let id = |b: &mut &[u8]| -> Result<Id, Error> {
            Ok(Id::from_bytes(take(b, 32)?.try_into().unwrap()))
        };
        let mut b = b
            .strip_prefix(UNITS_MAGIC)
            .ok_or(Error::Decode("train units"))?;
        let rule = id(&mut b)?;
        let unit = text(&mut b)?;
        let count = u32::from_le_bytes(take(&mut b, 4)?.try_into().unwrap()) as usize;
        let mut units = Vec::with_capacity(count.min(1 << 20));
        for _ in 0..count {
            let name = text(&mut b)?;
            let (shard, table) = (id(&mut b)?, id(&mut b)?);
            units.push(UnitEntry { name, shard, table });
        }
        if !b.is_empty() {
            return Err(Error::Decode("train units"));
        }
        Ok(Manifest { rule, unit, units })
    }
}

/// What [`Repo::train_table_units`] built.
#[derive(Clone, Debug)]
pub struct UnitsBuilt {
    pub manifest: Id,
    pub pin: Id,
    pub history: Id,
    /// Units in the table.
    pub units: usize,
    /// Units whose table was evaluated (not found already recorded).
    pub evaluated: Vec<String>,
    /// Table rows in the evaluated units.
    pub rows_evaluated: u64,
    /// Seconds: the history run, cutting it into units, building the units.
    pub run_s: f64,
    pub cut_s: f64,
    pub build_s: f64,
}

/// Rows of a table, from its bytes' header.
pub fn table_rows(bytes: &[u8]) -> u64 {
    let at = b"ikam-train-table/1\0".len();
    bytes
        .get(at..at + 8)
        .map_or(0, |b| u64::from_le_bytes(b.try_into().unwrap()))
}

/// A node's record in a batch, the parts that come from the view.
fn record_of(v: &View, k: usize) -> (u8, u16, u8, &[u32]) {
    let len = v.len[k];
    (
        v.kind[k],
        v.label[k],
        (64 - len.leading_zeros()) as u8,
        &v.adj[v.off[k] as usize..v.off[k + 1] as usize],
    )
}

/// What changed between two views numbered by paths.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delta {
    /// Every batch may differ: the views are not numbered by paths, or
    /// their node counts or targets differ.
    All(&'static str),
    Nodes {
        /// Nodes whose batch record changed: id, kind, label, size bits or
        /// neighbors. A batch holding none of them is the same batch.
        changed: Vec<u32>,
        /// Of those, the nodes whose record changed in something other
        /// than the id: what a batch without ids (E427's) would depend on.
        features: Vec<u32>,
    },
}

/// The nodes whose batch record differs between `old` and `new`.
pub fn view_delta(old: &View, new: &View) -> Delta {
    if old.order != Order::Paths || new.order != Order::Paths {
        return Delta::All("not numbered by paths");
    }
    if old.nodes() != new.nodes() || old.kind != new.kind {
        return Delta::All("the targets differ");
    }
    let (mut changed, mut features) = (Vec::new(), Vec::new());
    for k in 0..old.nodes() {
        let f = record_of(old, k) != record_of(new, k);
        if f || old.ids[k] != new.ids[k] {
            changed.push(k as u32);
        }
        if f {
            features.push(k as u32);
        }
    }
    Delta::Nodes { changed, features }
}

/// Which batches hold each node: the lineage from a view's nodes to its
/// batches (a projection, like `FragmentIndex`, kept by the caller).
#[derive(Clone, Debug, Default)]
pub struct BatchIndex {
    nodes: Vec<Vec<u32>>,
    by_node: HashMap<u32, Vec<u32>>,
}

impl BatchIndex {
    /// The index of batches given in order (their bytes).
    pub fn new<'a>(batches: impl IntoIterator<Item = &'a [u8]>) -> Result<BatchIndex, Error> {
        let mut ix = BatchIndex::default();
        for b in batches {
            let nodes = Batch::decode(b)?.nodes();
            let pos = ix.nodes.len() as u32;
            for &n in &nodes {
                ix.by_node.entry(n).or_default().push(pos);
            }
            ix.nodes.push(nodes);
        }
        Ok(ix)
    }

    pub fn batches(&self) -> usize {
        self.nodes.len()
    }

    /// Batches (positions) that hold one of `nodes`, sorted.
    pub fn reached(&self, nodes: &[u32]) -> Vec<usize> {
        let mut out: Vec<usize> = nodes
            .iter()
            .flat_map(|n| self.by_node.get(n).into_iter().flatten())
            .map(|&p| p as usize)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// Replace batch `pos`'s entry by `bytes`, after it was sampled again.
    pub fn update(&mut self, pos: usize, bytes: &[u8]) -> Result<(), Error> {
        let nodes = Batch::decode(bytes)?.nodes();
        for n in &self.nodes[pos] {
            if let Some(v) = self.by_node.get_mut(n) {
                v.retain(|&p| p as usize != pos);
            }
        }
        for &n in &nodes {
            let v = self.by_node.entry(n).or_default();
            let at = v.partition_point(|&p| (p as usize) < pos);
            v.insert(at, pos as u32);
        }
        self.nodes[pos] = nodes;
        Ok(())
    }
}

/// What [`Repo::resample`] did.
#[derive(Clone, Debug, Default)]
pub struct Resampled {
    /// Every batch of the new view, in the specs' order.
    pub batches: Vec<Id>,
    /// Positions sampled again.
    pub rebuilt: Vec<usize>,
    /// Positions a lineage by node id alone would sample again.
    pub by_id: usize,
    /// Positions that hold a node whose features (not only id) changed.
    pub by_features: usize,
    /// Why every batch was sampled again, when it was.
    pub all: Option<&'static str>,
}

/// Same paths, compared: each path's old and new file id, `None` when a
/// path was added or removed or changed kind.
fn diff_trees<S: Store>(
    repo: &Repo<S>,
    old: &Id,
    new: &Id,
    out: &mut Vec<(Id, Id)>,
) -> Result<bool, Error> {
    if old == new {
        return Ok(true);
    }
    let (Object::Tree(a), Object::Tree(b)) = (repo.get(old)?, repo.get(new)?) else {
        return Err(Error::WrongKind(*new));
    };
    if a.len() != b.len() {
        return Ok(false);
    }
    for (x, y) in a.iter().zip(&b) {
        if x.name != y.name || x.kind != y.kind {
            return Ok(false);
        }
        match x.kind {
            Kind::Tree => {
                if !diff_trees(repo, &x.id, &y.id, out)? {
                    return Ok(false);
                }
            }
            Kind::File if x.id != y.id => out.push((x.id, y.id)),
            _ => {}
        }
    }
    Ok(true)
}

impl<S: Store> Repo<S> {
    /// Build the training table for `rule` over the history ending at
    /// `tip` in units (module docs), and pin it like
    /// [`Repo::train_table`]: a commit on ref `pin` whose parent is `tip`
    /// and whose tree holds `history`, `rule`, `units` (the manifest) and
    /// the units' slices and tables. A unit whose slice is unchanged since
    /// an earlier build in this store reuses its recorded table; with
    /// `force`, every unit is evaluated.
    pub fn train_table_units(
        &mut self,
        pin: &str,
        tip: &Id,
        rule: &TrainRule,
        unit: &Unit,
        force: bool,
    ) -> Result<UnitsBuilt, Error> {
        let zero = Id::from_bytes([0; 32]);
        let mut out = UnitsBuilt {
            manifest: zero,
            pin: zero,
            history: zero,
            units: 0,
            evaluated: Vec::new(),
            rows_evaluated: 0,
            run_s: 0.0,
            cut_s: 0.0,
            build_s: 0.0,
        };
        let t = std::time::Instant::now();
        let history = self.train_history(tip, rule)?;
        out.run_s = t.elapsed().as_secs_f64();
        let t = std::time::Instant::now();
        let h = History::decode(&history)?;
        let shards = h.units(rule, |k| unit.of(k).to_string());
        out.cut_s = t.elapsed().as_secs_f64();
        let t = std::time::Instant::now();
        // One read memo over every unit's puts (when the handle keeps
        // one), as an edit does.
        let began = self.begin_memo();
        let built = self.put_units((pin, tip), rule, unit, force, &history, shards, &mut out);
        self.end_memo(began);
        built?;
        out.build_s = t.elapsed().as_secs_f64();
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn put_units(
        &mut self,
        (pin, tip): (&str, &Id),
        rule: &TrainRule,
        unit: &Unit,
        force: bool,
        history: &[u8],
        shards: Vec<(String, crate::train::Shard)>,
        out: &mut UnitsBuilt,
    ) -> Result<(), Error> {
        let rule_id = self.put_content(&rule.encode())?.id;
        out.history = self.put_content(history)?.id;
        let mut entries = Vec::with_capacity(shards.len());
        for (name, shard) in shards {
            let shard = self.put_content(&shard.encode())?.id;
            let (func, args) = (
                func::train_table(),
                vec![Arg::Whole(rule_id), Arg::Whole(shard)],
            );
            let recorded = if force {
                None
            } else {
                self.ledger()?
                    .by_key
                    .get(&Derivation::key(&func, &args))
                    .copied()
            };
            let known = match recorded {
                Some(o) if self.has_content(&o)? => Some(o),
                _ => None,
            };
            let table = match known {
                Some(table) => table,
                None => {
                    let (bytes, _) = self.evaluate(&func, &args)?;
                    out.rows_evaluated += table_rows(&bytes);
                    let output = self.put_content(&bytes)?.id;
                    self.record(Derivation { output, func, args })?;
                    out.evaluated.push(name.clone());
                    output
                }
            };
            entries.push(UnitEntry { name, shard, table });
        }
        out.units = entries.len();
        let manifest = Manifest {
            rule: rule_id,
            unit: unit.text(),
            units: entries,
        };
        out.manifest = self.put_content(&manifest.encode())?.id;
        let file = |name: String, id| TreeEntry {
            name,
            kind: Kind::File,
            id,
        };
        let dir = |this: &mut Self, ids: Vec<Id>| -> Result<Id, Error> {
            let es = ids
                .into_iter()
                .enumerate()
                .map(|(i, id)| file(format!("{i:07}"), id))
                .collect();
            this.put(&Object::tree(es)?)
        };
        let shards = dir(self, manifest.units.iter().map(|u| u.shard).collect())?;
        let tables = dir(self, manifest.units.iter().map(|u| u.table).collect())?;
        let sub = |name: &str, id| TreeEntry {
            name: name.into(),
            kind: Kind::Tree,
            id,
        };
        let tree = self.put(&Object::tree(vec![
            file("history".into(), out.history),
            file("rule".into(), rule_id),
            file("units".into(), out.manifest),
            sub("shards", shards),
            sub("tables", tables),
        ])?)?;
        out.pin = self.put(&Object::Commit(Commit {
            tree,
            parents: vec![*tip],
            message: "train table units".into(),
        }))?;
        let old = self.store.get_ref(pin)?;
        self.store.set_ref(pin, old, out.pin)?;
        Ok(())
    }

    /// The view of `commit` from `old`, a view numbered by paths of an
    /// earlier commit, reading only the files whose ids changed and their
    /// renders (E429). `None` when the graph's shape changed: a path added
    /// or removed, a render of another arity, one node becoming two or two
    /// one, or a new id that was already a node. Build the view again then.
    pub fn fragment_view_after(&self, old: &View, commit: &Id) -> Result<Option<View>, Error> {
        if old.order != Order::Paths {
            return Ok(None);
        }
        let tree = crate::snapshot::tree_of(self, commit)?;
        let mut pairs = Vec::new();
        if !diff_trees(self, &old.tree, &tree, &mut pairs)? {
            return Ok(None);
        }
        let mut map: HashMap<Id, Id> = HashMap::new();
        while let Some((a, b)) = pairs.pop() {
            match map.get(&a) {
                Some(x) if *x == b => continue,
                Some(_) => return Ok(None),
                None => {}
            }
            map.insert(a, b);
            match (self.fragments_of(&a)?, self.fragments_of(&b)?) {
                (None, None) => {}
                (Some(fa), Some(fb)) if fa.len() == fb.len() => {
                    pairs.extend(fa.into_iter().zip(fb).filter(|(x, y)| x != y));
                }
                _ => return Ok(None),
            }
        }
        let olds: HashSet<&Id> = old.ids.iter().collect();
        let mut news = HashSet::with_capacity(map.len());
        for (a, b) in &map {
            if !olds.contains(a) || !news.insert(*b) || (olds.contains(b) && !map.contains_key(b)) {
                return Ok(None);
            }
        }
        let mut v = old.clone();
        v.tree = tree;
        v.commit = match self.get(commit)? {
            Object::Commit(_) => *commit,
            _ => Id::from_bytes([0; 32]),
        };
        for k in 0..v.ids.len() {
            if let Some(b) = map.get(&v.ids[k]) {
                v.ids[k] = *b;
                v.len[k] = self.read_content(b)?.len() as u64;
            }
        }
        Ok(Some(v))
    }

    /// The view of `commit` numbered by paths, kept as a recorded step:
    /// from `old` when the shape held ([`Repo::fragment_view_after`]), else
    /// built again. Returns the view, its id and whether it was built from
    /// `old`.
    pub fn record_fragment_view_after(
        &mut self,
        old: &View,
        commit: &Id,
    ) -> Result<(View, Id, bool), Error> {
        let (v, from_old) = match self.fragment_view_after(old, commit)? {
            Some(v) => (v, true),
            None => (self.fragment_view_ordered(commit, Order::Paths)?, false),
        };
        let (id, _) = self.record_view_bytes(commit, Order::Paths, &v.encode())?;
        Ok((v, id, from_old))
    }

    /// The batches `specs` of the view `new` (numbered by paths), given
    /// `old_batches`, the same specs' batches of the view `old`, and
    /// `index`, their lineage. Batches that hold a node [`view_delta`]
    /// names are sampled again (and `index` updated); every other one is
    /// recorded as `sample(new, spec)` with its old output, unevaluated:
    /// nothing it depends on changed. Every batch is sampled again when the
    /// delta is [`Delta::All`].
    pub fn resample(
        &mut self,
        index: &mut BatchIndex,
        old: &View,
        new: (&View, Id),
        specs: &[Spec],
        old_batches: &[Id],
    ) -> Result<Resampled, Error> {
        if specs.len() != old_batches.len() || index.batches() != specs.len() {
            return Err(Error::Exec(
                "resample: specs, batches and index differ".into(),
            ));
        }
        let (view, view_id) = new;
        let mut out = Resampled::default();
        let reached = match view_delta(old, view) {
            Delta::All(why) => {
                out.all = Some(why);
                (0..specs.len()).collect()
            }
            Delta::Nodes { changed, features } => {
                out.by_features = index.reached(&features).len();
                let ids: Vec<u32> = changed
                    .iter()
                    .filter(|&&k| old.ids[k as usize] != view.ids[k as usize])
                    .copied()
                    .collect();
                out.by_id = index.reached(&ids).len();
                index.reached(&changed)
            }
        };
        let mut again = vec![false; specs.len()];
        reached.iter().for_each(|&p| again[p] = true);
        for (p, spec) in specs.iter().enumerate() {
            let text = self.put_content(spec.to_text().as_bytes())?.id;
            let args = vec![Arg::Whole(view_id), Arg::Whole(text)];
            let id = if again[p] {
                let id = self.apply(func::sample(), args)?;
                index.update(p, &self.read_content(&id)?)?;
                id
            } else {
                let output = old_batches[p];
                if !self.has_content(&output)? {
                    return Err(Error::NotFound(output));
                }
                self.record(Derivation {
                    output,
                    func: func::sample(),
                    args,
                })?;
                output
            };
            out.batches.push(id);
        }
        out.rebuilt = reached;
        Ok(out)
    }
}

/// The units a set of changed keys reaches (the forward lineage).
pub fn units_reached<'a>(unit: &Unit, keys: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let set: BTreeMap<&str, ()> = keys.into_iter().map(|k| (unit.of(k), ())).collect();
    set.into_keys().map(str::to_string).collect()
}
