//! Semantic definitions as versioned artifacts (E419, opt-in, library only).
//!
//! A semantic model (datasets, their fields, relationships, metrics, and an
//! ontology's concepts and mappings, as Apache Ossie writes them in YAML) is
//! kept as content beside the tables it describes:
//!
//! - **Fragments.** Each document of a definition set is cut into byte
//!   spans, one per definition: a dataset's own lines, each of its fields,
//!   each relationship, metric, concept and concept mapping, each other
//!   top-level key. Each span is a file under `semantic/<set>/<doc>/`
//!   (`datasets/<name>/_`, `datasets/<name>/fields/<field>`,
//!   `metrics/<name>`, `_model/<key>` ...), and `_order` lists them in the
//!   document's order. Comments and blank lines go with the definition
//!   they precede. The spans cover the document exactly, so an export
//!   (the files of `_order`, joined) is the imported bytes, and an edit of
//!   one metric is a change of that one file.
//! - **Links.** `semantic/<set>/_links` is a graph with a node per
//!   fragment (`f/<doc>/<path>`), per dataset (`d/<name>`) and per column
//!   (`c/<dataset>/<column>`); a fragment has a `names` edge to each column
//!   it names (a field its own; a dataset its keys; a relationship its
//!   columns; a metric or mapping the `dataset.column` names in its `sql`
//!   and `expression` values) and an `of` edge to its dataset. It is
//!   rebuilt from the fragments on every write, so text-only edits leave
//!   it unchanged.
//! - **Certification** is a `human` transition of the `certify` net, fired
//!   only with an [`Approval`] sealed by the owner's key (which a person's
//!   surface holds, not a scheduler). Its tokens are fragment content ids,
//!   so a certified definition that is edited is a new, uncertified
//!   version, and the old one stays certified at the commits that hold it.
//! - **Merging in** a definition set from another project is the scoped
//!   share merge (E365, [`import_share`]) behind the `authorize` human
//!   transition of the receiver's `merge-in` net: a share offered by an
//!   agent is merged only once a person has authorized that exact share
//!   (its scope and commit); the merge consumes the authorization.
//!
//! The YAML is not parsed into values: only line indents, `key:` prefixes
//! and `- ` items are read, which is enough to cut block-style documents
//! (all of Ossie's examples). A flow-style or otherwise unusual document
//! still round-trips exactly; it is just cut into fewer fragments.
//!
//! Nothing in the default paths calls this module.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::bm25::Bm25Mem;
use crate::merge::Merged;
use crate::net::{self, Firing, Marking, Net, Place, Transition};
use crate::share::{Refused, Right, Share, import_share};
use crate::{Db, Edge, Edit, Error, Id, Kind, Node, Object, Repo, Store, Target, node_key};

/// The directory of definition sets in a commit's tree.
pub const DEFS_DIR: &str = "semantic";
/// The file listing a document's fragments in order.
pub const ORDER: &str = "_order";
/// The links graph of a definition set.
pub const LINKS: &str = "_links";
/// The ref holding a project's `merge-in` net and its marking.
pub const MERGE_REF: &str = "merge-in";

/// Top-level keys whose sequence items become one fragment each.
const SPLIT: &[&str] = &[
    "datasets",
    "relationships",
    "metrics",
    "ontology",
    "concept_mappings",
];
/// Keys that name an item, in order of preference.
const NAME_KEYS: &[&str] = &["name", "concept", "vendor_name"];
/// Fragment names this module uses itself.
const RESERVED: &[&str] = &["_", "_rest", "_order", "_links", "_model"];

/// One fragment of a document: its path in the document's directory and
/// its byte range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub path: String,
    pub start: usize,
    pub end: usize,
}

struct Line {
    start: usize,
    end: usize,
    indent: usize,
    /// Blank or a comment.
    trivia: bool,
    blank: bool,
}

fn lines(b: &[u8]) -> Vec<Line> {
    let mut out = Vec::new();
    let mut s = 0;
    while s < b.len() {
        let e = memchr::memchr(b'\n', &b[s..]).map_or(b.len(), |i| s + i + 1);
        let text = &b[s..e];
        let indent = text.iter().take_while(|&&c| c == b' ').count();
        let rest = text[indent..].trim_ascii();
        out.push(Line {
            start: s,
            end: e,
            indent,
            trivia: rest.is_empty() || rest[0] == b'#',
            blank: rest.is_empty(),
        });
        s = e;
    }
    out
}

/// `key: value` at the start of `text` (already past its indent): the key
/// (quotes stripped) and what follows the colon.
fn key_of(text: &[u8]) -> Option<(String, &[u8])> {
    let text = text.trim_ascii_end();
    if text.is_empty() || matches!(text[0], b'-' | b'#' | b'[' | b'{' | b'?' | b'&' | b'*') {
        return None;
    }
    let (k, rest) = if matches!(text[0], b'"' | b'\'') {
        let q = text[0];
        let end = 1 + text[1..].iter().position(|&c| c == q)?;
        let rest = text[end + 1..].strip_prefix(b":")?;
        (&text[1..end], rest)
    } else {
        let mut i = 0;
        loop {
            let c = *text.get(i)?;
            if c == b':' && text.get(i + 1).is_none_or(|n| *n == b' ') {
                break;
            }
            if c == b'#' && i > 0 && text[i - 1] == b' ' {
                return None;
            }
            i += 1;
        }
        (text[..i].trim_ascii_end(), &text[i + 1..])
    };
    if rest.first().is_some_and(|c| *c != b' ') {
        return None;
    }
    Some((String::from_utf8_lossy(k).into_owned(), rest))
}

/// A value with nothing on its line but maybe a comment.
fn empty_value(v: &[u8]) -> bool {
    let v = v.trim_ascii();
    v.is_empty() || v[0] == b'#'
}

/// A scalar value, its quotes and a trailing comment removed.
fn scalar(v: &[u8]) -> String {
    let v = v.trim_ascii();
    let v = match v.first() {
        Some(q @ (b'"' | b'\'')) => {
            let end = v[1..]
                .iter()
                .position(|c| c == q)
                .map_or(v.len(), |i| i + 1);
            &v[1..end.max(1)]
        }
        _ => match v.windows(2).position(|w| w == b" #") {
            Some(i) => v[..i].trim_ascii_end(),
            None => v,
        },
    };
    String::from_utf8_lossy(v).into_owned()
}

/// Whether the line (past its indent) is a sequence item.
fn is_dash(text: &[u8]) -> bool {
    text.first() == Some(&b'-')
        && text
            .get(1)
            .is_none_or(|c| matches!(c, b' ' | b'\n' | b'\r'))
}

/// What follows a sequence item's dash, and the indent of its content.
fn after_dash(text: &[u8], indent: usize) -> (&[u8], usize) {
    let rest = &text[1..];
    let sp = rest.iter().take_while(|&&c| c == b' ').count();
    (&rest[sp..], indent + 1 + sp)
}

#[derive(Clone, Copy)]
enum Split {
    Model(usize),
    Section(usize),
    Item(usize),
    Field(usize, usize),
    Rest(usize),
}

#[derive(Default)]
struct Item {
    key: String,
    names: Vec<(usize, String)>,
    fields: Vec<Vec<(usize, String)>>,
}

fn name_of(names: &[(usize, String)]) -> Option<String> {
    names
        .iter()
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, n)| n.clone())
}

fn note_name(names: &mut Vec<(usize, String)>, text: &[u8]) {
    if let Some((k, v)) = key_of(text)
        && let Some(rank) = NAME_KEYS.iter().position(|n| *n == k)
        && !empty_value(v)
    {
        names.push((rank, scalar(v)));
    }
}

/// A name usable as one path part, never one this module reserves.
fn sanitize(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '.' => c,
            _ => '_',
        })
        .collect();
    if s.is_empty() || s.starts_with('.') || s.starts_with('_') || RESERVED.contains(&s.as_str()) {
        s.insert(0, '~');
    }
    s.truncate(96);
    s
}

fn unique(used: &mut HashSet<String>, base: String) -> String {
    if used.insert(base.clone()) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}~{n}"))
        .find(|p| used.insert(p.clone()))
        .unwrap()
}

/// Cut a document into fragments. The spans are contiguous and cover
/// every byte, in order, whatever the input.
pub fn segment(bytes: &[u8]) -> Vec<Span> {
    let ls = lines(bytes);
    let text = |i: usize| &bytes[ls[i].start + ls[i].indent..ls[i].end];
    let mut splits: Vec<(usize, Split)> = Vec::new();
    let mut models: Vec<String> = Vec::new();
    let mut sections: Vec<String> = Vec::new();
    let mut items: Vec<Item> = Vec::new();
    // The open split section: its key, its sequence's indent once seen.
    let mut section: Option<(String, Option<usize>)> = None;
    // The open item: its index and its content's indent.
    let mut item: Option<(usize, usize)> = None;
    // A dataset's fields: None, Some(None) awaiting the first, Some(indent).
    let mut fields: Option<Option<usize>> = None;
    let mut field_content = 0;
    let mut rest_started = false;
    for (i, line) in ls.iter().enumerate() {
        if line.trivia {
            continue;
        }
        let (ind, t) = (line.indent, text(i));
        let in_seq0 = matches!(&section, Some((_, Some(0) | None)));
        if ind == 0 && !(in_seq0 && is_dash(t)) {
            (item, fields, rest_started) = (None, None, false);
            section = None;
            match key_of(t) {
                Some((k, v)) if SPLIT.contains(&k.as_str()) && empty_value(v) => {
                    splits.push((i, Split::Section(sections.len())));
                    sections.push(k.clone());
                    section = Some((k, None));
                }
                Some((k, _)) => {
                    splits.push((i, Split::Model(models.len())));
                    models.push(k);
                }
                None => {}
            }
            continue;
        }
        let Some((key, seq)) = &mut section else {
            continue;
        };
        let si = match *seq {
            Some(si) => si,
            None if is_dash(t) => {
                *seq = Some(ind);
                ind
            }
            None => {
                section = None;
                continue;
            }
        };
        if ind == si && is_dash(t) {
            let (after, content) = after_dash(t, ind);
            let mut it = Item {
                key: key.clone(),
                ..Item::default()
            };
            note_name(&mut it.names, after);
            (fields, rest_started) = (None, false);
            if key.as_str() == "datasets"
                && let Some((k, v)) = key_of(after)
                && k == "fields"
                && empty_value(v)
            {
                fields = Some(None);
            }
            splits.push((i, Split::Item(items.len())));
            item = Some((items.len(), content));
            items.push(it);
            continue;
        }
        let Some((n, content)) = item else {
            continue;
        };
        if items[n].key == "datasets" {
            if fields == Some(None) {
                fields = if is_dash(t) && ind >= content {
                    Some(Some(ind))
                } else {
                    None
                };
            }
            if let Some(Some(fi)) = fields {
                if ind == fi && is_dash(t) {
                    let (after, fc) = after_dash(t, ind);
                    field_content = fc;
                    let mut names = Vec::new();
                    note_name(&mut names, after);
                    splits.push((i, Split::Field(n, items[n].fields.len())));
                    items[n].fields.push(names);
                    continue;
                }
                if ind > fi {
                    if ind == field_content
                        && let Some(f) = items[n].fields.last_mut()
                    {
                        note_name(f, t);
                    }
                    continue;
                }
                fields = None;
                if !rest_started {
                    splits.push((i, Split::Rest(n)));
                    rest_started = true;
                }
            }
            if ind == content {
                note_name(&mut items[n].names, t);
                if !rest_started
                    && let Some((k, v)) = key_of(t)
                    && k == "fields"
                    && empty_value(v)
                {
                    fields = Some(None);
                }
            }
        } else if ind == content {
            note_name(&mut items[n].names, t);
        }
    }
    // Paths.
    let mut used = HashSet::new();
    let mut base: Vec<String> = Vec::with_capacity(items.len());
    for it in &items {
        let name = name_of(&it.names).unwrap_or_default();
        base.push(unique(&mut used, format!("{}/{}", it.key, sanitize(&name))));
    }
    let mut field_used: HashMap<usize, HashSet<String>> = HashMap::new();
    let mut paths = Vec::with_capacity(splits.len());
    for (_, s) in &splits {
        let p = match *s {
            Split::Model(m) => unique(&mut used, format!("_model/{}", sanitize(&models[m]))),
            Split::Section(s) => unique(&mut used, format!("{}/_", sections[s])),
            Split::Item(n) if items[n].key == "datasets" => format!("{}/_", base[n]),
            Split::Item(n) => base[n].clone(),
            Split::Field(n, f) => {
                let name = name_of(&items[n].fields[f]).unwrap_or_default();
                let u = field_used.entry(n).or_default();
                format!("{}/fields/{}", base[n], unique(u, sanitize(&name)))
            }
            Split::Rest(n) => format!("{}/_rest", base[n]),
        };
        paths.push(p);
    }
    if splits.is_empty() {
        return vec![Span {
            path: "_model/_doc".into(),
            start: 0,
            end: bytes.len(),
        }];
    }
    // Each split takes the comments and blank lines just above it, up to
    // a comment indented deeper than itself (which stays with the one
    // before).
    let mut starts = Vec::with_capacity(splits.len());
    let mut floor = 0;
    for (k, (l, _)) in splits.iter().enumerate() {
        let mut s = *l;
        while s > floor
            && ls[s - 1].trivia
            && (ls[s - 1].blank || ls[s - 1].indent <= ls[*l].indent)
        {
            s -= 1;
        }
        starts.push(if k == 0 { 0 } else { ls[s].start });
        floor = l + 1;
    }
    (0..splits.len())
        .map(|k| Span {
            path: paths[k].clone(),
            start: starts[k],
            end: starts.get(k + 1).copied().unwrap_or(bytes.len()),
        })
        .collect()
}

/// What a fragment's path says it defines.
pub fn kind_of(path: &str) -> &'static str {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["_model", ..] => "model",
        [_, "_"] => "section",
        ["datasets", _, "fields", _] => "field",
        ["datasets", ..] => "dataset",
        ["relationships", _] => "relationship",
        ["metrics", _] => "metric",
        ["ontology", _] => "concept",
        ["concept_mappings", _] => "mapping",
        _ => "other",
    }
}

/// The name a fragment's first item gives itself (`name`, `concept` or
/// `vendor_name` at the item's own indent).
pub fn item_name(bytes: &[u8]) -> Option<String> {
    let ls = lines(bytes);
    let mut content = None;
    let mut names = Vec::new();
    for l in ls.iter().filter(|l| !l.trivia) {
        let t = &bytes[l.start + l.indent..l.end];
        match content {
            None if is_dash(t) => {
                let (after, c) = after_dash(t, l.indent);
                content = Some(c);
                note_name(&mut names, after);
            }
            None => return None,
            Some(c) if l.indent == c => note_name(&mut names, t),
            Some(c) if l.indent < c => break,
            _ => {}
        }
    }
    name_of(&names)
}

fn idents(s: &str) -> Vec<&str> {
    s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty() && !w.as_bytes()[0].is_ascii_digit())
        .collect()
}

/// `dataset.column` names in `s` whose dataset is in `known`.
fn qualified<'a>(s: &'a str, known: &BTreeSet<String>) -> Vec<(&'a str, &'a str)> {
    let b = s.as_bytes();
    let word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !word(b[i]) || (i > 0 && (word(b[i - 1]) || b[i - 1] == b'.')) {
            i += 1;
            continue;
        }
        let a0 = i;
        while i < b.len() && word(b[i]) {
            i += 1;
        }
        if i + 1 < b.len() && b[i] == b'.' && word(b[i + 1]) {
            let c0 = i + 1;
            let mut j = c0;
            while j < b.len() && word(b[j]) {
                j += 1;
            }
            let (ds, col) = (&s[a0..i], &s[c0..j]);
            if known.contains(ds) && !b[a0].is_ascii_digit() {
                out.push((ds, col));
            }
            i = j;
        }
    }
    out
}

/// The columns a fragment names, as `(dataset, column)`, given the
/// fragment's dataset (for a field or a dataset's own lines) and the
/// datasets the set defines.
pub fn columns_named(
    path: &str,
    bytes: &[u8],
    dataset: Option<&str>,
    known: &BTreeSet<String>,
) -> BTreeSet<(String, String)> {
    let text = String::from_utf8_lossy(bytes);
    let mut out = BTreeSet::new();
    let own = |out: &mut BTreeSet<(String, String)>, col: &str| {
        if let Some(d) = dataset {
            out.insert((d.to_string(), col.to_string()));
        }
    };
    match kind_of(path) {
        "field" => {
            if let Some(n) = item_name(bytes) {
                own(&mut out, &n);
            }
        }
        "dataset" => {
            let mut in_keys = None;
            for line in text.lines() {
                let ind = line.len() - line.trim_start().len();
                let t = line.trim();
                if in_keys.is_some_and(|k| ind <= k && !t.starts_with('-')) {
                    in_keys = None;
                }
                if let Some((k, v)) = key_of(t.as_bytes()) {
                    if k == "primary_key" || k == "unique_keys" {
                        in_keys = Some(ind);
                        idents(&scalar(v)).iter().for_each(|c| own(&mut out, c));
                    }
                } else if in_keys.is_some() && t.starts_with('-') {
                    idents(&scalar(&t.as_bytes()[1..]))
                        .iter()
                        .for_each(|c| own(&mut out, c));
                }
            }
        }
        "relationship" => {
            let mut vals: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let mut list: Option<(String, usize)> = None;
            for line in text.lines() {
                let ind = line.len() - line.trim_start().len();
                let trimmed = line.trim();
                if let Some((k, li)) = &list
                    && trimmed.starts_with('-')
                    && ind >= *li
                {
                    let v = scalar(&trimmed.as_bytes()[1..]);
                    vals.entry(k.clone()).or_default().push(v);
                    continue;
                }
                list = None;
                let t = trimmed.strip_prefix("- ").unwrap_or(trimmed);
                if let Some((k, v)) = key_of(t.as_bytes()) {
                    if empty_value(v) {
                        list = Some((k, ind));
                    } else {
                        let v = scalar(v);
                        let v = v.trim_matches(|c| c == '[' || c == ']');
                        vals.entry(k)
                            .or_default()
                            .extend(v.split(',').map(|s| scalar(s.as_bytes())));
                    }
                }
            }
            for side in ["from", "to"] {
                let ds = vals.get(side).and_then(|v| v.first()).cloned();
                if let Some(ds) = ds {
                    for c in vals.get(&format!("{side}_columns")).into_iter().flatten() {
                        if !c.is_empty() {
                            out.insert((ds.clone(), c.clone()));
                        }
                    }
                }
            }
        }
        _ => {
            for line in text.lines() {
                let t = line.trim().trim_start_matches("- ");
                if let Some((k, v)) = key_of(t.as_bytes())
                    && (k == "sql" || k == "expression")
                {
                    let v = String::from_utf8_lossy(v);
                    for (d, c) in qualified(&v, known) {
                        out.insert((d.to_string(), c.to_string()));
                    }
                }
            }
        }
    }
    out
}

/// One stored definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Def {
    pub doc: String,
    pub path: String,
    pub id: Id,
}

impl Def {
    pub fn key(&self) -> String {
        format!("{}/{}", self.doc, self.path)
    }
}

/// Where a set's files live in a commit's tree.
pub fn set_dir(set: &str) -> String {
    format!("{DEFS_DIR}/{set}")
}

/// A deterministic bag of hashed character trigrams, unit length: a
/// stand-in embedder for runs without model weights (not a learned
/// model; it finds shared spellings, not meanings).
pub fn trigram_vector(text: &str, dims: usize) -> Vec<f32> {
    let dims = dims.max(1);
    let mut v = vec![0f32; dims];
    for w in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
    {
        let w = format!(" {} ", w.to_lowercase());
        let cs: Vec<char> = w.chars().collect();
        for g in cs.windows(3) {
            let s: String = g.iter().collect();
            let h = blake3::hash(s.as_bytes());
            let x = u64::from_le_bytes(h.as_bytes()[..8].try_into().unwrap());
            v[(x % dims as u64) as usize] += if x >> 63 == 0 { 1.0 } else { -1.0 };
        }
    }
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

/// A text embedder for [`Repo::defs_search`].
pub type Embed = Box<dyn Fn(&str) -> Vec<f32>>;

/// A fragment's key, its dataset and the `(dataset, column)` pairs it names.
type FragmentLinks = (String, Option<String>, BTreeSet<(String, String)>);

/// Keyword, vector and fused search over a set's definitions at one
/// commit. Keys are `<doc>/<path>`.
pub struct DefsSearch {
    bm25: Bm25Mem,
    vecs: Vec<(String, Vec<f32>)>,
    embed: Embed,
}

impl DefsSearch {
    pub fn keyword(&self, q: &str, k: usize) -> Vec<(String, f32)> {
        self.bm25.search(q, k)
    }

    pub fn vector(&self, q: &str, k: usize) -> Vec<(String, f32)> {
        let qv = (self.embed)(q);
        let mut out: Vec<(String, f32)> = self
            .vecs
            .iter()
            .map(|(key, v)| (key.clone(), v.iter().zip(&qv).map(|(a, b)| a * b).sum()))
            .collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out.truncate(k);
        out
    }

    /// The fused list ([`crate::fusion::convex`]) of the two lists, each
    /// cut at `depth`.
    pub fn hybrid(&self, q: &str, k: usize, depth: usize, alpha: f32) -> Vec<(String, f32)> {
        let mut out = crate::fusion::convex(&self.keyword(q, depth), &self.vector(q, depth), alpha);
        out.truncate(k);
        out
    }

    pub fn len(&self) -> usize {
        self.vecs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vecs.is_empty()
    }
}

/// A person's approval of one firing of a `human` transition, sealed with
/// the owner's key by the surface the person uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Approval {
    pub person: String,
    pub net: String,
    pub transition: String,
    pub token: String,
}

const APPROVAL_TAG: &[u8; 16] = b"ikam:approval v1";

impl Approval {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = APPROVAL_TAG.to_vec();
        for s in [&self.person, &self.net, &self.transition, &self.token] {
            out.extend_from_slice(&(s.len() as u64).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        out
    }

    pub fn seal(&self, key: &[u8; 32]) -> [u8; 32] {
        *blake3::keyed_hash(key, &self.encode()).as_bytes()
    }
}

/// The certification net: an agent submits a definition version for
/// review; only a person certifies or rejects it.
pub fn certify_net() -> Net {
    let place = |name: &str| Place {
        name: name.into(),
        colored: true,
        initial: 0,
    };
    let arc = |p: &str| net::Arc {
        place: p.into(),
        weight: 1,
    };
    let tr = |name: &str, i: &[&str], o: &[&str], kind| Transition {
        name: name.into(),
        inputs: i.iter().map(|p| arc(p)).collect(),
        outputs: o.iter().map(|p| arc(p)).collect(),
        kind,
        capability: None,
    };
    Net {
        name: "certify".into(),
        version: 1,
        places: vec![place("review"), place("certified")],
        transitions: vec![
            tr("submit", &[], &["review"], net::Kind::Agent),
            tr("certify", &["review"], &["certified"], net::Kind::Human),
            tr("reject", &["review"], &[], net::Kind::Human),
        ],
    }
}

/// The receiver's net for merges from other projects: an agent offers a
/// share; a person authorizes it; the merge consumes the authorization.
pub fn merge_net() -> Net {
    let arc = |p: &str| net::Arc {
        place: p.into(),
        weight: 1,
    };
    let tr = |name: &str, i: &str, o: &str, kind| Transition {
        name: name.into(),
        inputs: (!i.is_empty()).then(|| arc(i)).into_iter().collect(),
        outputs: vec![arc(o)],
        kind,
        capability: None,
    };
    Net {
        name: "merge-in".into(),
        version: 1,
        places: ["offered", "authorized", "merged"]
            .iter()
            .map(|p| Place {
                name: (*p).into(),
                colored: true,
                initial: 0,
            })
            .collect(),
        transitions: vec![
            tr("offer", "", "offered", net::Kind::Agent),
            tr("authorize", "offered", "authorized", net::Kind::Human),
            tr("merge", "authorized", "merged", net::Kind::Pure),
        ],
    }
}

/// The token naming one share (its scope, commit, right and expiry).
pub fn share_token(share: &Share) -> String {
    blake3::hash(&share.encode()).to_hex().to_string()
}

/// Why a merge in was refused.
#[derive(Debug, PartialEq, Eq)]
pub enum MergeRefused {
    /// No person has authorized this share (or its authorization was used).
    NotAuthorized,
    Share(Refused),
}

impl From<Refused> for MergeRefused {
    fn from(r: Refused) -> Self {
        MergeRefused::Share(r)
    }
}

impl From<Error> for MergeRefused {
    fn from(e: Error) -> Self {
        MergeRefused::Share(Refused::Store(e.to_string()))
    }
}

fn cert_ref(set: &str) -> String {
    format!("certify/{set}")
}

fn fire_key(transition: &str, token: &str) -> String {
    format!("{transition}.{}", &token[..token.len().min(100)])
}

impl<S: Store> Repo<S> {
    fn entry_at(&self, root: Id, path: &str) -> Result<Option<(Kind, Id)>, Error> {
        let (mut kind, mut id) = (Kind::Tree, root);
        for part in path.split('/').filter(|p| !p.is_empty()) {
            let Object::Tree(entries) = self.get(&id)? else {
                return Ok(None);
            };
            match entries.into_iter().find(|e| e.name == part) {
                Some(e) => (kind, id) = (e.kind, e.id),
                None => return Ok(None),
            }
        }
        Ok(Some((kind, id)))
    }

    fn ref_tree(&self, refname: &str) -> Result<Option<Id>, Error> {
        match self.store.get_ref(refname)? {
            Some(c) => Ok(Some(self.commit_tree(&c)?)),
            None => Ok(None),
        }
    }

    /// Import Ossie documents (`(file name, bytes)`) as definition set `set`
    /// on `refname`, replacing an earlier import of the set, and commit.
    pub fn defs_import(
        &mut self,
        refname: &str,
        set: &str,
        docs: &[(&str, &[u8])],
        message: &str,
    ) -> Result<Id, Error> {
        let dir = set_dir(set);
        let root = self.ref_tree(refname)?;
        let mut edits = Vec::new();
        if let Some(r) = root
            && self.entry_at(r, &dir)?.is_some()
        {
            edits.push(Edit::Remove { path: dir.clone() });
        }
        for (name, bytes) in docs {
            let spans = segment(bytes);
            let joined: Vec<u8> = spans
                .iter()
                .flat_map(|s| bytes[s.start..s.end].iter().copied())
                .collect();
            if joined != *bytes {
                return Err(Error::Decode(
                    "semantic fragments do not cover the document",
                ));
            }
            let mut order = String::new();
            for s in &spans {
                let id = self.put_content(&bytes[s.start..s.end])?.id;
                edits.push(Edit::Put {
                    path: format!("{dir}/{name}/{}", s.path),
                    id,
                });
                order.push_str(&s.path);
                order.push('\n');
            }
            let id = self.put_content(order.as_bytes())?.id;
            edits.push(Edit::Put {
                path: format!("{dir}/{name}/{ORDER}"),
                id,
            });
        }
        let tree = self.edit_tree(root, &edits)?;
        let tree = self.defs_relink_tree(tree, set)?;
        self.commit(refname, tree, message)
    }

    /// The documents of `set` at `commit`.
    pub fn defs_docs(&self, commit: &Id, set: &str) -> Result<Vec<String>, Error> {
        let root = self.commit_tree(commit)?;
        let Some((Kind::Tree, d)) = self.entry_at(root, &set_dir(set))? else {
            return Ok(Vec::new());
        };
        let Object::Tree(es) = self.get(&d)? else {
            return Ok(Vec::new());
        };
        Ok(es
            .into_iter()
            .filter(|e| e.kind == Kind::Tree)
            .map(|e| e.name)
            .collect())
    }

    fn defs_in_tree(&self, root: Id, set: &str) -> Result<Vec<Def>, Error> {
        let mut out = Vec::new();
        let dir = set_dir(set);
        let Some((Kind::Tree, d)) = self.entry_at(root, &dir)? else {
            return Ok(out);
        };
        let Object::Tree(docs) = self.get(&d)? else {
            return Ok(out);
        };
        for doc in docs.into_iter().filter(|e| e.kind == Kind::Tree) {
            let Some((Kind::File, o)) = self.entry_at(doc.id, ORDER)? else {
                return Err(Error::Decode("semantic document without _order"));
            };
            let order = String::from_utf8(self.read_content(&o)?)
                .map_err(|_| Error::Decode("semantic _order"))?;
            for path in order.lines() {
                match self.entry_at(doc.id, path)? {
                    Some((Kind::File, id)) => out.push(Def {
                        doc: doc.name.clone(),
                        path: path.to_string(),
                        id,
                    }),
                    _ => return Err(Error::Decode("semantic fragment missing")),
                }
            }
        }
        Ok(out)
    }

    /// Every definition of `set` at `commit`, document by document in
    /// order.
    pub fn defs_list(&self, commit: &Id, set: &str) -> Result<Vec<Def>, Error> {
        self.defs_in_tree(self.commit_tree(commit)?, set)
    }

    /// Document `doc` of `set` at `commit`, as Ossie YAML: its fragments
    /// joined in order.
    pub fn defs_export(&self, commit: &Id, set: &str, doc: &str) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        for d in self
            .defs_list(commit, set)?
            .into_iter()
            .filter(|d| d.doc == doc)
        {
            out.extend(self.read_content(&d.id)?);
        }
        Ok(out)
    }

    /// The links of `set` as the fragments name them in tree `root`: each
    /// fragment key with its dataset and its columns.
    fn defs_links_of(&self, root: Id, set: &str) -> Result<Vec<FragmentLinks>, Error> {
        let defs = self.defs_in_tree(root, set)?;
        let mut bytes = Vec::with_capacity(defs.len());
        for d in &defs {
            bytes.push(self.read_content(&d.id)?);
        }
        // Dataset names, from each dataset's own lines.
        let mut ds_of: HashMap<(String, String), String> = HashMap::new();
        let mut known = BTreeSet::new();
        for (d, b) in defs.iter().zip(&bytes) {
            if d.path.starts_with("datasets/")
                && d.path.ends_with("/_")
                && let Some(n) = item_name(b)
            {
                let dir = d.path.trim_end_matches("/_").to_string();
                ds_of.insert((d.doc.clone(), dir), n.clone());
                known.insert(n);
            }
        }
        let mut out = Vec::with_capacity(defs.len());
        for (d, b) in defs.iter().zip(&bytes) {
            let ds = d
                .path
                .strip_prefix("datasets/")
                .and_then(|r| r.split('/').next())
                .and_then(|n| ds_of.get(&(d.doc.clone(), format!("datasets/{n}"))))
                .cloned();
            let cols = columns_named(&d.path, b, ds.as_deref(), &known);
            out.push((d.key(), ds, cols));
        }
        Ok(out)
    }

    /// Tree `root` with `set`'s links graph rebuilt from its fragments.
    fn defs_relink_tree(&mut self, root: Id, set: &str) -> Result<Id, Error> {
        let links = self.defs_links_of(root, set)?;
        let mut nodes: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
        let edge = |label: &str, to: &str| (label.to_string(), to.to_string());
        for (key, ds, cols) in &links {
            let f = nodes.entry(format!("f/{key}")).or_default();
            if let Some(d) = ds {
                f.insert(edge("of", &format!("d/{d}")));
            }
            for (d, c) in cols {
                f.insert(edge("names", &format!("c/{d}/{c}")));
            }
            if let Some(d) = ds {
                nodes.entry(format!("d/{d}")).or_default();
            }
            for (d, c) in cols {
                nodes
                    .entry(format!("c/{d}/{c}"))
                    .or_default()
                    .insert(edge("in", &format!("d/{d}")));
                nodes.entry(format!("d/{d}")).or_default();
            }
        }
        let nodes: Vec<Node> = nodes
            .into_iter()
            .map(|(label, es)| Node {
                label,
                target: Target::None,
                edges: es
                    .into_iter()
                    .map(|(l, to)| Edge {
                        to: node_key(&to),
                        label: l,
                    })
                    .collect(),
            })
            .collect();
        let g = self.put_graph(nodes)?;
        let path = format!("{}/{LINKS}", set_dir(set));
        let mut parts: Vec<&str> = path.split('/').collect();
        let name = parts.pop().unwrap();
        self.put_at(root, &parts, name, Kind::Graph, g)
    }

    fn put_at(
        &mut self,
        root: Id,
        dirs: &[&str],
        name: &str,
        kind: Kind,
        id: Id,
    ) -> Result<Id, Error> {
        let mut entries = match self.get(&root)? {
            Object::Tree(es) => es,
            _ => return Err(Error::WrongKind(root)),
        };
        let (name, kind, id) = match dirs.split_first() {
            None => (name.to_string(), kind, id),
            Some((d, rest)) => {
                let sub = match entries.iter().find(|e| e.name == *d) {
                    Some(e) if e.kind == Kind::Tree => e.id,
                    _ => self.edit_tree(None, &[])?,
                };
                (
                    d.to_string(),
                    Kind::Tree,
                    self.put_at(sub, rest, name, kind, id)?,
                )
            }
        };
        entries.retain(|e| e.name != name);
        entries.push(crate::TreeEntry { name, kind, id });
        self.put(&Object::tree(entries)?)
    }

    /// Rebuild `set`'s links on `refname` (after a merge in, which copies
    /// files only) and commit if they changed; the head otherwise.
    pub fn defs_relink(&mut self, refname: &str, set: &str, message: &str) -> Result<Id, Error> {
        let head = self.resolve(refname)?;
        let root = self.commit_tree(&head)?;
        let tree = self.defs_relink_tree(root, set)?;
        if tree == root {
            return Ok(head);
        }
        self.commit(refname, tree, message)
    }

    /// The links graph of `set` at `commit`: each fragment key (`<doc>/
    /// <path>`) with the column labels (`<dataset>/<column>`) it names,
    /// sorted.
    pub fn defs_links(
        &self,
        commit: &Id,
        set: &str,
    ) -> Result<BTreeMap<String, Vec<String>>, Error> {
        let root = self.commit_tree(commit)?;
        let Some((Kind::Graph, g)) = self.entry_at(root, &format!("{}/{LINKS}", set_dir(set)))?
        else {
            return Ok(BTreeMap::new());
        };
        let nodes = self.graph_nodes(&g)?;
        let by_key: HashMap<[u8; 8], &str> = nodes
            .iter()
            .map(|n| (node_key(&n.label), n.label.as_str()))
            .collect();
        let mut out = BTreeMap::new();
        for n in &nodes {
            if let Some(f) = n.label.strip_prefix("f/") {
                let mut cols: Vec<String> = n
                    .edges
                    .iter()
                    .filter(|e| e.label == "names")
                    .filter_map(|e| by_key.get(&e.to)?.strip_prefix("c/"))
                    .map(String::from)
                    .collect();
                cols.sort();
                out.insert(f.to_string(), cols);
            }
        }
        Ok(out)
    }

    /// Replace fragment `path` of document `doc` with `bytes`, relink and
    /// commit on `refname`.
    pub fn defs_edit(
        &mut self,
        refname: &str,
        set: &str,
        doc: &str,
        path: &str,
        bytes: &[u8],
        message: &str,
    ) -> Result<Id, Error> {
        let head = self.resolve(refname)?;
        let root = self.commit_tree(&head)?;
        let full = format!("{}/{doc}/{path}", set_dir(set));
        if !matches!(self.entry_at(root, &full)?, Some((Kind::File, _))) {
            return Err(Error::InvalidName(full));
        }
        let id = self.put_content(bytes)?.id;
        let tree = self.edit_tree(Some(root), &[Edit::Put { path: full, id }])?;
        let tree = self.defs_relink_tree(tree, set)?;
        self.commit(refname, tree, message)
    }

    /// Paths (`<doc>/<path>`) whose fragment differs between two commits.
    pub fn defs_diff(&self, a: &Id, b: &Id, set: &str) -> Result<Vec<String>, Error> {
        let index = |c: &Id| -> Result<BTreeMap<String, Id>, Error> {
            Ok(self
                .defs_list(c, set)?
                .into_iter()
                .map(|d| (d.key(), d.id))
                .collect())
        };
        let (x, y) = (index(a)?, index(b)?);
        let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
        Ok(keys
            .into_iter()
            .filter(|k| x.get(*k) != y.get(*k))
            .cloned()
            .collect())
    }

    /// Keyword, vector and fused search over `set` at `commit`, with
    /// `embed` for the vectors (one call per fragment, here).
    pub fn defs_search(&self, commit: &Id, set: &str, embed: Embed) -> Result<DefsSearch, Error> {
        let mut bm25 = Bm25Mem::new();
        let mut vecs = Vec::new();
        for d in self.defs_list(commit, set)? {
            let text = String::from_utf8_lossy(&self.read_content(&d.id)?).into_owned();
            let key = d.key();
            bm25.put(&key, &text);
            vecs.push((key, embed(&text)));
        }
        Ok(DefsSearch { bm25, vecs, embed })
    }

    fn net_db(&mut self, refname: &str, net: &Net) -> Result<Db, Error> {
        let db = match self.store.get_ref(refname)? {
            Some(c) => self.db_at(&c)?,
            None => self.db_empty()?,
        };
        self.net_init(&db, net)
    }

    /// Fire a transition a scheduler or agent may fire; `human` ones are
    /// refused.
    fn fire_auto(
        &mut self,
        refname: &str,
        net: &Net,
        t: &str,
        token: &str,
    ) -> Result<Firing, Error> {
        let tr = net
            .transition(t)
            .ok_or_else(|| Error::InvalidName(t.into()))?;
        if tr.kind == net::Kind::Human {
            return Err(Error::Exec(format!(
                "transition {t} is human: only a person fires it"
            )));
        }
        let db = self.net_db(refname, net)?;
        let (db, f) = self.net_fire(&db, net, t, Some(token), &fire_key(t, token))?;
        self.db_commit(refname, &db, &format!("{} {t}", net.name))?;
        Ok(f)
    }

    /// Fire a `human` transition with a person's sealed approval of
    /// exactly this net, transition and token; the approval is the
    /// firing's recorded output.
    fn fire_human(
        &mut self,
        refname: &str,
        net: &Net,
        approval: &Approval,
        tag: &[u8; 32],
        key: &[u8; 32],
    ) -> Result<Firing, Error> {
        let t = approval.transition.as_str();
        let tr = net
            .transition(t)
            .ok_or_else(|| Error::InvalidName(t.into()))?;
        if tr.kind != net::Kind::Human || approval.net != net.name {
            return Err(Error::Exec(format!(
                "{t} is not a human transition of {}",
                net.name
            )));
        }
        if blake3::keyed_hash(key, &approval.encode()) != blake3::Hash::from(*tag) {
            return Err(Error::Exec("approval seal does not check".into()));
        }
        let db = self.net_db(refname, net)?;
        let (db, f) = self.net_fire_output(
            &db,
            net,
            t,
            Some(&approval.token),
            &fire_key(t, &approval.token),
            &approval.encode(),
        )?;
        self.db_commit(
            refname,
            &db,
            &format!("{} {t} by {}", net.name, approval.person),
        )?;
        Ok(f)
    }

    fn marking_of(&mut self, refname: &str, net: &Net) -> Result<Marking, Error> {
        let db = self.net_db(refname, net)?;
        self.net_marking(&db, &net.name)
    }

    /// Submit definition version `id` of `set` for review (an agent may).
    pub fn defs_submit(&mut self, set: &str, id: &Id) -> Result<Firing, Error> {
        self.fire_auto(&cert_ref(set), &certify_net(), "submit", &id.to_hex())
    }

    /// Fire `certify` or `reject` on a submitted version, with a person's
    /// approval (`approval.net` is `certify`, its token the version's id).
    pub fn defs_review(
        &mut self,
        set: &str,
        approval: &Approval,
        tag: &[u8; 32],
        key: &[u8; 32],
    ) -> Result<Firing, Error> {
        self.fire_human(&cert_ref(set), &certify_net(), approval, tag, key)
    }

    /// Try to fire any `certify` transition as a scheduler would: always
    /// refused (a check that certification cannot be automated).
    pub fn defs_certify_auto(&mut self, set: &str, id: &Id) -> Result<Firing, Error> {
        self.fire_auto(&cert_ref(set), &certify_net(), "certify", &id.to_hex())
    }

    /// Each definition of `set` at `commit` with whether that version is
    /// certified.
    pub fn defs_certified(&mut self, commit: &Id, set: &str) -> Result<Vec<(Def, bool)>, Error> {
        let m = self.marking_of(&cert_ref(set), &certify_net())?;
        let done = m.get("certified").cloned().unwrap_or_default();
        Ok(self
            .defs_list(commit, set)?
            .into_iter()
            .map(|d| {
                let c = done.contains_key(&d.id.to_hex());
                (d, c)
            })
            .collect())
    }

    /// The certification net's database, for replay checks.
    pub fn defs_cert_db(&mut self, set: &str) -> Result<Db, Error> {
        self.net_db(&cert_ref(set), &certify_net())
    }

    /// Record that `share` (from another project, to merge into this one)
    /// is offered; an agent may. Copies no bytes.
    pub fn defs_offer(&mut self, share: &Share) -> Result<String, Error> {
        let token = share_token(share);
        self.fire_auto(MERGE_REF, &merge_net(), "offer", &token)?;
        Ok(token)
    }

    /// A person authorizes the offered share whose token `approval` names.
    pub fn defs_authorize(
        &mut self,
        approval: &Approval,
        tag: &[u8; 32],
        key: &[u8; 32],
    ) -> Result<Firing, Error> {
        self.fire_human(MERGE_REF, &merge_net(), approval, tag, key)
    }

    /// Merge `share` (issued by project `src` to this project `me`) into
    /// `into`, only if a person authorized that exact share and it has not
    /// been merged on that authorization yet. Nothing is read from `src`
    /// before the authorization checks. The merge is E365's
    /// [`import_share`]; a successful one consumes the authorization.
    #[allow(clippy::too_many_arguments)]
    pub fn defs_merge_in<T: Store>(
        &mut self,
        me: &str,
        src: &mut Repo<T>,
        share: &Share,
        tag: &[u8; 32],
        src_key: &[u8; 32],
        into: &str,
        message: &str,
        now: u64,
    ) -> Result<Merged, MergeRefused> {
        let net = merge_net();
        let token = share_token(share);
        let m = self.marking_of(MERGE_REF, &net)?;
        if !m.get("authorized").is_some_and(|t| t.contains_key(&token)) {
            return Err(MergeRefused::NotAuthorized);
        }
        src.share_check(share, tag, src_key, me, Right::Merge, now)?;
        let merged = import_share(src, share, self, into, message)?;
        if !matches!(merged, Merged::Conflicts(_)) {
            self.fire_auto(MERGE_REF, &net, "merge", &token)?;
        }
        Ok(merged)
    }

    /// The `merge-in` net's database, for replay checks.
    pub fn defs_merge_db(&mut self) -> Result<Db, Error> {
        self.net_db(MERGE_REF, &merge_net())
    }
}
