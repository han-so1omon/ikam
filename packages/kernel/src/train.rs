//! Point-in-time training tables pinned to commits (E426, opt-in, library
//! only).
//!
//! A training table has one row per (row key, observation commit) of a
//! database's history: integer features of the row as of the observation
//! commit, and a target from the commits after it, "does this row's value
//! change within the next `horizon` commits?". It is built in two steps:
//!
//! 1. A run, [`Repo::train_history`]: walk the first-parent chain ending at
//!    a tip commit and fold its database log into the history of every key
//!    under a prefix (for each commit, the keys whose value changed in it,
//!    with their new values as integers). This step reads commits, trees and
//!    graphs, which are objects, not content. A kernel function sees only
//!    its arguments' bytes, and arguments must be content (repack re-plans
//!    content alone, gc keeps a derivation's arguments as content), so this
//!    step cannot be a derivation. Its output is stored as plain content and
//!    recorded by the pin commit, whose parent is the tip it read;
//!    [`Repo::train_verify`] runs it again and compares.
//! 2. A derivation, `table = train-table/1(rule, history)`: a builtin over
//!    two contents, recorded with `apply`. Repack may drop the table's bytes
//!    and fsck re-evaluates it.
//!
//! Every feature is read through [`Reads::feature`], which counts reads of
//! a change logged after the observation commit; the builtin refuses to
//! give a table when that count is not zero, so the point-in-time property
//! is checked on every evaluation, fsck's included.
//!
//! Nothing in the default paths calls this module.

use std::collections::BTreeMap;

use crate::{
    Arg, Commit, Error, Event, Id, Kind, Object, Op, Repo, Store, Target, TreeEntry, func,
};

const RULE_MAGIC: &[u8] = b"ikam-train-rule/1\0";
const HISTORY_MAGIC: &[u8] = b"ikam-train-history/1\0";
const TABLE_MAGIC: &[u8] = b"ikam-train-table/1\0";

/// Which rows, and the label rule: a row of `prefix` observed at commit `t`
/// (every `every`-th commit of the chain, from the first, while
/// `t + horizon` is still in it) is labelled 1 when its value changes in
/// one of the `horizon` commits after `t`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrainRule {
    pub prefix: String,
    pub horizon: u32,
    pub every: u32,
}

impl TrainRule {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = RULE_MAGIC.to_vec();
        out.extend_from_slice(&self.horizon.to_le_bytes());
        out.extend_from_slice(&self.every.to_le_bytes());
        out.extend_from_slice(self.prefix.as_bytes());
        out
    }

    pub fn decode(b: &[u8]) -> Result<TrainRule, Error> {
        let bad = Error::Decode("train rule");
        let mut b = b
            .strip_prefix(RULE_MAGIC)
            .ok_or(Error::Decode("train rule"))?;
        let horizon = u32_at(&mut b)?;
        let every = u32_at(&mut b)?;
        let prefix = String::from_utf8(b.to_vec()).map_err(|_| Error::Decode("train rule"))?;
        if horizon == 0 || every == 0 {
            return Err(bad);
        }
        Ok(TrainRule {
            prefix,
            horizon,
            every,
        })
    }
}

/// A key's value changed in a commit: `row` and `commit` index the
/// history's keys and commits; `value` is the value as a decimal integer,
/// or its length in bytes when it is not one. A removal has `present` false.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub row: u32,
    pub commit: u32,
    pub present: bool,
    pub value: i64,
}

/// The run's output: the chain's commits (oldest first), the keys that
/// ever held a value (sorted), and every change, by row then commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    pub commits: Vec<Id>,
    pub keys: Vec<String>,
    pub changes: Vec<Change>,
}

impl History {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = HISTORY_MAGIC.to_vec();
        out.extend_from_slice(&(self.commits.len() as u32).to_le_bytes());
        for c in &self.commits {
            out.extend_from_slice(c.as_bytes());
        }
        out.extend_from_slice(&(self.keys.len() as u32).to_le_bytes());
        for k in &self.keys {
            out.extend_from_slice(&(k.len() as u32).to_le_bytes());
            out.extend_from_slice(k.as_bytes());
        }
        let c = &self.changes;
        out.extend_from_slice(&(c.len() as u64).to_le_bytes());
        c.iter()
            .for_each(|c| out.extend_from_slice(&c.row.to_le_bytes()));
        c.iter()
            .for_each(|c| out.extend_from_slice(&c.commit.to_le_bytes()));
        c.iter().for_each(|c| out.push(c.present as u8));
        c.iter()
            .for_each(|c| out.extend_from_slice(&c.value.to_le_bytes()));
        out
    }

    pub fn decode(b: &[u8]) -> Result<History, Error> {
        let bad = || Error::Decode("train history");
        let mut b = b.strip_prefix(HISTORY_MAGIC).ok_or_else(bad)?;
        let n = u32_at(&mut b)? as usize;
        let commits = (0..n)
            .map(|_| Ok(Id::from_bytes(take(&mut b, 32)?.try_into().unwrap())))
            .collect::<Result<Vec<_>, Error>>()?;
        let n = u32_at(&mut b)? as usize;
        let mut keys = Vec::with_capacity(n.min(b.len()));
        for _ in 0..n {
            let len = u32_at(&mut b)? as usize;
            keys.push(String::from_utf8(take(&mut b, len)?.to_vec()).map_err(|_| bad())?);
        }
        let n = usize::try_from(u64::from_le_bytes(take(&mut b, 8)?.try_into().unwrap()))
            .map_err(|_| bad())?;
        if b.len() != n.checked_mul(17).ok_or_else(bad)? {
            return Err(bad());
        }
        let (rows, rest) = b.split_at(4 * n);
        let (at, rest) = rest.split_at(4 * n);
        let (present, values) = rest.split_at(n);
        let u32s = |s: &[u8], i: usize| u32::from_le_bytes(s[4 * i..4 * i + 4].try_into().unwrap());
        let changes: Vec<Change> = (0..n)
            .map(|i| Change {
                row: u32s(rows, i),
                commit: u32s(at, i),
                present: present[i] == 1,
                value: i64::from_le_bytes(values[8 * i..8 * i + 8].try_into().unwrap()),
            })
            .collect();
        let ordered = changes
            .windows(2)
            .all(|w| (w[0].row, w[0].commit) < (w[1].row, w[1].commit));
        let in_range = changes
            .iter()
            .all(|c| (c.row as usize) < keys.len() && (c.commit as usize) < commits.len());
        if !ordered || !in_range || present.iter().any(|&p| p > 1) {
            return Err(bad());
        }
        Ok(History {
            commits,
            keys,
            changes,
        })
    }
}

/// One table row, integers only: `row` and `obs` index the history's keys
/// and commits. Features as of commit `obs`: the row's `value`, how many
/// `changes` it had up to and including `obs`, and its `age`, the commits
/// since its last change. `label`: it changes within the horizon after.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub row: u32,
    pub obs: u32,
    pub label: bool,
    pub value: i64,
    pub changes: u32,
    pub age: u32,
}

/// What a build read: feature reads, those of a change logged after the
/// row's observation commit (must be 0), and label reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reads {
    pub feature: u64,
    pub past_obs: u64,
    pub label: u64,
}

impl Reads {
    /// A feature read of a change logged at commit `at` for a row observed
    /// at `obs`.
    pub fn feature(&mut self, at: u32, obs: u32) {
        self.feature += 1;
        self.past_obs += u64::from(at > obs);
    }
}

/// The table's row for one key observed at `t`, from the key's changes in
/// commit order: `None` when it holds no value at `t`.
fn observe(changes: &[Change], t: u32, horizon: u32, reads: &mut Reads) -> Option<Row> {
    let seen = changes.partition_point(|c| c.commit <= t);
    let last = changes[..seen].last()?;
    for c in &changes[..seen] {
        reads.feature(c.commit, t);
    }
    if !last.present {
        return None;
    }
    let next = changes.get(seen);
    reads.label += u64::from(next.is_some());
    Some(Row {
        row: last.row,
        obs: t,
        label: next.is_some_and(|c| c.commit <= t + horizon),
        value: last.value,
        changes: seen as u32,
        age: t - last.commit,
    })
}

/// `train-table/1`: the table for `rule` over `history` (both encoded),
/// rows by observation commit then key, and what it read.
pub fn build(rule: &[u8], history: &[u8]) -> Result<(Vec<u8>, Reads), Error> {
    let rule = TrainRule::decode(rule)?;
    let h = History::decode(history)?;
    let mut groups: Vec<&[Change]> = Vec::new();
    let mut rest = h.changes.as_slice();
    while let Some(first) = rest.first() {
        let n = rest.partition_point(|c| c.row == first.row);
        groups.push(&rest[..n]);
        rest = &rest[n..];
    }
    let last = h.commits.len() as u64;
    let mut reads = Reads::default();
    let mut rows = Vec::new();
    for t in (0..h.commits.len() as u32).step_by(rule.every as usize) {
        if u64::from(t) + u64::from(rule.horizon) >= last {
            break;
        }
        rows.extend(
            groups
                .iter()
                .filter_map(|g| observe(g, t, rule.horizon, &mut reads)),
        );
    }
    Ok((encode_table(&rows), reads))
}

/// `build` as the builtin runs it: refused when a feature was read past
/// its observation commit.
pub(crate) fn run(args: &[Vec<u8>]) -> Result<Vec<u8>, Error> {
    let [rule, history] = args else {
        return Err(Error::Exec(
            "train-table/1 takes a rule and a history".into(),
        ));
    };
    let (out, reads) = build(rule, history)?;
    if reads.past_obs != 0 {
        return Err(Error::Exec(format!(
            "train-table/1: {} feature reads past their observation commit",
            reads.past_obs
        )));
    }
    Ok(out)
}

fn encode_table(rows: &[Row]) -> Vec<u8> {
    let mut out = TABLE_MAGIC.to_vec();
    out.extend_from_slice(&(rows.len() as u64).to_le_bytes());
    rows.iter()
        .for_each(|r| out.extend_from_slice(&r.row.to_le_bytes()));
    rows.iter()
        .for_each(|r| out.extend_from_slice(&r.obs.to_le_bytes()));
    rows.iter().for_each(|r| out.push(r.label as u8));
    rows.iter()
        .for_each(|r| out.extend_from_slice(&r.value.to_le_bytes()));
    rows.iter()
        .for_each(|r| out.extend_from_slice(&r.changes.to_le_bytes()));
    rows.iter()
        .for_each(|r| out.extend_from_slice(&r.age.to_le_bytes()));
    out
}

/// A table's rows, from its bytes.
pub fn decode_table(b: &[u8]) -> Result<Vec<Row>, Error> {
    let bad = || Error::Decode("train table");
    let mut b = b.strip_prefix(TABLE_MAGIC).ok_or_else(bad)?;
    let n = usize::try_from(u64::from_le_bytes(take(&mut b, 8)?.try_into().unwrap()))
        .map_err(|_| bad())?;
    if b.len() != n.checked_mul(25).ok_or_else(bad)? {
        return Err(bad());
    }
    let col = |at: usize, i: usize, w: usize| &b[at * n + w * i..at * n + w * i + w];
    let u32s = |at, i| u32::from_le_bytes(col(at, i, 4).try_into().unwrap());
    Ok((0..n)
        .map(|i| Row {
            row: u32s(0, i),
            obs: u32s(4, i),
            label: b[8 * n + i] == 1,
            value: i64::from_le_bytes(b[9 * n + 8 * i..9 * n + 8 * i + 8].try_into().unwrap()),
            changes: u32s(17, i),
            age: u32s(21, i),
        })
        .collect())
}

fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8], Error> {
    if b.len() < n {
        return Err(Error::Decode("train: truncated"));
    }
    let (head, rest) = b.split_at(n);
    *b = rest;
    Ok(head)
}

fn u32_at(b: &mut &[u8]) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(take(b, 4)?.try_into().unwrap()))
}

/// A value as an integer feature: itself when it is a decimal integer,
/// else its length in bytes.
fn int_of(v: &[u8]) -> i64 {
    std::str::from_utf8(v)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(v.len() as i64)
}

/// A table row with its key and observation commit resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrainRow {
    pub key: String,
    pub commit: Id,
    pub label: bool,
    pub value: i64,
    pub changes: u32,
    pub age: u32,
}

/// The values a commit's events gave the keys they touched, as the db fold
/// does (`db.rs`): `before` keeps each touched key's value at the commit's
/// start.
fn apply_event(
    state: &mut BTreeMap<String, Vec<u8>>,
    before: &mut BTreeMap<String, Option<Vec<u8>>>,
    e: Event,
    prefix: &str,
) {
    let mut touch = |state: &BTreeMap<String, Vec<u8>>, k: &String| {
        if !before.contains_key(k) {
            before.insert(k.clone(), state.get(k).cloned());
        }
    };
    let gone: Vec<String> = match &e.op {
        Op::Upsert(v) => {
            if e.key.starts_with(prefix) {
                touch(state, &e.key);
                state.insert(e.key, v.clone());
            }
            return;
        }
        Op::Remove { subtree } => {
            let below = format!("{}/", e.key);
            let mut ks: Vec<String> = state
                .contains_key(&e.key)
                .then(|| e.key.clone())
                .into_iter()
                .collect();
            if *subtree {
                ks.extend(
                    state
                        .range(below.clone()..)
                        .take_while(|(k, _)| k.starts_with(&below))
                        .map(|(k, _)| k.clone()),
                );
            }
            ks
        }
        Op::RemoveRange { end } => state
            .range(e.key.clone()..end.clone())
            .map(|(k, _)| k.clone())
            .collect(),
    };
    for k in gone {
        touch(state, &k);
        state.remove(&k);
    }
}

impl<S: Store> Repo<S> {
    /// The run of step 1 (module docs): the encoded [`History`] of keys
    /// under `rule.prefix` along the first-parent chain ending at `tip`.
    /// Each commit's database log must be a prefix of the tip's.
    pub fn train_history(&mut self, tip: &Id, rule: &TrainRule) -> Result<Vec<u8>, Error> {
        let mut chain: Vec<Id> = self.log(*tip)?.into_iter().map(|(id, _)| id).collect();
        chain.reverse();
        let tip_db = self.db_at(tip)?;
        let nodes = self.graph_nodes(&tip_db.log)?;
        let not_prefix =
            || Error::Exec("train: a commit's db log is not a prefix of the tip's".into());
        let mut heads = Vec::with_capacity(chain.len());
        for c in &chain {
            let db = self.db_at(c)?;
            let head = self.db_head(&db.log)?;
            if head > 0 && self.graph_last(&db.log)?.as_ref() != nodes.get(head as usize - 1) {
                return Err(not_prefix());
            }
            if heads.last().is_some_and(|&h| h > head) {
                return Err(not_prefix());
            }
            heads.push(head);
        }
        let mut state: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut by_key: BTreeMap<String, Vec<(u32, bool, i64)>> = BTreeMap::new();
        let mut done = 0u64;
        for (t, &head) in heads.iter().enumerate() {
            let mut before = BTreeMap::new();
            for node in &nodes[done as usize..head as usize] {
                let Target::Content(a) = &node.target else {
                    return Err(Error::Decode("db log"));
                };
                let event = Event::decode(&self.read_content(&a.id())?)?.0;
                apply_event(&mut state, &mut before, event, &rule.prefix);
            }
            done = head;
            for (k, old) in before {
                let new = state.get(&k);
                if new != old.as_ref() {
                    let change = (t as u32, new.is_some(), new.map_or(0, |v| int_of(v)));
                    by_key.entry(k).or_default().push(change);
                }
            }
        }
        let mut history = History {
            commits: chain,
            keys: Vec::with_capacity(by_key.len()),
            changes: Vec::new(),
        };
        for (row, (key, changes)) in by_key.into_iter().enumerate() {
            history.keys.push(key);
            history
                .changes
                .extend(changes.into_iter().map(|(commit, present, value)| Change {
                    row: row as u32,
                    commit,
                    present,
                    value,
                }));
        }
        Ok(history.encode())
    }

    /// Build the training table for `rule` over the history ending at
    /// `tip` and pin it: a commit on ref `pin` whose parent is `tip` and
    /// whose tree holds `history`, `rule` and `table`. Returns the table's
    /// content id, the output of the recorded derivation
    /// `train-table/1(rule, history)`.
    pub fn train_table(&mut self, pin: &str, tip: &Id, rule: &TrainRule) -> Result<Id, Error> {
        let history = self.train_history(tip, rule)?;
        let rule_id = self.put_content(&rule.encode())?.id;
        let history_id = self.put_content(&history)?.id;
        let table = self.apply(
            func::train_table(),
            vec![Arg::Whole(rule_id), Arg::Whole(history_id)],
        )?;
        let file = |name: &str, id| TreeEntry {
            name: name.into(),
            kind: Kind::File,
            id,
        };
        let tree = self.put(&Object::tree(vec![
            file("history", history_id),
            file("rule", rule_id),
            file("table", table),
        ])?)?;
        let commit = self.put(&Object::Commit(Commit {
            tree,
            parents: vec![*tip],
            message: "train table".into(),
        }))?;
        let old = self.store.get_ref(pin)?;
        self.store.set_ref(pin, old, commit)?;
        Ok(table)
    }

    /// The rule and history a table was derived from (its recorded
    /// `train-table/1` derivation's arguments).
    pub fn train_inputs(&self, table: &Id) -> Result<(Id, Id), Error> {
        let d = (self.derivations(table)?.into_iter())
            .find(|d| d.func == func::train_table() && d.args.len() == 2)
            .ok_or(Error::WrongKind(*table))?;
        Ok((d.args[0].id(), d.args[1].id()))
    }

    /// The table's rows with keys and observation commits resolved through
    /// the history it was derived from.
    pub fn train_rows(&self, table: &Id) -> Result<Vec<TrainRow>, Error> {
        let (_, history) = self.train_inputs(table)?;
        let h = History::decode(&self.read_content(&history)?)?;
        let rows = decode_table(&self.read_content(table)?)?;
        rows.into_iter()
            .map(|r| {
                Ok(TrainRow {
                    key: h
                        .keys
                        .get(r.row as usize)
                        .ok_or(Error::Corrupt(*table))?
                        .clone(),
                    commit: *h
                        .commits
                        .get(r.obs as usize)
                        .ok_or(Error::Corrupt(*table))?,
                    label: r.label,
                    value: r.value,
                    changes: r.changes,
                    age: r.age,
                })
            })
            .collect()
    }

    /// Check a pin commit: its history is what the run gives again from its
    /// parent, and its table is `train-table/1(rule, history)` evaluated
    /// again. Returns that evaluation's reads.
    pub fn train_verify(&mut self, pin: &Id) -> Result<Reads, Error> {
        let Object::Commit(c) = self.get(pin)? else {
            return Err(Error::WrongKind(*pin));
        };
        let Object::Tree(entries) = self.get(&c.tree)? else {
            return Err(Error::WrongKind(c.tree));
        };
        let entry = |n: &str| {
            (entries.iter().find(|e| e.name == n).map(|e| e.id)).ok_or(Error::WrongKind(*pin))
        };
        let (history, rule, table) = (entry("history")?, entry("rule")?, entry("table")?);
        let [parent] = c.parents[..] else {
            return Err(Error::WrongKind(*pin));
        };
        let rule_bytes = self.read_content(&rule)?;
        let again = self.train_history(&parent, &TrainRule::decode(&rule_bytes)?)?;
        if Id::of_content(&again) != history || self.train_inputs(&table)? != (rule, history) {
            return Err(Error::Corrupt(history));
        }
        let (out, reads) = build(&rule_bytes, &again)?;
        if Id::of_content(&out) != table || reads.past_obs != 0 {
            return Err(Error::Corrupt(table));
        }
        Ok(reads)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_past_the_observation_commit_is_counted() {
        let mut r = Reads::default();
        r.feature(3, 3);
        r.feature(4, 3);
        assert_eq!((r.feature, r.past_obs), (2, 1));
    }

    #[test]
    fn rows_see_only_changes_up_to_their_commit() {
        let ch = |commit, present, value| Change {
            row: 0,
            commit,
            present,
            value,
        };
        // set at 0, changed at 2, removed at 5
        let changes = [ch(0, true, 10), ch(2, true, 11), ch(5, false, 0)];
        let mut reads = Reads::default();
        let at1 = observe(&changes, 1, 1, &mut reads).unwrap();
        assert_eq!(
            (at1.value, at1.changes, at1.age, at1.label),
            (10, 1, 1, true)
        );
        let at3 = observe(&changes, 3, 1, &mut reads).unwrap();
        assert_eq!(
            (at3.value, at3.changes, at3.age, at3.label),
            (11, 2, 1, false)
        );
        assert!(observe(&changes, 3, 2, &mut reads).unwrap().label);
        assert_eq!(observe(&changes, 5, 1, &mut reads), None);
        assert_eq!(reads.past_obs, 0);
    }
}
