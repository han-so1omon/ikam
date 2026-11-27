//! Answers pinned to a commit, flagged stale when their data changes
//! (E421). An answer is a record of a question, the query that answered
//! it, the definitions version it used, the data commit it read, its
//! result and its read set: the key ranges it read and, within them,
//! which fields of a row could change its result.
//!
//! - A `pure` answer's query is deterministic: rerunning it at its commit
//!   gives its result byte for byte, and rerunning it at a later commit
//!   is a full recheck.
//! - A `recorded` answer's result came from a step that cannot be rerun
//!   (an agent, a model, anything nondeterministic). Its output is kept as
//!   it was given; only its read set says when it may be out of date.
//!
//! When the data moves from one commit to another, `answers_flag` reads
//! the rows that differ (only the chunks the two versions do not share)
//! and marks stale exactly the answers whose read set a changed row
//! falls in. A row falls in a read set when its key is in one of the
//! ranges and either it starts or stops matching the read set's filter,
//! or it matches before or after (always, with no filter) and the fields
//! the answer reads differ between its old and new value (a missing row
//! has none).
//!
//! Records are contents; a set of answers is a graph with a node
//! `a/NAME` per record and `s/NAME` per stale mark (the commit the data
//! changed at). The records never change: a stale mark sits beside them.
//!
//! Opt-in: nothing in the kernel records or flags answers unless a caller
//! asks.

use std::collections::BTreeMap;

use crate::{Arg, Db, Error, Id, Node, Repo, Store, Target};

const MAGIC: &str = "ikam-answer/1";

/// A deterministic query over one database's rows. Rows are JSON arrays
/// or objects; a field is an array position ("0", "1", ...) or an
/// object's key, its value the field's JSON text as stored.
#[derive(Clone, Debug, PartialEq)]
pub enum Query {
    /// The row at `key`, its value as stored (empty if it is not set).
    Get { key: String },
    /// The sum of a number field over the rows from `start` to `end`
    /// (exclusive), as Rust prints an f64, and how many rows had it.
    Sum {
        start: String,
        end: String,
        field: String,
    },
    /// How many rows under `prefix` have `field` equal to `value` (JSON
    /// text, compared byte for byte).
    CountEq {
        prefix: String,
        field: String,
        value: Vec<u8>,
    },
}

/// What an answer read.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ReadSet {
    /// Key ranges, `[start, end)`.
    pub ranges: Vec<(String, String)>,
    /// The fields whose values the result depends on; `None`: the whole
    /// row.
    pub fields: Option<Vec<String>>,
    /// Only rows whose field equals this value (before or after) count.
    pub filter: Option<(String, Vec<u8>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerKind {
    Pure,
    Recorded,
}

/// One answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub question: String,
    /// The query text: `Query::text` for a pure answer, free text for a
    /// recorded one.
    pub query: String,
    pub kind: AnswerKind,
    /// The definitions version the answer used, if any (E419's model).
    pub definitions: Option<Id>,
    /// The data commit it read.
    pub commit: Id,
    pub read: ReadSet,
    pub result: Vec<u8>,
}

/// What `answers_flag` found.
#[derive(Clone, Debug, Default)]
pub struct Flagged {
    /// The answer set's new root, with the new stale marks.
    pub root: Option<Id>,
    /// Names newly marked stale.
    pub stale: Vec<String>,
    /// Rows that differ between the two commits.
    pub rows_changed: usize,
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// The end of the JSON value starting at `i`.
fn value_end(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i;
    match *b.get(j)? {
        b'"' => {
            j += 1;
            while j < b.len() {
                match b[j] {
                    b'\\' => j += 2,
                    b'"' => return Some(j + 1),
                    _ => j += 1,
                }
            }
            None
        }
        b'[' | b'{' => {
            let mut depth = 0usize;
            while j < b.len() {
                match b[j] {
                    b'"' => {
                        j = value_end(b, j)?;
                        continue;
                    }
                    b'[' | b'{' => depth += 1,
                    b']' | b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(j + 1);
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            None
        }
        _ => {
            while j < b.len() && !matches!(b[j], b',' | b']' | b'}' | b' ' | b'\t' | b'\n' | b'\r')
            {
                j += 1;
            }
            (j > i).then_some(j)
        }
    }
}

/// A top-level field's JSON text in a row, if it has it.
pub fn field<'a>(row: &'a [u8], name: &str) -> Option<&'a [u8]> {
    let mut i = skip_ws(row, 0);
    let (object, close) = match row.get(i)? {
        b'{' => (true, b'}'),
        b'[' => (false, b']'),
        _ => return None,
    };
    let want: Option<usize> = if object { None } else { name.parse().ok() };
    if !object && want.is_none() {
        return None;
    }
    i += 1;
    let mut pos = 0usize;
    loop {
        i = skip_ws(row, i);
        if *row.get(i)? == close {
            return None;
        }
        let hit = if object {
            let end = value_end(row, i).filter(|_| row[i] == b'"')?;
            let hit = &row[i + 1..end - 1] == name.as_bytes();
            i = skip_ws(row, end);
            if row.get(i) != Some(&b':') {
                return None;
            }
            i = skip_ws(row, i + 1);
            hit
        } else {
            pos += 1;
            Some(pos - 1) == want
        };
        let end = value_end(row, i)?;
        if hit {
            return Some(&row[i..end]);
        }
        i = skip_ws(row, end);
        match row.get(i)? {
            b',' => i += 1,
            _ => return None,
        }
    }
}

impl Query {
    /// The query as one line of text (the record's `query`).
    pub fn text(&self) -> String {
        match self {
            Query::Get { key } => format!("get {key}"),
            Query::Sum { start, end, field } => format!("sum {start} {end} {field}"),
            Query::CountEq {
                prefix,
                field,
                value,
            } => format!("count {prefix} {field} {}", hex(value)),
        }
    }

    /// The query a `text` line names.
    pub fn parse(s: &str) -> Option<Query> {
        let p: Vec<&str> = s.split(' ').collect();
        match p.as_slice() {
            ["get", key] => Some(Query::Get {
                key: key.to_string(),
            }),
            ["sum", start, end, field] => Some(Query::Sum {
                start: start.to_string(),
                end: end.to_string(),
                field: field.to_string(),
            }),
            ["count", prefix, field, value] => Some(Query::CountEq {
                prefix: prefix.to_string(),
                field: field.to_string(),
                value: unhex(value)?,
            }),
            _ => None,
        }
    }

    /// What this query reads.
    pub fn read_set(&self) -> ReadSet {
        match self {
            Query::Get { key } => ReadSet {
                ranges: vec![(key.clone(), format!("{key}\0"))],
                fields: None,
                filter: None,
            },
            Query::Sum { start, end, field } => ReadSet {
                ranges: vec![(start.clone(), end.clone())],
                fields: Some(vec![field.clone()]),
                filter: None,
            },
            Query::CountEq {
                prefix,
                field,
                value,
            } => ReadSet {
                ranges: vec![(
                    prefix.clone(),
                    crate::prefix_end(prefix).unwrap_or_else(|| "\u{10ffff}".into()),
                )],
                fields: Some(vec![field.clone()]),
                filter: Some((field.clone(), value.clone())),
            },
        }
    }
}

impl ReadSet {
    /// Does a row's change from `before` to `after` (`None`: not set) fall
    /// in this read set? The key is taken to be in one of its ranges. With
    /// a filter, a row that starts or stops matching it does; one that
    /// matches neither before nor after does not.
    pub fn touched_by(&self, before: Option<&[u8]>, after: Option<&[u8]>) -> bool {
        if let Some((f, v)) = &self.filter {
            let matches = |row: Option<&[u8]>| row.and_then(|r| field(r, f)) == Some(&v[..]);
            match (matches(before), matches(after)) {
                (false, false) => return false,
                (true, true) => {}
                _ => return true,
            }
        }
        match &self.fields {
            None => before != after,
            Some(fs) => fs
                .iter()
                .any(|f| before.and_then(|r| field(r, f)) != after.and_then(|r| field(r, f))),
        }
    }
}

fn hex(b: &[u8]) -> String {
    if b.is_empty() {
        return "-".into();
    }
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s == "-" {
        return Some(Vec::new());
    }
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

impl Answer {
    /// The record's bytes: a header of `key: value` lines (texts and
    /// keys hex-encoded where they could hold a newline), then the result.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = format!(
            "{MAGIC}\nquestion: {}\nquery: {}\nkind: {}\ndefinitions: {}\ncommit: {}\n",
            hex(self.question.as_bytes()),
            hex(self.query.as_bytes()),
            match self.kind {
                AnswerKind::Pure => "pure",
                AnswerKind::Recorded => "recorded",
            },
            self.definitions.map_or("-".into(), |d| d.to_string()),
            self.commit,
        );
        for (s, e) in &self.read.ranges {
            out.push_str(&format!(
                "range: {} {}\n",
                hex(s.as_bytes()),
                hex(e.as_bytes())
            ));
        }
        if let Some(fs) = &self.read.fields {
            let fs: Vec<String> = fs.iter().map(|f| hex(f.as_bytes())).collect();
            out.push_str(&format!("fields: {}\n", fs.join(" ")));
        }
        if let Some((f, v)) = &self.read.filter {
            out.push_str(&format!("filter: {} {}\n", hex(f.as_bytes()), hex(v)));
        }
        out.push_str(&format!("result: {}\n", self.result.len()));
        let mut out = out.into_bytes();
        out.extend_from_slice(&self.result);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Answer, Error> {
        let bad = || Error::Decode("answer record");
        let text = |s: &str| -> Result<String, Error> {
            String::from_utf8(unhex(s).ok_or_else(bad)?).map_err(|_| bad())
        };
        let mut i = 0usize;
        let mut line = || -> Result<&str, Error> {
            let end = memchr::memchr(b'\n', &bytes[i..]).ok_or_else(bad)? + i;
            let l = std::str::from_utf8(&bytes[i..end]).map_err(|_| bad())?;
            i = end + 1;
            Ok(l)
        };
        if line()? != MAGIC {
            return Err(bad());
        }
        let mut a = Answer {
            question: String::new(),
            query: String::new(),
            kind: AnswerKind::Pure,
            definitions: None,
            commit: Id::of_content(&[]),
            read: ReadSet::default(),
            result: Vec::new(),
        };
        loop {
            let l = line()?;
            let (k, v) = l.split_once(": ").ok_or_else(bad)?;
            match k {
                "question" => a.question = text(v)?,
                "query" => a.query = text(v)?,
                "kind" => {
                    a.kind = match v {
                        "pure" => AnswerKind::Pure,
                        "recorded" => AnswerKind::Recorded,
                        _ => return Err(bad()),
                    }
                }
                "definitions" => {
                    a.definitions = match v {
                        "-" => None,
                        id => Some(id.parse().map_err(|_| bad())?),
                    }
                }
                "commit" => a.commit = v.parse().map_err(|_| bad())?,
                "range" => {
                    let (s, e) = v.split_once(' ').ok_or_else(bad)?;
                    a.read.ranges.push((text(s)?, text(e)?));
                }
                "fields" => {
                    a.read.fields = Some(v.split(' ').map(text).collect::<Result<_, _>>()?);
                }
                "filter" => {
                    let (f, val) = v.split_once(' ').ok_or_else(bad)?;
                    a.read.filter = Some((text(f)?, unhex(val).ok_or_else(bad)?));
                }
                "result" => {
                    let n: usize = v.parse().map_err(|_| bad())?;
                    let r = bytes.get(i..).ok_or_else(bad)?;
                    if r.len() != n {
                        return Err(bad());
                    }
                    a.result = r.to_vec();
                    return Ok(a);
                }
                _ => return Err(bad()),
            }
        }
    }
}

impl<S: Store> Repo<S> {
    /// Run a query at `db`: its result bytes.
    pub fn answer_query(&self, db: &Db, q: &Query) -> Result<Vec<u8>, Error> {
        match q {
            Query::Get { key } => Ok(self.db_get(db, key)?.unwrap_or_default()),
            Query::Sum {
                start,
                end,
                field: f,
            } => {
                let (mut sum, mut n) = (0.0f64, 0u64);
                for (_, v) in self.db_range(db, start, Some(end), usize::MAX, false)? {
                    if let Some(x) = field(&v, f)
                        .and_then(|t| std::str::from_utf8(t).ok())
                        .and_then(|t| t.parse::<f64>().ok())
                    {
                        sum += x;
                        n += 1;
                    }
                }
                Ok(format!("{sum:?} {n}").into_bytes())
            }
            Query::CountEq {
                prefix,
                field: f,
                value,
            } => {
                let n = self
                    .db_prefix(db, prefix, usize::MAX, false)?
                    .iter()
                    .filter(|(_, v)| field(v, f) == Some(&value[..]))
                    .count();
                Ok(n.to_string().into_bytes())
            }
        }
    }

    /// Answer `question` with a pure query at the data commit `commit`.
    pub fn answer_pure(
        &mut self,
        question: &str,
        q: &Query,
        commit: &Id,
        definitions: Option<Id>,
    ) -> Result<Answer, Error> {
        let db = self.db_at(commit)?;
        Ok(Answer {
            question: question.into(),
            query: q.text(),
            kind: AnswerKind::Pure,
            definitions,
            commit: *commit,
            read: q.read_set(),
            result: self.answer_query(&db, q)?,
        })
    }

    /// Rerun a pure answer at `commit`: is its result still the same?
    /// `None` for a recorded answer, which cannot be rerun.
    pub fn answer_recheck(&mut self, a: &Answer, commit: &Id) -> Result<Option<bool>, Error> {
        if a.kind != AnswerKind::Pure {
            return Ok(None);
        }
        let q = Query::parse(&a.query).ok_or(Error::Decode("answer query"))?;
        let db = self.db_at(commit)?;
        Ok(Some(self.answer_query(&db, &q)? == a.result))
    }

    /// Store answers by name as a new answer set: a graph of `a/NAME`
    /// nodes, each pointing at its record.
    pub fn answers_put(&mut self, answers: &[(String, Answer)]) -> Result<Id, Error> {
        let mut nodes = Vec::with_capacity(answers.len());
        for (name, a) in answers {
            let id = self.put_content(&a.encode())?.id;
            nodes.push(Node {
                label: format!("a/{name}"),
                target: Target::Content(Arg::Whole(id)),
                edges: Vec::new(),
            });
        }
        self.put_graph(nodes)
    }

    /// Every answer in a set, by name, and the commit each was marked
    /// stale at, if it was.
    pub fn answers_read(&self, set: &Id) -> Result<BTreeMap<String, (Answer, Option<Id>)>, Error> {
        let mut out = BTreeMap::new();
        let mut marks = BTreeMap::new();
        for (label, v) in self.node_values(self.graph_nodes(set)?)? {
            if let Some(name) = label.strip_prefix("a/") {
                out.insert(name.to_string(), (Answer::decode(&v)?, None));
            } else if let Some(name) = label.strip_prefix("s/") {
                let c = std::str::from_utf8(&v)
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .ok_or(Error::Decode("stale mark"))?;
                marks.insert(name.to_string(), c);
            }
        }
        for (name, c) in marks {
            if let Some(e) = out.get_mut(&name) {
                e.1 = Some(c);
            }
        }
        Ok(out)
    }

    /// Mark stale the answers of `set` that the change from data commit
    /// `from` to `to` touches; answers already marked keep their mark.
    /// Reads only the rows that differ, and each record once.
    pub fn answers_flag(&mut self, set: &Id, from: &Id, to: &Id) -> Result<Flagged, Error> {
        let (a, b) = (self.db_at(from)?, self.db_at(to)?);
        let delta = self.kv_delta(&a.kv, &b.kv)?;
        let answers = self.answers_read(set)?;
        let mut stale = Vec::new();
        for (name, (ans, mark)) in &answers {
            if mark.is_some() {
                continue;
            }
            let hit = ans.read.ranges.iter().any(|(s, e)| {
                delta
                    .range::<str, _>((
                        std::ops::Bound::Included(s.as_str()),
                        std::ops::Bound::Excluded(e.as_str()),
                    ))
                    .any(|(_, (before, after))| {
                        ans.read.touched_by(before.as_deref(), after.as_deref())
                    })
            });
            if hit {
                stale.push(name.clone());
            }
        }
        if stale.is_empty() {
            return Ok(Flagged {
                root: Some(*set),
                stale,
                rows_changed: delta.len(),
            });
        }
        let mark = self.put_content(to.to_string().as_bytes())?.id;
        let mut nodes = self.graph_nodes(set)?;
        nodes.extend(stale.iter().map(|name| Node {
            label: format!("s/{name}"),
            target: Target::Content(Arg::Whole(mark)),
            edges: Vec::new(),
        }));
        nodes.sort_by(|x, y| x.label.cmp(&y.label));
        Ok(Flagged {
            root: Some(self.put_graph(nodes)?),
            stale,
            rows_changed: delta.len(),
        })
    }
}
