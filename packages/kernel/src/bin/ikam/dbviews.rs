//! Kept views over `db serve` (E361): groups and secondary indexes that the
//! server keeps for every database, so a client reads a total per group or
//! the rows with a field's value instead of scanning a table and grouping
//! or filtering in its own code.
//!
//! The views are the kernel's (src/views.rs, E242), incremental, over the
//! database's `kv`, with rules read from a file (`--views FILE`), one view a
//! line (`#` starts a comment):
//!
//! ```text
//! NAME group PREFIX by=KEY [sum=field:F]
//! NAME index PREFIX by=KEY
//! ```
//!
//! KEY is `field:F` (a top-level JSON field of the row's value: a string,
//! a number or a boolean) or `seg:N` (the Nth `/` segment of the key after
//! PREFIX, from 0). A row under PREFIX that has no such key is not in the
//! view.
//!
//! - A group keeps `count,sum` per key (the kernel's `GroupSum`). `sum`
//!   adds a whole-number field (a fraction is rounded; a row without it
//!   adds 0); without `sum` the sum is 0.
//! - An index maps KEY's value, U+001F and the row's key after PREFIX to the
//!   row's key, so the rows with a value are one prefix of the view.
//!
//! A database's views are built on its first view request and kept in
//! memory with the head they reflect; each later request brings them to
//! the head from the `kv` diff (`Repo::views_update`), so a request pays for
//! the rows changed since the last one. Appends do not wait for views. A
//! restarted server builds again on its first view request; the graphs it
//! writes are not referenced by a commit.
//!
//! Routes, under `/db/NAME/`:
//! - `GET views`: `[{"name":...,"kind":"group"|"index","prefix":...}]`.
//! - `GET view?name=V`: a group's rows as frames, the group and `count,sum`;
//!   `prefix=P` keeps groups starting with P; `limit=N`.
//! - `GET view?name=V&eq=X`: an index's rows with value X, as frames of
//!   the row's key and value (read at the same head); `keys=1` leaves the
//!   values out (empty), `limit=N`.
//!
//! Add `format=json` for `[["key","value"],...]` (values must be UTF-8).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ikam_kernel::views::{GroupRule, MapRule, Mode, View, ViewGraph, ViewOp, ViewState};
use ikam_kernel::{Db, Error, Repo, Store};

use crate::json::{Json, quote};

/// What a view keys rows by.
#[derive(Clone, Debug, PartialEq)]
pub enum By {
    Field(String),
    Seg(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Group { sum: Option<String> },
    Index,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub name: String,
    pub prefix: String,
    pub by: By,
    pub kind: Kind,
}

/// The separator between an index's value and the row's key.
pub const US: char = '\u{1f}';

fn field_text(v: &Json) -> Option<String> {
    match v {
        Json::Str(s) => Some(s.clone()),
        Json::Num(n) => Some(if n.fract() == 0.0 && n.abs() < 9.0e15 {
            format!("{}", *n as i64)
        } else {
            format!("{n}")
        }),
        Json::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn parsed(value: &[u8]) -> Option<Json> {
    crate::json::parse(std::str::from_utf8(value).ok()?).ok()
}

impl By {
    /// The key of the row `rest` (its key after the prefix) with `value`;
    /// `doc` is the parsed value, if parsed already.
    fn key(&self, rest: &str, doc: Option<&Json>) -> Option<String> {
        match self {
            By::Seg(n) => rest.split('/').nth(*n).map(str::to_string),
            By::Field(f) => doc?.get(f).and_then(field_text),
        }
    }

    fn parse(s: &str) -> Result<By, String> {
        if let Some(f) = s.strip_prefix("field:").filter(|f| !f.is_empty()) {
            return Ok(By::Field(f.to_string()));
        }
        if let Some(n) = s.strip_prefix("seg:") {
            return n.parse().map(By::Seg).map_err(|_| format!("bad seg: {s}"));
        }
        Err(format!("by= is field:F or seg:N, not {s}"))
    }

    fn needs_doc(&self) -> bool {
        matches!(self, By::Field(_))
    }
}

fn whole(v: &Json) -> Option<i64> {
    match v {
        Json::Num(n) if n.is_finite() && n.abs() < 9.0e18 => Some(n.round() as i64),
        _ => None,
    }
}

impl Rule {
    fn op(&self) -> ViewOp {
        let prefix = self.prefix.clone();
        let by = self.by.clone();
        match &self.kind {
            Kind::Group { sum } => {
                let sum = sum.clone();
                let doc_needed = by.needs_doc() || sum.is_some();
                let f: GroupRule = Arc::new(move |k: &str, v: &[u8]| {
                    let rest = k.strip_prefix(prefix.as_str())?;
                    let doc = if doc_needed { parsed(v) } else { None };
                    let g = by.key(rest, doc.as_ref())?;
                    let m = match &sum {
                        Some(f) => doc.as_ref().and_then(|d| d.get(f)).and_then(whole),
                        None => None,
                    };
                    Some((g, m.unwrap_or(0)))
                });
                ViewOp::GroupSumWith(f)
            }
            Kind::Index => {
                let f: MapRule = Arc::new(move |k: &str, v: &[u8]| {
                    let rest = k.strip_prefix(prefix.as_str())?;
                    let doc = if by.needs_doc() { parsed(v) } else { None };
                    let x = by.key(rest, doc.as_ref())?;
                    if x.contains(US) {
                        return None;
                    }
                    Some((format!("{x}{US}{rest}"), k.as_bytes().to_vec()))
                });
                ViewOp::MapWith(f)
            }
        }
    }

    fn json(&self) -> String {
        let kind = match self.kind {
            Kind::Group { .. } => "group",
            Kind::Index => "index",
        };
        format!(
            "{{\"name\":{},\"kind\":\"{kind}\",\"prefix\":{}}}",
            quote(&self.name),
            quote(&self.prefix)
        )
    }
}

/// Parse a views file.
pub fn parse(text: &str) -> Result<Vec<Rule>, String> {
    let mut out: Vec<Rule> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let bad = |m: &str| format!("views line {}: {m}", i + 1);
        let words: Vec<&str> = line.split_whitespace().collect();
        let [name, kind, prefix, rest @ ..] = words.as_slice() else {
            return Err(bad("NAME group|index PREFIX by=..."));
        };
        let ok_name = name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b));
        if !ok_name || *name == "kv" || out.iter().any(|r| r.name == *name) {
            return Err(bad(&format!("bad or repeated name {name}")));
        }
        let mut by = None;
        let mut sum = None;
        for w in rest {
            match w.split_once('=') {
                Some(("by", v)) => by = Some(By::parse(v).map_err(|m| bad(&m))?),
                Some(("sum", v)) => match v.strip_prefix("field:").filter(|f| !f.is_empty()) {
                    Some(f) => sum = Some(f.to_string()),
                    None => return Err(bad("sum= is field:F")),
                },
                _ => return Err(bad(&format!("unknown {w}"))),
            }
        }
        let by = by.ok_or_else(|| bad("by= is needed"))?;
        let kind = match *kind {
            "group" => Kind::Group { sum },
            "index" if sum.is_none() => Kind::Index,
            _ => return Err(bad("the kind is group, or index without sum=")),
        };
        out.push(Rule {
            name: name.to_string(),
            prefix: prefix.to_string(),
            by,
            kind,
        });
    }
    Ok(out)
}

/// The views a server keeps, and each database's state.
pub struct Served {
    rules: Vec<Rule>,
    graph: ViewGraph,
    states: Mutex<HashMap<String, ViewState>>,
}

impl Served {
    pub fn new(rules: Vec<Rule>) -> Result<Served, String> {
        let views = rules
            .iter()
            .map(|r| View {
                name: r.name.clone(),
                input: "kv".into(),
                mode: Mode::Incremental,
                op: r.op(),
                check: None,
            })
            .collect();
        let graph = ViewGraph::new(views).map_err(|e| e.to_string())?;
        Ok(Served {
            rules,
            graph,
            states: Mutex::new(HashMap::new()),
        })
    }

    pub fn list_json(&self) -> String {
        let items: Vec<String> = self.rules.iter().map(Rule::json).collect();
        format!("[{}]", items.join(","))
    }

    pub fn rule(&self, name: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.name == name)
    }

    /// `name`'s views at `db`: built the first time, else brought to `db`.
    /// Takes the writer, since it writes the views' graphs.
    pub fn at<S: Store>(
        &self,
        writer: &mut Repo<S>,
        name: &str,
        db: &Db,
    ) -> Result<ViewState, Error> {
        let kept = self.states.lock().unwrap().get(name).cloned();
        let state = match kept {
            Some(s) if s.db == *db => return Ok(s),
            Some(s) => {
                // The same changes either way (E249); it skips the chunks
                // both heads share instead of listing every leaf of both.
                let prune = std::mem::replace(&mut writer.prune_diff, true);
                let out = writer.views_update(&self.graph, &s, db);
                writer.prune_diff = prune;
                out?
            }
            None => writer.views_build(&self.graph, db)?,
        };
        self.states
            .lock()
            .unwrap()
            .insert(name.to_string(), state.clone());
        Ok(state)
    }

    /// Rows of view `view` at `state` starting with `prefix`.
    pub fn rows<S: Store>(
        &self,
        repo: &Repo<S>,
        state: &ViewState,
        view: &str,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<(String, Vec<u8>)>, Error> {
        repo.view_prefix(&self.graph, state, view, prefix, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_files_parse_and_refuse() {
        let r = parse(
            "# orders\nby_status group t/o/ by=field:status sum=field:amount\n\
             per_day group t/e/ by=seg:0  # first segment\n\
             by_cust index t/o/ by=field:customer\n",
        )
        .unwrap();
        assert_eq!(r.len(), 3);
        assert_eq!(r[1].by, By::Seg(0));
        assert_eq!(
            r[0].kind,
            Kind::Group {
                sum: Some("amount".into())
            }
        );
        for bad in [
            "a group t/",
            "a sort t/ by=seg:0",
            "a index t/ by=field:x sum=field:y",
            "a group t/ by=col:x",
            "kv group t/ by=seg:0",
            "a group t/ by=seg:0\na index t/ by=seg:1",
            "a b group t/ by=seg:0",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rules_key_rows() {
        let r = parse("g group t/ by=field:k sum=field:n\ni index t/ by=seg:1").unwrap();
        let ViewOp::GroupSumWith(g) = r[0].op() else {
            panic!()
        };
        assert_eq!(g("t/1", br#"{"k":"a","n":2.6}"#), Some(("a".into(), 3)));
        assert_eq!(g("t/1", br#"{"k":7}"#), Some(("7".into(), 0)));
        assert_eq!(g("t/1", br#"{"n":1}"#), None);
        assert_eq!(g("u/1", br#"{"k":"a"}"#), None);
        assert_eq!(g("t/1", b"not json"), None);
        let ViewOp::MapWith(m) = r[1].op() else {
            panic!()
        };
        assert_eq!(
            m("t/x/y/z", b""),
            Some((format!("y{US}x/y/z"), b"t/x/y/z".to_vec()))
        );
        assert_eq!(m("t/x", b""), None);
    }
}
