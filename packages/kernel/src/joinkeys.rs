//! Join keys proposed by name and confirmed against the data (E420),
//! after NVIDIA Auto Ontology's inferred joins, which it keeps only when
//! the data confirms them.
//!
//! - `propose` pairs a child column with a parent column of another table
//!   when their names say they are one thing: each column's table prefix
//!   (`ss_`, `i_`) is dropped, a key suffix (`sk`, `id`, `key`) on either
//!   side must be on both, and the parent's other words must each be in
//!   the child's name, whole or as a prefix of at least three letters
//!   (`addr` for `address`); a column called only by its suffix stands for
//!   its table's name. With a name embedder (E387's, when its weights are
//!   present), the cosine of the two names may vouch for a pair instead.
//! - `stats_check` rejects what the column statistics (E403) and zone maps
//!   (E405) already rule out: a parent that is not unique and not null in
//!   every row, kinds that differ, a child range outside the parent's or
//!   outside every parent zone, more distinct child values than parent
//!   ones, child categories the parent lacks.
//! - `Repo::join_key_confirm` then checks exactly: every child value that
//!   is not null is one of the parent's values, the parent's values are
//!   all different, and the join through `join.rs` is not empty.
//! - `Repo::put_join_keys` keeps each confirmed pair with its evidence: a
//!   graph with a node `CHILD_TABLE.COLUMN>PARENT_TABLE.COLUMN` per pair,
//!   its content the evidence as JSON.
//!
//! Opt-in: nothing in the kernel proposes or confirms keys unless a caller
//! asks.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::colstats::{ColumnStats, TableStats, field_value};
use crate::join::{Arrangement, Join};
use crate::{Arg, Db, Error, Id, Node, Repo, Store, Target};

/// Name words that say a column is a key, not what it is the key of.
const KEY_WORDS: [&str; 3] = ["sk", "id", "key"];
/// A parent is unique when its distinct estimate is at least this share
/// of its rows (HyperLogLog's error is about 1.6%).
const UNIQUE_SHARE: f64 = 0.95;
/// Slack on the distinct estimates' comparison.
const DISTINCT_SLACK: f64 = 1.05;

/// A table and its statistics. `table` is its key prefix without the
/// trailing `/` (rows are the `kv` keys under `table/`); `name` its name.
pub struct TableInfo<'a> {
    pub table: String,
    pub name: String,
    pub stats: &'a TableStats,
}

/// A proposed pair: the child column's values should be the parent's.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub child_table: String,
    pub child: String,
    pub parent_table: String,
    pub parent: String,
    pub name_score: f64,
}

impl Candidate {
    /// The pair's label, `CHILD_TABLE.COLUMN>PARENT_TABLE.COLUMN`.
    pub fn label(&self) -> String {
        format!(
            "{}.{}>{}.{}",
            self.child_table, self.child, self.parent_table, self.parent
        )
    }
}

/// Why a pair was not kept.
#[derive(Clone, Debug, PartialEq)]
pub enum Rejected {
    /// The parent has nulls, is missing from rows, or repeats values.
    NotUnique,
    /// Numbers on one side, strings on the other.
    Kind,
    /// The child's numbers fall outside the parent's range.
    Range,
    /// The child's numbers fall outside every parent zone.
    Zones,
    /// The child has more distinct values than the parent.
    Distinct,
    /// The child has categories the parent lacks.
    Values,
    /// So many child values (not null) are not parent values.
    NotContained(usize),
    /// The child has no value that is not null.
    Empty,
    /// The join through `join.rs` has no rows.
    EmptyJoin,
}

/// What confirmed a pair.
#[derive(Clone, Debug, PartialEq)]
pub struct Evidence {
    pub name_score: f64,
    pub child_rows: u64,
    pub child_nulls: u64,
    pub child_distinct_est: f64,
    pub parent_rows: u64,
    pub parent_distinct_est: f64,
    /// Numbers' ranges, child then parent, when both hold numbers.
    pub ranges: Option<((f64, f64), (f64, f64))>,
    /// Child values (not null) checked, all found among the parent's.
    pub contained: usize,
    pub child_distinct: usize,
    pub parent_distinct: usize,
    pub join_rows: usize,
    pub join_root: Id,
}

impl Evidence {
    pub fn to_json(&self, c: &Candidate) -> String {
        let ranges = match self.ranges {
            Some(((a, b), (x, y))) => format!("[[{a:?},{b:?}],[{x:?},{y:?}]]"),
            None => "null".into(),
        };
        format!(
            "{{\"child\":\"{}.{}\",\"parent\":\"{}.{}\",\"name_score\":{:?},\"child_rows\":{},\"child_nulls\":{},\"child_distinct_est\":{:.0},\"parent_rows\":{},\"parent_distinct_est\":{:.0},\"ranges\":{ranges},\"contained\":{},\"child_distinct\":{},\"parent_distinct\":{},\"join_rows\":{},\"join_root\":\"{}\"}}",
            c.child_table,
            c.child,
            c.parent_table,
            c.parent,
            self.name_score,
            self.child_rows,
            self.child_nulls,
            self.child_distinct_est,
            self.parent_rows,
            self.parent_distinct_est,
            self.contained,
            self.child_distinct,
            self.parent_distinct,
            self.join_rows,
            self.join_root,
        )
    }
}

/// The prefix every column of a table starts with, up to and including
/// an underscore (`ss_`), or "" if they share none.
pub fn column_prefix<'a>(columns: impl IntoIterator<Item = &'a str>) -> String {
    let mut it = columns.into_iter();
    let Some(first) = it.next() else {
        return String::new();
    };
    let mut p = first.to_string();
    for c in it {
        let n = p.bytes().zip(c.bytes()).take_while(|(a, b)| a == b).count();
        p.truncate(n);
    }
    match p.rfind('_') {
        Some(i) if i + 1 < first.len() => p[..=i].to_string(),
        _ => String::new(),
    }
}

fn words(s: &str) -> Vec<String> {
    s.split(['_', ' ', '-'])
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn word_match(a: &str, b: &str) -> bool {
    a == b || (a.len().min(b.len()) >= 3 && (a.starts_with(b) || b.starts_with(a)))
}

/// How well a child column's name names a parent column (0 to 1): the
/// share of the parent's words, its key suffix aside, found in the
/// child's name; 0 when a key suffix is on one side only or differs.
/// `parent_table` stands in for a parent named only by its suffix.
pub fn name_score(child: &str, parent: &str, parent_table: &str) -> f64 {
    let (c, p) = (words(child), words(parent));
    let (Some(cl), Some(pl)) = (c.last(), p.last()) else {
        return 0.0;
    };
    let key = |w: &str| KEY_WORDS.contains(&w);
    if (key(cl) || key(pl)) && cl != pl {
        return 0.0;
    }
    let mut head: Vec<String> = p.iter().filter(|w| !key(w)).cloned().collect();
    if head.is_empty() {
        head = words(parent_table)
            .into_iter()
            .filter(|w| w != "dim")
            .collect();
    }
    if head.is_empty() {
        return 0.0;
    }
    let found = head
        .iter()
        .filter(|h| c.iter().any(|w| !key(w) && word_match(h, w)))
        .count();
    found as f64 / head.len() as f64
}

/// Cosine of two vectors (0 if either is zero).
fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let (mut d, mut x, mut y) = (0f64, 0f64, 0f64);
    for (p, q) in a.iter().zip(b) {
        d += (*p as f64) * (*q as f64);
        x += (*p as f64) * (*p as f64);
        y += (*q as f64) * (*q as f64);
    }
    if x == 0.0 || y == 0.0 {
        0.0
    } else {
        d / (x.sqrt() * y.sqrt())
    }
}

/// Embeds a column's name (words split at underscores) as a vector.
pub type NameEmbedder = dyn Fn(&str) -> Vec<f32>;

/// Every pair of columns in two different tables whose names score at
/// least `min_score` (by `name_score`, or by the cosine of `embed`'s
/// vectors for the two names if that is higher), best first.
pub fn propose(
    tables: &[TableInfo],
    min_score: f64,
    embed: Option<&NameEmbedder>,
) -> Vec<Candidate> {
    let stripped: Vec<(String, Vec<(&String, String)>)> = tables
        .iter()
        .map(|t| {
            let pre = column_prefix(t.stats.columns.keys().map(String::as_str));
            let cols = t
                .stats
                .columns
                .keys()
                .map(|c| (c, c.strip_prefix(&pre).unwrap_or(c).to_string()))
                .collect();
            (pre, cols)
        })
        .collect();
    let mut vectors: HashMap<String, Vec<f32>> = HashMap::new();
    let mut vec_of = |s: &str| -> Option<Vec<f32>> {
        let e = embed?;
        Some(
            vectors
                .entry(s.to_string())
                .or_insert_with(|| e(&s.replace('_', " ")))
                .clone(),
        )
    };
    let mut out = Vec::new();
    for (ci, ct) in tables.iter().enumerate() {
        for (pi, pt) in tables.iter().enumerate() {
            if ci == pi {
                continue;
            }
            for (child, cs) in &stripped[ci].1 {
                for (parent, ps) in &stripped[pi].1 {
                    let mut score = name_score(cs, ps, &pt.name);
                    if let (Some(a), Some(b)) = (vec_of(cs), vec_of(&format!("{} {ps}", pt.name))) {
                        score = score.max(cosine(&a, &b));
                    }
                    if score >= min_score {
                        out.push(Candidate {
                            child_table: ct.table.clone(),
                            child: (*child).clone(),
                            parent_table: pt.table.clone(),
                            parent: (*parent).clone(),
                            name_score: score,
                        });
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| {
        b.name_score
            .total_cmp(&a.name_score)
            .then_with(|| a.label().cmp(&b.label()))
    });
    out
}

/// What the statistics alone rule out for a child column `c` and a parent
/// column `p` of the table `parent` (its zone maps too, if kept).
pub fn stats_check(c: &ColumnStats, p: &ColumnStats, parent: &TableStats) -> Result<(), Rejected> {
    if p.nulls > 0 || p.count < parent.rows {
        return Err(Rejected::NotUnique);
    }
    if p.distinct.estimate() < UNIQUE_SHARE * parent.rows as f64 {
        return Err(Rejected::NotUnique);
    }
    let kinds = |s: &ColumnStats| (s.numbers > 0, s.strings > 0, s.bools > 0, s.other > 0);
    let (ck, pk) = (kinds(c), kinds(p));
    let held = |k: (bool, bool, bool, bool)| [k.0, k.1, k.2, k.3];
    if held(ck).iter().zip(held(pk)).any(|(a, b)| *a && !b) {
        return Err(Rejected::Kind);
    }
    if c.count == c.nulls {
        return Err(Rejected::Empty);
    }
    if c.numbers > 0 && (c.min < p.min || c.max > p.max) {
        return Err(Rejected::Range);
    }
    if c.distinct.estimate() > DISTINCT_SLACK * p.distinct.estimate() {
        return Err(Rejected::Distinct);
    }
    if let (Some(cv), Some(pv)) = (&c.values, &p.values)
        && cv.keys().any(|k| !pv.contains_key(k))
    {
        return Err(Rejected::Values);
    }
    if c.numbers > 0
        && let Some(z) = &parent.zones
    {
        let ranges: Vec<(f64, f64)> = z
            .zones
            .iter()
            .filter_map(|zone| zone.columns.get(p.idx()).and_then(|zc| zc.num))
            .collect();
        let inside = |x: f64| ranges.iter().any(|(a, b)| *a <= x && x <= *b);
        if !ranges.is_empty() && !(inside(c.min) && inside(c.max)) {
            return Err(Rejected::Zones);
        }
    }
    Ok(())
}

/// Column values already read, by table and column: each row's value
/// (`None`: missing or null), in key order.
pub type ValueCache = HashMap<(String, String), Vec<Option<Vec<u8>>>>;

thread_local! {
    /// The fields the arrangements of `join_key_confirm` key rows by
    /// (`Arrangement::key` is a plain function).
    static KEY_FIELDS: RefCell<(String, String)> = const { RefCell::new((String::new(), String::new())) };
}

fn key_text(v: &[u8], left: bool) -> Option<String> {
    KEY_FIELDS.with(|f| {
        let f = f.borrow();
        let name = if left { &f.0 } else { &f.1 };
        field_value(v, name)
            .filter(|t| *t != b"null")
            .map(|t| String::from_utf8_lossy(t).into_owned())
            .filter(|t| !t.contains('\u{1f}'))
    })
}

fn left_key(_: &str, v: &[u8]) -> Option<String> {
    key_text(v, true)
}

fn right_key(_: &str, v: &[u8]) -> Option<String> {
    key_text(v, false)
}

fn nothing(_: &[u8], _: &[u8]) -> Vec<u8> {
    Vec::new()
}

impl<S: Store> Repo<S> {
    fn column_values<'c>(
        &self,
        db: &Db,
        table: &str,
        column: &str,
        cache: &'c mut ValueCache,
    ) -> Result<&'c Vec<Option<Vec<u8>>>, Error> {
        let k = (table.to_string(), column.to_string());
        if !cache.contains_key(&k) {
            let rows = self.db_prefix(db, &format!("{table}/"), usize::MAX, false)?;
            let vals = rows
                .iter()
                .map(|(_, v)| {
                    field_value(v, column)
                        .filter(|t| *t != b"null")
                        .map(<[u8]>::to_vec)
                })
                .collect();
            cache.insert(k.clone(), vals);
        }
        Ok(&cache[&k])
    }

    /// Check a proposed pair against the data: first its columns'
    /// statistics (`stats`, by table), then exactly (every child value
    /// not null among the parent's values, the parent's values all
    /// different), then a join through `join.rs` that must not be empty.
    pub fn join_key_confirm(
        &mut self,
        db: &Db,
        c: &Candidate,
        stats: &BTreeMap<String, TableStats>,
        cache: &mut ValueCache,
    ) -> Result<Result<Evidence, Rejected>, Error> {
        let (ct, pt) = (&stats[&c.child_table], &stats[&c.parent_table]);
        let (Some(cs), Some(ps)) = (ct.columns.get(&c.child), pt.columns.get(&c.parent)) else {
            return Ok(Err(Rejected::Empty));
        };
        if let Err(r) = stats_check(cs, ps, pt) {
            return Ok(Err(r));
        }
        let parent_vals = self.column_values(db, &c.parent_table, &c.parent, cache)?;
        let parent_n = parent_vals.len();
        let parent: HashSet<Vec<u8>> = parent_vals.iter().flatten().cloned().collect();
        if parent.len() != parent_n {
            return Ok(Err(Rejected::NotUnique));
        }
        let child_vals = self.column_values(db, &c.child_table, &c.child, cache)?;
        let mut child_distinct = HashSet::new();
        let (mut contained, mut missing) = (0usize, 0usize);
        for v in child_vals.iter().flatten() {
            if parent.contains(v) {
                contained += 1;
                child_distinct.insert(v.clone());
            } else {
                missing += 1;
            }
        }
        if missing > 0 {
            return Ok(Err(Rejected::NotContained(missing)));
        }
        if contained == 0 {
            return Ok(Err(Rejected::Empty));
        }
        KEY_FIELDS.with(|f| *f.borrow_mut() = (c.child.clone(), c.parent.clone()));
        let arrs = [
            Arrangement {
                name: "child".into(),
                table: c.child_table.clone(),
                key: left_key,
            },
            Arrangement {
                name: "parent".into(),
                table: c.parent_table.clone(),
                key: right_key,
            },
        ];
        let j = Join {
            name: "key".into(),
            left: "child".into(),
            right: "parent".into(),
            combine: nothing,
        };
        let (join_root, rows) = self.join_recompute(&arrs, &j, db)?;
        if rows.is_empty() {
            return Ok(Err(Rejected::EmptyJoin));
        }
        Ok(Ok(Evidence {
            name_score: c.name_score,
            child_rows: ct.rows,
            child_nulls: ct.rows - (cs.count - cs.nulls),
            child_distinct_est: cs.distinct.estimate(),
            parent_rows: pt.rows,
            parent_distinct_est: ps.distinct.estimate(),
            ranges: (cs.numbers > 0 && ps.numbers > 0)
                .then_some(((cs.min, cs.max), (ps.min, ps.max))),
            contained,
            child_distinct: child_distinct.len(),
            parent_distinct: parent.len(),
            join_rows: rows.len(),
            join_root,
        }))
    }

    /// Keep confirmed pairs with their evidence: a graph with a node per
    /// pair (`Candidate::label`), its content the evidence's JSON.
    pub fn put_join_keys(&mut self, kept: &[(Candidate, Evidence)]) -> Result<Id, Error> {
        let mut nodes = Vec::with_capacity(kept.len());
        for (c, e) in kept {
            let id = self.put_content(e.to_json(c).as_bytes())?.id;
            nodes.push(Node {
                label: c.label(),
                target: Target::Content(Arg::Whole(id)),
                edges: Vec::new(),
            });
        }
        nodes.sort_by(|a, b| a.label.cmp(&b.label));
        nodes.dedup_by(|a, b| a.label == b.label);
        self.put_graph(nodes)
    }
}
