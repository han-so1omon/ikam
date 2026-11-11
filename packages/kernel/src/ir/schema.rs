//! E202: what the IR's columns say about the data.
//!
//! The IR puts every line into a template (its skeleton) and slots (its
//! cells). Across files, the same template's slot is a column, and some
//! columns follow from another column of the same line: a link's lag from
//! its id, a client's region from its name. `predict` (E200) learns these
//! online to store a cell in one byte; here they are read out.
//!
//! - [`dependencies`]: for each column, the column of its line it follows
//!   from best, counted in file order: how many lines repeat the value
//!   their key had last time (a key's first line counts as a miss, so a
//!   near-unique column is no key). A column is
//!   kept when that holds often and explains at least half of the column's
//!   changes, so a constant column and a key seen once are left out.
//! - [`events`]: along each dependency, per key, a value that differs from
//!   the one before and is back to it on the next line (a blip, likely an
//!   error), a value that differs and holds (a change), and a key with two
//!   values in one file (a conflict: two rows share what should be an id).

use std::collections::HashMap;

use super::lex_lines;

/// A column that follows from another column of its line.
#[derive(Clone, Debug, PartialEq)]
pub struct Dependency {
    /// The line skeleton, as `ir::encode` stores it (see `ir::segments`).
    pub skeleton: Vec<u8>,
    /// The key's slot and the dependent slot.
    pub from: usize,
    pub to: usize,
    /// Lines whose key had been seen before, and how many of them repeated
    /// the key's last value.
    pub seen: usize,
    pub held: usize,
    /// Distinct keys.
    pub keys: usize,
    /// One key and its value, as an example.
    pub example: (Vec<u8>, Vec<u8>),
}

impl Dependency {
    /// The share of lines that repeat their key's last value, counting each
    /// key's first line as a miss: a key with a value of its own on every
    /// line predicts nothing.
    pub fn strength(&self) -> f64 {
        self.held as f64 / (self.seen + self.keys).max(1) as f64
    }
}

/// A place in the files: file index, line index within the file.
pub type Place = (usize, usize);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Differs from the key's value before and after, which agree.
    Blip,
    /// Differs from the key's value before and holds on the next line.
    Change,
    /// The key has another value elsewhere in the same file: two rows
    /// share a key that should name one thing. Blips and changes then
    /// follow each of its rows by place (its first row in a file, its
    /// second, ...).
    Conflict,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub kind: Kind,
    pub at: Place,
    /// Index into the dependencies.
    pub dep: usize,
    pub key: Vec<u8>,
    pub was: Vec<u8>,
    pub now: Vec<u8>,
}

/// One key's values along a dependency, with their places.
type Run<'a> = Vec<(Place, &'a [u8])>;

/// Every line of every file: (skeleton id, place, cells).
struct Rows {
    skeletons: Vec<Vec<u8>>,
    rows: Vec<Vec<(Place, Vec<Vec<u8>>)>>,
}

fn rows(files: &[(String, Vec<u8>)]) -> Rows {
    let mut ids: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut out = Rows {
        skeletons: Vec::new(),
        rows: Vec::new(),
    };
    for (fi, (name, b)) in files.iter().enumerate() {
        for (li, (sk, cells)) in lex_lines(name, b).into_iter().enumerate() {
            if cells.len() < 2 {
                continue;
            }
            let id = *ids.entry(sk.clone()).or_insert_with(|| {
                out.skeletons.push(sk);
                out.rows.push(Vec::new());
                out.rows.len() - 1
            });
            out.rows[id].push(((fi, li), cells));
        }
    }
    out
}

/// The columns that follow from another column of their line: for each
/// skeleton with at least `min_rows` repeated keys and each slot, the slot
/// it follows from best, if that holds on at least `min_strength` of them
/// and explains at least half of the slot's changes. Ties go to the lower
/// slot.
pub fn dependencies(
    files: &[(String, Vec<u8>)],
    min_rows: usize,
    min_strength: f64,
) -> Vec<Dependency> {
    let r = rows(files);
    let mut out = Vec::new();
    for (sk, rows) in r.skeletons.iter().zip(&r.rows) {
        let width = rows[0].1.len();
        for to in 0..width {
            // how often the column changes from one line to the next
            let changes = rows.windows(2).filter(|w| w[0].1[to] != w[1].1[to]).count();
            if changes == 0 {
                continue;
            }
            let base = changes as f64 / (rows.len() - 1) as f64;
            let mut best: Option<Dependency> = None;
            for from in (0..width).filter(|&f| f != to) {
                let mut last: HashMap<&[u8], &[u8]> = HashMap::new();
                let (mut seen, mut held) = (0, 0);
                let mut example = None;
                for (_, cells) in rows {
                    let (k, v) = (cells[from].as_slice(), cells[to].as_slice());
                    if let Some(old) = last.insert(k, v) {
                        seen += 1;
                        if old == v {
                            held += 1;
                            example.get_or_insert((k, v));
                        }
                    }
                }
                if seen < min_rows {
                    continue;
                }
                let d = Dependency {
                    skeleton: sk.clone(),
                    from,
                    to,
                    seen,
                    held,
                    keys: last.len(),
                    example: example
                        .map_or_else(Default::default, |(k, v)| (k.to_vec(), v.to_vec())),
                };
                let miss = 1.0 - d.strength();
                if d.strength() >= min_strength
                    && miss <= base / 2.0
                    && best.as_ref().is_none_or(|b| d.strength() > b.strength())
                {
                    best = Some(d);
                }
            }
            out.extend(best);
        }
    }
    out
}

/// Blips and changes along each dependency, in file order.
pub fn events(files: &[(String, Vec<u8>)], deps: &[Dependency]) -> Vec<Event> {
    let r = rows(files);
    let by_sk: HashMap<&[u8], usize> = r
        .skeletons
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_slice(), i))
        .collect();
    let mut out = Vec::new();
    for (di, d) in deps.iter().enumerate() {
        let Some(&si) = by_sk.get(d.skeleton.as_slice()) else {
            continue;
        };
        // per key and file, its values in order: two values in one file
        // are a conflict
        let mut seen: HashMap<(&[u8], usize), Vec<&[u8]>> = HashMap::new();
        let mut runs: HashMap<(&[u8], usize), Run> = HashMap::new();
        for (at, cells) in &r.rows[si] {
            let (Some(k), Some(v)) = (cells.get(d.from), cells.get(d.to)) else {
                continue;
            };
            let here = seen.entry((k.as_slice(), at.0)).or_default();
            if let Some(&first) = here.iter().find(|x| **x != v.as_slice()) {
                out.push(Event {
                    kind: Kind::Conflict,
                    at: *at,
                    dep: di,
                    key: k.clone(),
                    was: first.to_vec(),
                    now: v.clone(),
                });
            }
            let nth = here.len();
            here.push(v);
            runs.entry((k.as_slice(), nth)).or_default().push((*at, v));
        }
        for ((k, _), run) in runs {
            let mut back = false;
            for i in 1..run.len() {
                let (at, v) = run[i];
                let was = run[i - 1].1;
                // the line after a blip is the value coming back, not a change
                if v == was || std::mem::take(&mut back) {
                    continue;
                }
                let kind = match run.get(i + 1) {
                    Some(&(_, next)) if next == was => {
                        back = true;
                        Kind::Blip
                    }
                    Some(&(_, next)) if next == v => Kind::Change,
                    None => Kind::Change,
                    // a third value: the next line decides
                    Some(_) => continue,
                };
                out.push(Event {
                    kind,
                    at,
                    dep: di,
                    key: k.to_vec(),
                    was: was.to_vec(),
                    now: v.to_vec(),
                });
            }
        }
    }
    out.sort_by_key(|e| (e.at, e.dep));
    out
}
