//! Shared parsing for the experiment loop examples (E275, E276): the
//! registry, the log and the research notes, read as plain text, and the
//! gap candidates they hold.
//!
//! A gap is something the project's own records say is worth an
//! experiment and has not had one: a stale registry verdict, a "Not
//! measured" line in a log entry, a research hypothesis still `open`, or
//! an experiment id the log mentions that has no registry row.

#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The objectives the scores measure now; kept in step with
/// `examples/registry.rs`.
pub const CURRENT: [&str; 2] = ["cost", "north"];

/// Mechanisms that matter under the current objective; kept in step with
/// `examples/registry.rs`.
pub const WEIGHTY: [&str; 8] = [
    "read-path",
    "search",
    "repack",
    "planner",
    "programs",
    "graph",
    "speed",
    "blocks",
];

#[derive(Clone, Debug)]
pub struct Row {
    pub id: u32,
    pub slug: String,
    pub kind: String,
    pub verdict: String,
    pub recorded: String,
    pub objective: String,
    pub mechanisms: Vec<String>,
    pub weight_sensitive: bool,
    pub note: String,
}

impl Row {
    /// Same rule as `examples/registry.rs`.
    pub fn relevance(&self) -> &'static str {
        match () {
            _ if self.kind == "benchmark" => "reference",
            _ if CURRENT.contains(&self.objective.as_str()) => "current",
            _ if !self.weight_sensitive => "holds",
            _ => "stale",
        }
    }

    pub fn action(&self) -> &'static str {
        match (self.relevance(), self.verdict.as_str()) {
            ("stale", "kept") => "ablate",
            ("stale", _) => "retest",
            _ => "none",
        }
    }

    pub fn weighty(&self) -> bool {
        self.mechanisms
            .iter()
            .any(|m| WEIGHTY.contains(&m.as_str()))
    }
}

/// `E123` or `E0123` to 123.
pub fn parse_id(s: &str) -> Option<u32> {
    let d = s.strip_prefix('E')?;
    let digits: String = d.chars().take_while(|c| c.is_ascii_digit()).collect();
    (digits.len() >= 3).then(|| digits.parse().ok()).flatten()
}

pub fn docs() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

pub fn rows(docs: &Path) -> Vec<Row> {
    std::fs::read_to_string(docs.join("experiments/registry.tsv"))
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            Some(Row {
                id: parse_id(f[0])?,
                slug: f[1].into(),
                kind: f[2].into(),
                verdict: f[3].into(),
                recorded: f[4].into(),
                objective: f[5].into(),
                mechanisms: f[6].split(',').map(String::from).collect(),
                weight_sensitive: f[7] == "yes",
                note: f.get(8).copied().unwrap_or("").into(),
            })
        })
        .collect()
}

/// Log entries by id, in file order (`### E<nnn> <slug>` to the next one).
/// A later entry under the same id (a rerun) is appended to the first.
pub fn entries(docs: &Path) -> BTreeMap<u32, String> {
    let log = std::fs::read_to_string(docs.join("experiments/log.md")).unwrap();
    let mut out: BTreeMap<u32, String> = BTreeMap::new();
    for part in log.split("\n### ").skip(1) {
        if let Some(id) = parse_id(part) {
            let e = out.entry(id).or_default();
            if !e.is_empty() {
                e.push('\n');
            }
            e.push_str("### ");
            e.push_str(part);
        }
    }
    out
}

/// Every `E<nnn>` mentioned in a text.
pub fn mentions(text: &str) -> BTreeSet<u32> {
    let b = text.as_bytes();
    let mut out = BTreeSet::new();
    for (i, _) in text.match_indices('E') {
        if i > 0 && b[i - 1].is_ascii_alphanumeric() {
            continue;
        }
        if let Some(id) = parse_id(&text[i..]) {
            out.insert(id);
        }
    }
    out
}

/// Ranges such as "E261 to E264" or "E261-E264", expanded.
pub fn mentioned_ranges(text: &str) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    let words: Vec<&str> = text.split_whitespace().collect();
    for w in words.windows(3) {
        if let (Some(a), "to", Some(b)) = (parse_id(w[0]), w[1], parse_id(w[2]))
            && a < b
            && b - a <= 20
        {
            out.extend(a..=b);
        }
    }
    for w in &words {
        if let Some((a, b)) = w.split_once('-')
            && let (Some(a), Some(b)) = (parse_id(a), parse_id(b))
            && a < b
            && b - a <= 20
        {
            out.extend(a..=b);
        }
    }
    out
}

/// How far below the newest recorded id a mentioned, unrecorded id still
/// counts as planned (or recorded only on a branch).
pub const PLANNED_WINDOW: u32 = 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    /// A stale registry row: a kept mechanism to ablate, a rejected one to
    /// retest (registry.rs's rule).
    Stale,
    /// A "Not measured", "Not built", "Not tried" or "not run" line in a log
    /// entry.
    NotMeasured,
    /// A research note hypothesis whose status is still `open`.
    OpenHypothesis,
    /// An experiment id the log mentions (alone or in a range), within
    /// PLANNED_WINDOW of the newest, that has no registry row and no log
    /// entry here: planned, or recorded only on a branch.
    Planned,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Stale => "stale",
            Source::NotMeasured => "not-measured",
            Source::OpenHypothesis => "open-hypothesis",
            Source::Planned => "planned",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Gap {
    /// Stable label: `<source>/<where>/<n>`.
    pub label: String,
    pub source: Source,
    /// The experiment the gap comes from (for stale, not-measured) or is
    /// (for planned).
    pub from: Option<u32>,
    /// Mechanisms of the experiment it comes from, when there is one.
    pub mechanisms: Vec<String>,
    pub text: String,
}

const NOT_MEASURED: [&str; 5] = [
    "not measured",
    "not built",
    "not tried",
    "not run",
    "not yet measured",
];

/// Lines of an entry that name work left undone. Since E290: a table row
/// is not one, and a phrase inside quotes, inside a `(Source: ...)` or
/// after "does" or "did" ("the bench does not run batches") does not
/// count, since those quote or describe instead of leaving work undone.
pub fn undone_lines(entry: &str) -> Vec<String> {
    entry
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('|'))
        .filter(|l| {
            let low = unquoted(&l.to_lowercase());
            NOT_MEASURED.iter().any(|p| {
                low.match_indices(p)
                    .any(|(i, _)| !(low[..i].ends_with("does ") || low[..i].ends_with("did ")))
            })
        })
        .map(|l| l.trim_start_matches("- ").to_string())
        .collect()
}

/// A line with its quoted spans and `(source: ...)` parentheticals blanked.
fn unquoted(low: &str) -> String {
    let mut out = String::with_capacity(low.len());
    let mut rest = low;
    loop {
        let q = rest.find(['"', '\u{201c}']);
        let s = rest.find("(source:");
        let (start, close) = match (q, s) {
            (Some(q), Some(s)) if s < q => (s, ")"),
            (Some(q), _) => (
                q,
                if rest[q..].starts_with('"') {
                    "\""
                } else {
                    "\u{201d}"
                },
            ),
            (None, Some(s)) => (s, ")"),
            (None, None) => {
                out.push_str(rest);
                return out;
            }
        };
        out.push_str(&rest[..start]);
        let open_len = rest[start..].chars().next().unwrap().len_utf8();
        let after = &rest[start + open_len..];
        match after.find(close) {
            Some(e) => rest = &after[e + close.len()..],
            None => return out,
        }
        out.push(' ');
    }
}

/// Open hypotheses in the research notes: a `**Hn. Title.**` paragraph
/// whose `Status:` line says `open`.
pub fn open_hypotheses(docs: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(dir) = std::fs::read_dir(docs.join("research")) else {
        return out;
    };
    let mut files: Vec<_> = dir.flatten().map(|e| e.path()).collect();
    files.sort();
    for path in files {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        if name == "README" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut title = String::new();
        for line in text.lines() {
            let t = line.trim().trim_start_matches("- ");
            if let Some(bold) = t.strip_prefix("**").and_then(|b| b.split("**").next())
                && is_hypothesis_id(bold.split_whitespace().next().unwrap_or(""))
            {
                title = bold.to_string();
            }
            if t.contains("Status:") && t.contains("`open`") && !title.is_empty() {
                out.push((name.clone(), title.clone()));
            }
        }
    }
    out
}

/// `H1.`, `G4.`, `H12.`: a hypothesis label in a research note.
fn is_hypothesis_id(w: &str) -> bool {
    let w = w.trim_end_matches('.');
    let letters = w.chars().take_while(|c| c.is_ascii_uppercase()).count();
    (1..=2).contains(&letters)
        && w.len() > letters
        && w[letters..].chars().all(|c| c.is_ascii_digit())
}

/// The gap list. With `cutoff`, only what the log and registry said up to
/// and including that experiment (research notes and staleness are not
/// time-sliced: they are left out of a cutoff list).
pub fn gaps(
    rows: &[Row],
    entries: &BTreeMap<u32, String>,
    notes: &[(String, String)],
    cutoff: Option<u32>,
) -> Vec<Gap> {
    let upto = |id: u32| cutoff.is_none_or(|c| id <= c);
    let by_id: BTreeMap<u32, &Row> = rows.iter().map(|r| (r.id, r)).collect();
    let mechs = |id: u32| {
        by_id
            .get(&id)
            .map(|r| r.mechanisms.clone())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    if cutoff.is_none() {
        for r in rows.iter().filter(|r| r.action() != "none") {
            out.push(Gap {
                label: format!("stale/E{:03}", r.id),
                source: Source::Stale,
                from: Some(r.id),
                mechanisms: r.mechanisms.clone(),
                text: format!(
                    "{} E{:03} {} ({}, measured under {}): {}",
                    r.action(),
                    r.id,
                    r.slug,
                    r.verdict,
                    r.objective,
                    r.note
                ),
            });
        }
        for (i, (note, title)) in notes.iter().enumerate() {
            out.push(Gap {
                label: format!("open-hypothesis/{note}/{i}"),
                source: Source::OpenHypothesis,
                from: None,
                mechanisms: vec![],
                text: format!("{note}: {title}"),
            });
        }
    }
    for (&id, entry) in entries.iter().filter(|(id, _)| upto(**id)) {
        for (n, line) in undone_lines(entry).into_iter().enumerate() {
            out.push(Gap {
                label: format!("not-measured/E{id:03}/{n}"),
                source: Source::NotMeasured,
                from: Some(id),
                mechanisms: mechs(id),
                text: line,
            });
        }
    }
    // Planned: mentioned up to the cutoff, but neither recorded nor run by then.
    let mut mentioned = BTreeSet::new();
    for (&id, entry) in entries.iter().filter(|(id, _)| upto(**id)) {
        let _ = id;
        mentioned.extend(mentions(entry));
        mentioned.extend(mentioned_ranges(entry));
    }
    let done: BTreeSet<u32> = (entries.keys().copied().filter(|&i| upto(i)))
        .chain(rows.iter().map(|r| r.id).filter(|&i| upto(i)))
        .collect();
    let latest = done.iter().next_back().copied().unwrap_or(0);
    for id in mentioned {
        // Only ids near the newest experiment: an old id with no record is
        // a gap in numbering, not a plan.
        if done.contains(&id) || id <= latest.saturating_sub(PLANNED_WINDOW) {
            continue;
        }
        if cutoff.is_none() && (entries.contains_key(&id) || by_id.contains_key(&id)) {
            continue;
        }
        out.push(Gap {
            label: format!("planned/E{id:03}"),
            source: Source::Planned,
            from: Some(id),
            mechanisms: vec![],
            text: format!("E{id:03} is mentioned in the log but has no entry or registry row"),
        });
    }
    out
}

/// Content words of a text (lowercase, 4+ letters, minus a few stop words).
pub fn words(text: &str) -> BTreeSet<String> {
    const STOP: [&str; 24] = [
        "with", "that", "this", "from", "than", "they", "them", "their", "then", "there", "here",
        "were", "have", "each", "into", "only", "when", "what", "which", "would", "measured",
        "built", "tried", "against",
    ];
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.len() >= 4 && !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !STOP.contains(&w.as_str()))
        .collect()
}

/// Minimal JSON string escaping for the examples' output lines.
pub fn json_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Experiment ids named inside an entry's `(Source: ...)` parentheticals
/// (E290): the entries it says it builds on.
pub fn source_ids(entry: &str) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    for part in entry.split("(Source:").skip(1) {
        let inner = part.split(')').next().unwrap_or("");
        out.extend(mentions(inner));
    }
    out
}

/// The Question and Hypothesis lines of an entry.
fn ask_lines(entry: &str) -> String {
    entry
        .lines()
        .filter(|l| l.starts_with("- Question:") || l.starts_with("- Hypothesis:"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Experiment ids an entry's Question or Hypothesis names as the source of
/// a gap it takes up: `E123's "Not measured"` (or "Not built", "Not
/// tried"), the form the loop's entries use (E290).
pub fn gap_citations(entry: &str) -> BTreeSet<u32> {
    let asks = ask_lines(entry);
    let mut out = BTreeSet::new();
    for (i, _) in asks.match_indices("'s \"Not ") {
        let head = &asks[..i];
        let start = head.rfind('E').unwrap_or(i);
        if let Some(id) = parse_id(&head[start..]) {
            out.insert(id);
        }
    }
    out
}

/// Not-measured gaps that a later entry follows up (E290). `strict` takes
/// only [`gap_citations`]; otherwise any id in the entry's `(Source: ...)`.
/// Of the cited experiment's not-measured lines, the one sharing the most
/// content words (at least one) with the later entry's Question and
/// Hypothesis is taken as the one followed. Gap label to the following
/// entry.
pub fn followups(
    gaps: &[Gap],
    entries: &BTreeMap<u32, String>,
    strict: bool,
) -> BTreeMap<String, u32> {
    let mut out = BTreeMap::new();
    for (&later, entry) in entries {
        let asks = words(&ask_lines(entry));
        let cited = if strict {
            gap_citations(entry)
        } else {
            source_ids(entry)
        };
        for cited in cited.into_iter().filter(|&c| c < later) {
            let best = gaps
                .iter()
                .filter(|g| g.source == Source::NotMeasured && g.from == Some(cited))
                .map(|g| (words(&g.text).intersection(&asks).count(), g))
                .filter(|(n, _)| *n > 0)
                .max_by_key(|(n, g)| (*n, std::cmp::Reverse(g.label.clone())));
            if let Some((_, g)) = best {
                out.entry(g.label.clone()).or_insert(later);
            }
        }
    }
    out
}
