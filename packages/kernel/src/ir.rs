//! Intermediate representations for compression (E192, opt-in).
//!
//! Nothing in the default put or repack path calls this module: it is an
//! option an application (or a later experiment) chooses, and the fixed
//! bench is unchanged with it present.
//!
//! A text file is lexed into *cells* (table cells, json values, numbers in
//! prose) and the *skeleton* around them. The skeleton is cut into lines;
//! each distinct line skeleton is stored once (a template: one layout, many
//! values). Cells are stored as token streams, in file order or by column
//! (one stream per skeleton slot, so the same report row in successive runs
//! lands side by side). Options:
//! - `columns`: one stream per (line skeleton, slot) instead of file order;
//! - `numbers`: cells that read as numbers (`-$1,234.50`, `12.5%`) are
//!   stored as a delta against the previous number in their stream;
//! - `siblings`: a file whose name differs from an earlier file's only in
//!   its extension (the same table as csv, json and md) stores its cells as
//!   a diff against that file's cells, so the facts are stored once across
//!   formats (canonical form);
//! - `rows` (with `columns`): one stream per (line skeleton, slot, the
//!   line's first cell), so a table row keyed by its first cell (a report's
//!   link id) is stored beside the same row of other files.
//!
//! `decode(encode(files, o))` returns `files` byte-exact for any bytes and
//! any options (`tests/ir.rs` checks this by property). The pack is
//! uncompressed; a general compressor (xz, zstd) runs over it.

pub mod query;
pub mod schema;

use std::collections::{HashMap, HashSet};

use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::Error;

#[derive(Clone, Copy, Debug, Default)]
pub struct Options {
    pub columns: bool,
    pub numbers: bool,
    pub siblings: bool,
    pub rows: bool,
    /// E198: a file under a tenth of whose bytes are cells (prose) is stored
    /// as it is, in one raw part with the other such
    /// files, so the general compressor sees running text.
    pub prose: bool,
    /// E199: with a window of `nearest` (0 is off), a file's cells are
    /// stored as a diff against whichever of the last `nearest` files with
    /// its extension (or its sibling, with `siblings`) shares the most
    /// cells with it, so a rerun stores only the cells that changed.
    pub nearest: usize,
    /// E200 (with `columns`): each column learns which earlier cell of its
    /// line determines it (a client's region, a link's sensor) and stores
    /// a cell its predictor gets right as one byte.
    pub predict: bool,
    /// E201: the streams an encode adds are laid out sorted by their key
    /// (line skeleton, slot, row key) instead of in the order they first
    /// appear, so the same column of every row sits together for the
    /// general compressor.
    pub group: bool,
}

/// An encoded set of files: the line skeletons, each file's lines and cell
/// diff, and the cell streams (one, or one per column in (skeleton, slot)
/// order). Each part can be compressed and read on its own.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parts {
    pub skeletons: Vec<Vec<u8>>,
    pub files: Vec<u8>,
    pub directory: Vec<u8>,
    pub streams: Vec<Vec<u8>>,
    /// Files stored as they are (`Options::prose`), one after another.
    pub raw: Vec<u8>,
}

/// A lexed line: its skeleton (segments around the cells) and its cells.
struct Line<'a> {
    segs: Vec<&'a [u8]>,
    cells: Vec<&'a [u8]>,
}

fn put_var(out: &mut Vec<u8>, mut v: u128) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn get_var(b: &[u8], at: &mut usize) -> Result<u128, Error> {
    let (mut v, mut shift) = (0u128, 0);
    loop {
        let c = *b.get(*at).ok_or(Error::Decode("ir: truncated varint"))?;
        *at += 1;
        if shift > 126 {
            return Err(Error::Decode("ir: varint too long"));
        }
        v |= u128::from(c & 0x7f) << shift;
        if c < 0x80 {
            return Ok(v);
        }
        shift += 7;
    }
}

fn get_usize(b: &[u8], at: &mut usize) -> Result<usize, Error> {
    usize::try_from(get_var(b, at)?).map_err(|_| Error::Decode("ir: length overflow"))
}

fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_var(out, b.len() as u128);
    out.extend_from_slice(b);
}

fn get_bytes<'a>(b: &'a [u8], at: &mut usize) -> Result<&'a [u8], Error> {
    let n = get_usize(b, at)?;
    let s = b
        .get(*at..*at + n)
        .ok_or(Error::Decode("ir: truncated bytes"))?;
    *at += n;
    Ok(s)
}

fn zigzag(v: i128) -> u128 {
    ((v << 1) ^ (v >> 127)) as u128
}

fn unzigzag(v: u128) -> i128 {
    (v >> 1) as i128 ^ -((v & 1) as i128)
}

// ---- lexers: (is_cell, range) pieces covering the file in order ----

type Pieces = Vec<(bool, usize, usize)>;

fn ext(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(_, e)| e)
}

fn lex(name: &str, b: &[u8]) -> Pieces {
    match ext(name) {
        "csv" => lex_csv(b),
        "json" | "jsonl" | "ndjson" => lex_json(b),
        _ => lex_text(b),
    }
}

fn push(p: &mut Pieces, cell: bool, s: usize, e: usize) {
    if s < e {
        p.push((cell, s, e));
    }
}

/// Numbers in prose, and the cells of markdown pipe-table lines.
fn lex_text(b: &[u8]) -> Pieces {
    let mut p = Vec::new();
    let mut s = 0;
    while s < b.len() {
        let e = memchr::memchr(b'\n', &b[s..]).map_or(b.len(), |i| s + i + 1);
        if b[s] == b'|' {
            lex_md_row(b, s, e, &mut p);
        } else {
            lex_numbers(b, s, e, &mut p);
        }
        s = e;
    }
    p
}

fn lex_md_row(b: &[u8], s: usize, e: usize, p: &mut Pieces) {
    let mut lit = s;
    let mut i = s + 1;
    while i < e {
        let end = memchr::memchr(b'|', &b[i..e]).map_or(e, |j| i + j);
        let (mut cs, mut ce) = (i, end);
        while cs < ce && b[cs] == b' ' {
            cs += 1;
        }
        while ce > cs && matches!(b[ce - 1], b' ' | b'\n' | b'\r') {
            ce -= 1;
        }
        if cs < ce && !b[cs..ce].iter().all(|c| matches!(c, b'-' | b':')) {
            push(p, false, lit, cs);
            push(p, true, cs, ce);
            lit = ce;
        }
        i = end + 1;
    }
    push(p, false, lit, e);
}

fn lex_numbers(b: &[u8], s: usize, e: usize, p: &mut Pieces) {
    let mut lit = s;
    let mut i = s;
    while i < e {
        let prev_alnum = i > s && b[i - 1].is_ascii_alphanumeric();
        let start = i;
        let mut j = i;
        if j < e && b[j] == b'-' && !prev_alnum {
            j += 1;
        }
        let digits = j;
        while j < e && b[j].is_ascii_digit() {
            j += 1;
        }
        if j > digits && !(digits == start && prev_alnum && b[start - 1].is_ascii_digit()) {
            if j + 1 < e && b[j] == b'.' && b[j + 1].is_ascii_digit() {
                j += 1;
                while j < e && b[j].is_ascii_digit() {
                    j += 1;
                }
            }
            push(p, false, lit, start);
            push(p, true, start, j);
            lit = j;
            i = j;
        } else {
            i = start + 1;
        }
    }
    push(p, false, lit, e);
}

fn lex_csv(b: &[u8]) -> Pieces {
    let mut p = Vec::new();
    let (mut lit, mut i) = (0, 0);
    while i < b.len() {
        if b[i] == b'"' {
            let mut j = i + 1;
            while j < b.len() && !(b[j] == b'"' && b.get(j + 1) != Some(&b'"')) {
                j += if b[j] == b'"' { 2 } else { 1 };
            }
            push(&mut p, false, lit, i + 1);
            push(&mut p, true, i + 1, j.min(b.len()));
            lit = j.min(b.len());
            i = j + 1;
        } else if matches!(b[i], b',' | b'\n' | b'\r') {
            i += 1;
        } else {
            let j = i + b[i..]
                .iter()
                .position(|c| matches!(c, b',' | b'\n' | b'\r'))
                .unwrap_or(b.len() - i);
            push(&mut p, false, lit, i);
            push(&mut p, true, i, j);
            lit = j;
            i = j;
        }
    }
    push(&mut p, false, lit, b.len());
    p
}

/// String values and numbers are cells; keys (a string followed by `:`),
/// punctuation, `true`, `false` and `null` are skeleton.
fn lex_json(b: &[u8]) -> Pieces {
    let mut p = Vec::new();
    let (mut lit, mut i) = (0, 0);
    while i < b.len() {
        let c = b[i];
        if c == b'"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let end = j.min(b.len());
            let mut k = end + 1;
            while k < b.len() && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if b.get(k) != Some(&b':') {
                push(&mut p, false, lit, i + 1);
                push(&mut p, true, i + 1, end);
                lit = end;
            }
            i = end + 1;
        } else if c == b'-' || c.is_ascii_digit() {
            let j = i + b[i..]
                .iter()
                .position(|c| !matches!(c, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'))
                .unwrap_or(b.len() - i);
            push(&mut p, false, lit, i);
            push(&mut p, true, i, j);
            lit = j;
            i = j;
        } else {
            i += 1;
        }
    }
    push(&mut p, false, lit, b.len().max(lit));
    p
}

/// Pieces cut into lines at each newline in the skeleton.
fn lines<'a>(b: &'a [u8], pieces: &Pieces) -> Vec<Line<'a>> {
    let mut out = Vec::new();
    let mut cur = Line {
        segs: Vec::new(),
        cells: Vec::new(),
    };
    let mut seg = 0..0;
    for &(cell, s, e) in pieces {
        if cell {
            cur.segs.push(&b[seg.clone()]);
            cur.cells.push(&b[s..e]);
            seg = e..e;
            continue;
        }
        let mut at = s;
        if seg.is_empty() {
            seg = s..s;
        }
        while let Some(i) = memchr::memchr(b'\n', &b[at..e]) {
            seg.end = at + i + 1;
            cur.segs.push(&b[seg.clone()]);
            out.push(std::mem::replace(
                &mut cur,
                Line {
                    segs: Vec::new(),
                    cells: Vec::new(),
                },
            ));
            at += i + 1;
            seg = at..at;
        }
        seg.end = e;
    }
    if !seg.is_empty() || !cur.cells.is_empty() {
        cur.segs.push(&b[seg]);
        out.push(cur);
    }
    out
}

fn skeleton_bytes(segs: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    put_var(&mut out, segs.len() as u128);
    segs.iter().for_each(|s| put_bytes(&mut out, s));
    out
}

/// Each line's skeleton (as `encode` stores it) and cells.
pub fn lex_lines(name: &str, b: &[u8]) -> Vec<(Vec<u8>, Vec<Vec<u8>>)> {
    lines(b, &lex(name, b))
        .into_iter()
        .map(|l| {
            (
                skeleton_bytes(&l.segs),
                l.cells.iter().map(|c| c.to_vec()).collect(),
            )
        })
        .collect()
}

/// A skeleton's segments (one more than its cells).
pub fn segments(skeleton: &[u8]) -> Result<Vec<&[u8]>, Error> {
    let mut at = 0;
    let n = get_usize(skeleton, &mut at)?;
    if n == 0 || n > skeleton.len() {
        return Err(Error::Decode("ir: bad skeleton"));
    }
    (0..n).map(|_| get_bytes(skeleton, &mut at)).collect()
}

// ---- numbers ----

#[derive(Clone, Copy, PartialEq, Debug)]
struct Num {
    m: i128,
    scale: u8,
    fmt: u8,
}

const DOLLAR: u8 = 1;
const COMMAS: u8 = 2;
const PERCENT: u8 = 4;

fn render_num(n: Num) -> Vec<u8> {
    let mut out = Vec::new();
    if n.m < 0 {
        out.push(b'-');
    }
    if n.fmt & DOLLAR != 0 {
        out.push(b'$');
    }
    let digits = n.m.unsigned_abs().to_string();
    let scale = usize::from(n.scale);
    let padded = format!("{digits:0>width$}", width = scale + 1);
    let (int, frac) = padded.split_at(padded.len() - scale);
    for (i, c) in int.bytes().enumerate() {
        if n.fmt & COMMAS != 0 && i > 0 && (int.len() - i) % 3 == 0 {
            out.push(b',');
        }
        out.push(c);
    }
    if scale > 0 {
        out.push(b'.');
        out.extend_from_slice(frac.as_bytes());
    }
    if n.fmt & PERCENT != 0 {
        out.push(b'%');
    }
    out
}

/// A cell that renders back to itself as a number, or None.
fn parse_num(c: &[u8]) -> Option<Num> {
    let mut s = c;
    let neg = s.first() == Some(&b'-');
    s = &s[usize::from(neg)..];
    let mut fmt = 0;
    if s.first() == Some(&b'$') {
        fmt |= DOLLAR;
        s = &s[1..];
    }
    if s.last() == Some(&b'%') {
        fmt |= PERCENT;
        s = &s[..s.len() - 1];
    }
    if s.contains(&b',') {
        fmt |= COMMAS;
    }
    let (int, frac) = match memchr::memchr(b'.', s) {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, &s[..0]),
    };
    let digits: Vec<u8> = int
        .iter()
        .chain(frac)
        .copied()
        .filter(|&c| c != b',')
        .collect();
    if digits.is_empty() || digits.len() > 30 || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let m: i128 = std::str::from_utf8(&digits).ok()?.parse().ok()?;
    let n = Num {
        m: if neg { -m } else { m },
        scale: u8::try_from(frac.len()).ok()?,
        fmt,
    };
    (render_num(n) == c).then_some(n)
}

// ---- cell token streams ----
//
// Tokens: 0 the previous value again; 1 a number with the previous
// number's scale and format, as a delta; 2 a number with its format, scale
// and delta; 3 a string seen before in this stream, by index; 4 a new
// string; 5 (E200, `predict`) the value the column's predictor gives.

/// One earlier cell's map: each of its values to this column's value.
type Predictor = HashMap<Vec<u8>, Vec<u8>>;

/// How many of a line's first cells a column may be predicted from (E200).
const PREDICTORS: usize = 4;

#[derive(Clone, Default)]
struct Stream {
    out: Vec<u8>,
    prev: Option<Vec<u8>>,
    num: Option<Num>,
    dict: HashMap<Vec<u8>, usize>,
    seen: Vec<Vec<u8>>,
    /// For each of the line's first `PREDICTORS` cells, by place: the value this
    /// column last had beside each of its values, and how often that held.
    pred: Vec<(Predictor, usize)>,
}

impl Stream {
    /// The value the best-scoring earlier cell of the line maps to.
    fn predicted(&self, line: &[&[u8]]) -> Option<&Vec<u8>> {
        let mut best: Option<(usize, &Vec<u8>)> = None;
        for (j, cell) in line.iter().take(PREDICTORS).enumerate() {
            let Some((map, hits)) = self.pred.get(j) else {
                break;
            };
            if let Some(v) = map.get(*cell)
                && best.is_none_or(|b| *hits > b.0)
            {
                best = Some((*hits, v));
            }
        }
        best.map(|b| b.1)
    }

    fn learn(&mut self, line: &[&[u8]], v: &[u8]) {
        let line = &line[..line.len().min(PREDICTORS)];
        if self.pred.len() < line.len() {
            self.pred.resize_with(line.len(), Default::default);
        }
        for (j, cell) in line.iter().enumerate() {
            let (map, hits) = &mut self.pred[j];
            match map.get_mut(*cell) {
                Some(old) if old.as_slice() == v => *hits += 1,
                Some(old) => *old = v.to_vec(),
                None => {
                    map.insert(cell.to_vec(), v.to_vec());
                }
            }
        }
    }

    /// `put` with the line's earlier cells, when the column predicts.
    fn put_in(&mut self, c: &[u8], numbers: bool, line: Option<&[&[u8]]>) {
        let Some(line) = line else {
            return self.put(c, numbers);
        };
        if self.prev.as_deref() != Some(c) && self.predicted(line).map(Vec::as_slice) == Some(c) {
            self.out.push(5);
            if let Some(n) = parse_num(c).filter(|_| numbers) {
                self.num = Some(n);
            }
            self.prev = Some(c.to_vec());
        } else {
            self.put(c, numbers);
        }
        self.learn(line, c);
    }

    /// `get` with the line's earlier cells, when the column predicts.
    fn get_in(
        &mut self,
        b: &[u8],
        at: &mut usize,
        line: Option<&[&[u8]]>,
    ) -> Result<Vec<u8>, Error> {
        let Some(line) = line else {
            return self.get(b, at);
        };
        let v = if b.get(*at) == Some(&5) {
            *at += 1;
            let v = self
                .predicted(line)
                .cloned()
                .ok_or(Error::Decode("ir: nothing predicted"))?;
            if let Some(n) = parse_num(&v) {
                self.num = Some(n);
            }
            self.prev = Some(v.clone());
            v
        } else {
            self.get(b, at)?
        };
        self.learn(line, &v);
        Ok(v)
    }

    fn put(&mut self, c: &[u8], numbers: bool) {
        if self.prev.as_deref() == Some(c) {
            self.out.push(0);
        } else if let Some(n) = parse_num(c).filter(|_| numbers) {
            let p = self.num.unwrap_or(Num {
                m: 0,
                scale: n.scale,
                fmt: n.fmt,
            });
            if self.num.is_some() && p.scale == n.scale && p.fmt == n.fmt {
                self.out.push(1);
            } else {
                self.out.extend([2, n.fmt, n.scale]);
            }
            put_var(&mut self.out, zigzag(n.m - p.m));
            self.num = Some(n);
        } else if let Some(&i) = self.dict.get(c) {
            self.out.push(3);
            put_var(&mut self.out, i as u128);
        } else {
            self.out.push(4);
            put_bytes(&mut self.out, c);
            self.dict.insert(c.to_vec(), self.dict.len());
        }
        self.prev = Some(c.to_vec());
    }

    fn get(&mut self, b: &[u8], at: &mut usize) -> Result<Vec<u8>, Error> {
        let tag = *b.get(*at).ok_or(Error::Decode("ir: stream ended"))?;
        *at += 1;
        let v = match tag {
            0 => self
                .prev
                .clone()
                .ok_or(Error::Decode("ir: no previous value"))?,
            1 | 2 => {
                let mut p = self.num.ok_or(Error::Decode("ir: no previous number"));
                if tag == 2 {
                    let h = b
                        .get(*at..*at + 2)
                        .ok_or(Error::Decode("ir: truncated number"))?;
                    *at += 2;
                    let base = self.num.map_or(0, |n| n.m);
                    p = Ok(Num {
                        m: base,
                        fmt: h[0],
                        scale: h[1],
                    });
                }
                let p = p?;
                let n = Num {
                    m: p.m + unzigzag(get_var(b, at)?),
                    ..p
                };
                self.num = Some(n);
                render_num(n)
            }
            3 => {
                let i = get_usize(b, at)?;
                self.seen
                    .get(i)
                    .cloned()
                    .ok_or(Error::Decode("ir: bad string index"))?
            }
            4 => {
                let s = get_bytes(b, at)?.to_vec();
                self.seen.push(s.clone());
                s
            }
            _ => return Err(Error::Decode("ir: bad token")),
        };
        self.prev = Some(v.clone());
        Ok(v)
    }
}

/// Every value of one stream, decoded on its own (a column read).
pub fn stream_values(stream: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    let (mut s, mut at, mut out) = (Stream::default(), 0, Vec::new());
    while at < stream.len() {
        out.push(s.get(stream, &mut at)?);
    }
    Ok(out)
}

// ---- encode / decode ----

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

/// Copy (from, len) and insert (len) runs of a cell diff.
type Ops = Vec<(bool, usize, usize)>;

/// The cells `cells` shares with `base`, and its copy and insert runs
/// against `base` (None when nothing is shared).
fn cell_diff(base: &[&[u8]], cells: &[&[u8]]) -> (usize, Option<Ops>) {
    let mut ops = Vec::new();
    let mut shared = 0;
    for op in capture_diff_slices(Algorithm::Myers, base, cells) {
        match op {
            DiffOp::Equal { old_index, len, .. } => {
                shared += len;
                ops.push((true, old_index, len));
            }
            DiffOp::Insert { new_len, .. } | DiffOp::Replace { new_len, .. } => {
                ops.push((false, 0, new_len));
            }
            DiffOp::Delete { .. } => {}
        }
    }
    (shared, (shared > 0).then_some(ops))
}

/// The stream a cell goes to: one stream, one per (skeleton, slot), or with
/// `rows` one per (skeleton, slot, the line's first cell), so a table row
/// keyed by its first cell lines up with the same row in other files.
fn stream_key(o: Options, slot: (usize, usize), first: &[u8]) -> (usize, usize, Vec<u8>) {
    match (o.columns, o.rows && slot.1 > 0) {
        (false, _) => (0, 0, Vec::new()),
        (true, false) => (slot.0, slot.1, Vec::new()),
        (true, true) => (slot.0, slot.1, first.to_vec()),
    }
}

/// Encodes `files` (names and bytes) with the given options.
pub fn encode(files: &[(String, Vec<u8>)], o: Options) -> Parts {
    let mut table = Skeletons::default();
    let mut parts = encode_with(files, o, &mut table, &mut EncState::default());
    parts.skeletons = table.list;
    parts
}

/// A skeleton table shared by several encodes (E197 blocks).
#[derive(Default)]
struct Skeletons {
    ids: HashMap<Vec<u8>, usize>,
    list: Vec<Vec<u8>>,
}

/// The streams after an encode, so a later encode can go on from them (a
/// block coded against a keyframe block, E197).
/// A stream's key: line skeleton, slot, row key.
type StreamKey = (usize, usize, Vec<u8>);

#[derive(Clone, Default)]
struct EncState {
    index: HashMap<StreamKey, usize>,
    streams: Vec<Stream>,
}

/// `encode` against a skeleton table that outlives it, going on from the
/// streams in `state`: the returned parts have no skeletons of their own,
/// and their directory names only the streams this encode added.
fn encode_with(
    files: &[(String, Vec<u8>)],
    o: Options,
    table: &mut Skeletons,
    state: &mut EncState,
) -> Parts {
    let mut parts = Parts::default();
    let EncState { index, streams } = state;
    let first_new = streams.len();
    let lexed: Vec<Vec<Line>> = files.iter().map(|(n, b)| lines(b, &lex(n, b))).collect();
    // prose: under a tenth of the file's bytes are cells (E198 measured 2 to
    // 5% for prose, 14% and up for reports and tables)
    let raw: Vec<bool> = files
        .iter()
        .zip(&lexed)
        .map(|((_, b), fl)| {
            let cells: usize = fl.iter().flat_map(|l| &l.cells).map(|c| c.len()).sum();
            o.prose && !b.is_empty() && cells * 10 < b.len()
        })
        .collect();
    let mut all_cells: Vec<Vec<&[u8]>> = Vec::new();
    let mut by_stem: HashMap<&str, usize> = HashMap::new();
    let mut by_ext: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut cell_sets: Vec<HashSet<&[u8]>> = Vec::new();
    parts.files.push(
        u8::from(o.columns)
            | u8::from(o.rows) << 1
            | u8::from(o.prose) << 2
            | u8::from(o.predict && o.columns) << 3,
    );
    put_var(&mut parts.files, files.len() as u128);
    for (fi, ((name, b), ls)) in files.iter().zip(lexed).enumerate() {
        put_bytes(&mut parts.files, name.as_bytes());
        if o.prose {
            parts.files.push(u8::from(raw[fi]));
            if raw[fi] {
                put_var(&mut parts.files, b.len() as u128);
                parts.raw.extend_from_slice(b);
                all_cells.push(Vec::new());
                cell_sets.push(HashSet::new());
                continue;
            }
        }
        put_var(&mut parts.files, ls.len() as u128);
        let mut slots = Vec::new();
        for l in &ls {
            let key = skeleton_bytes(&l.segs);
            let next = table.list.len();
            let id = *table.ids.entry(key.clone()).or_insert_with(|| {
                table.list.push(key);
                next
            });
            put_var(&mut parts.files, id as u128);
            slots.extend((0..l.cells.len()).map(|k| (id, k)));
        }
        let mut firsts = Vec::new();
        let mut at = 0;
        for l in &ls {
            firsts.extend(std::iter::repeat_n(at, l.cells.len()));
            at += l.cells.len();
        }
        let cells: Vec<&[u8]> = ls.iter().flat_map(|l| l.cells.iter().copied()).collect();
        let sibling = by_stem.get(stem(name)).copied().filter(|_| o.siblings);
        let recent = by_ext.get(ext(name)).map_or(&[][..], |v: &Vec<usize>| {
            &v[v.len().saturating_sub(o.nearest)..]
        });
        // a cheap first look (how many of this file's cells each candidate
        // holds anywhere) picks the two likeliest bases; the diff runs on
        // those and the sibling only
        let mut likely: Vec<(usize, usize)> = recent
            .iter()
            .map(|&bi| {
                let have = &cell_sets[bi];
                (cells.iter().filter(|c| have.contains(*c)).count(), bi)
            })
            .collect();
        likely.sort_unstable_by(|a, b| b.cmp(a));
        let mut best: (usize, Option<usize>, Option<Ops>) = (0, None, None);
        for bi in sibling
            .into_iter()
            .chain(likely.iter().take(2).map(|x| x.1))
        {
            let (shared, ops) = cell_diff(&all_cells[bi], &cells);
            if shared > best.0 {
                best = (shared, Some(bi), ops);
            }
        }
        let (_, base, ops) = best;
        put_var(
            &mut parts.files,
            ops.as_ref().map_or(0, |_| base.unwrap() + 1) as u128,
        );
        let mut inserted = vec![ops.is_none(); cells.len()];
        if let Some(ops) = &ops {
            put_var(&mut parts.files, ops.len() as u128);
            let mut at = 0;
            for &(copy, from, n) in ops {
                put_var(&mut parts.files, u128::from(copy));
                if copy {
                    put_var(&mut parts.files, from as u128);
                } else {
                    inserted[at..at + n].iter_mut().for_each(|x| *x = true);
                }
                put_var(&mut parts.files, n as u128);
                at += n;
            }
        }
        for (k, c) in cells.iter().enumerate().filter(|(k, _)| inserted[*k]) {
            let key = stream_key(o, slots[k], cells[firsts[k]]);
            let next = streams.len();
            let si = *index.entry(key.clone()).or_insert_with(|| {
                put_var(&mut parts.directory, key.0 as u128);
                put_var(&mut parts.directory, key.1 as u128);
                put_bytes(&mut parts.directory, &key.2);
                next
            });
            if si == streams.len() {
                streams.push(Stream::default());
            }
            let line = (o.predict && o.columns).then(|| &cells[firsts[k]..k]);
            streams[si].put_in(c, o.numbers, line);
        }
        by_stem.entry(stem(name)).or_insert(fi);
        if o.nearest > 0 && !cells.is_empty() {
            by_ext.entry(ext(name)).or_default().push(fi);
        }
        cell_sets.push(if o.nearest > 0 {
            cells.iter().copied().collect()
        } else {
            HashSet::new()
        });
        all_cells.push(cells);
    }
    if o.group && streams.len() > first_new + 1 {
        let mut keys: Vec<(StreamKey, usize)> = index
            .iter()
            .filter(|(_, i)| **i >= first_new)
            .map(|(k, i)| (k.clone(), *i))
            .collect();
        keys.sort_unstable();
        let mut moved: Vec<Option<Stream>> = streams.drain(first_new..).map(Some).collect();
        parts.directory.clear();
        for (key, old) in keys {
            put_var(&mut parts.directory, key.0 as u128);
            put_var(&mut parts.directory, key.1 as u128);
            put_bytes(&mut parts.directory, &key.2);
            index.insert(key, streams.len());
            streams.push(moved[old - first_new].take().expect("each stream once"));
        }
    }
    parts.streams = streams
        .iter_mut()
        .map(|s| std::mem::take(&mut s.out))
        .collect();
    parts
}

/// The streams after a decode (the decoding side of `EncState`).
#[derive(Clone, Default)]
struct DecState {
    index: HashMap<StreamKey, usize>,
    states: Vec<Stream>,
}

/// Reads cells from the streams in the order `encode` wrote them.
struct Cursor<'a> {
    streams: &'a [Vec<u8>],
    state: &'a mut DecState,
    pos: Vec<usize>,
    o: Options,
}

impl Cursor<'_> {
    fn next(
        &mut self,
        slot: (usize, usize),
        first: &[u8],
        line: Option<&[&[u8]]>,
    ) -> Result<Vec<u8>, Error> {
        let key = stream_key(self.o, slot, first);
        let i = *self
            .state
            .index
            .get(&key)
            .ok_or(Error::Decode("ir: no stream"))?;
        let st = self
            .state
            .states
            .get_mut(i)
            .ok_or(Error::Decode("ir: no stream"))?;
        st.get_in(&self.streams[i], &mut self.pos[i], line)
    }
}

/// The stream keys, in stream order: (skeleton, slot, first cell).
pub fn directory(parts: &Parts) -> Result<Vec<(usize, usize, Vec<u8>)>, Error> {
    let (d, mut at, mut out) = (&parts.directory, 0, Vec::new());
    while at < d.len() {
        let id = get_usize(d, &mut at)?;
        let k = get_usize(d, &mut at)?;
        out.push((id, k, get_bytes(d, &mut at)?.to_vec()));
    }
    Ok(out)
}

/// The files `encode` was given, byte-exact.
pub fn decode(parts: &Parts) -> Result<Vec<(String, Vec<u8>)>, Error> {
    decode_with(parts, &parts.skeletons, &mut DecState::default())
}

/// `decode` with the skeletons given apart from the parts, going on from the
/// streams in `state` (E197 blocks).
fn decode_with(
    parts: &Parts,
    skeletons: &[Vec<u8>],
    state: &mut DecState,
) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let f = &parts.files;
    let flags = *f.first().ok_or(Error::Decode("ir: empty"))?;
    let o = Options {
        columns: flags & 1 != 0,
        rows: flags & 2 != 0,
        prose: flags & 4 != 0,
        predict: flags & 8 != 0,
        ..Options::default()
    };
    let mut raw_at: usize = 0;
    let skels: Vec<Vec<&[u8]>> = skeletons
        .iter()
        .map(|s| segments(s))
        .collect::<Result<_, _>>()?;
    let base = state.states.len();
    let keys = directory(parts)?;
    if parts.streams.len() != base + keys.len() {
        return Err(Error::Decode("ir: stream count"));
    }
    for (i, k) in keys.into_iter().enumerate() {
        state.index.insert(k, base + i);
    }
    if !o.columns {
        state.index.entry((0, 0, Vec::new())).or_insert(0);
    }
    state
        .states
        .resize_with(parts.streams.len(), Stream::default);
    let mut cur = Cursor {
        streams: &parts.streams,
        state,
        pos: vec![0; parts.streams.len()],
        o,
    };
    let mut at = 1;
    let n = get_usize(f, &mut at)?;
    let mut out = Vec::with_capacity(n.min(1 << 20));
    let mut all_cells: Vec<Vec<Vec<u8>>> = Vec::new();
    for _ in 0..n {
        let name = String::from_utf8(get_bytes(f, &mut at)?.to_vec())
            .map_err(|_| Error::Decode("ir: name"))?;
        if o.prose {
            let kind = *f.get(at).ok_or(Error::Decode("ir: truncated"))?;
            at += 1;
            if kind == 1 {
                let len = get_usize(f, &mut at)?;
                let bytes = parts
                    .raw
                    .get(raw_at..raw_at.saturating_add(len))
                    .ok_or(Error::Decode("ir: raw"))?;
                raw_at += len;
                all_cells.push(Vec::new());
                out.push((name, bytes.to_vec()));
                continue;
            }
        }
        let nl = get_usize(f, &mut at)?;
        let ids: Vec<usize> = (0..nl)
            .map(|_| get_usize(f, &mut at))
            .collect::<Result<_, _>>()?;
        if ids.iter().any(|&i| i >= skels.len()) {
            return Err(Error::Decode("ir: bad skeleton id"));
        }
        let mut slots = Vec::new();
        for &id in &ids {
            let first = slots.len();
            slots.extend((0..skels[id].len() - 1).map(|k| ((id, k), first)));
        }
        let base = get_usize(f, &mut at)?;
        let mut cells: Vec<Vec<u8>> = Vec::with_capacity(slots.len());
        let mut plan: Vec<Option<usize>> = Vec::new();
        if base == 0 {
            plan.resize(slots.len(), None);
        } else {
            let bc = all_cells
                .get(base - 1)
                .ok_or(Error::Decode("ir: bad base"))?;
            for _ in 0..get_usize(f, &mut at)? {
                let copy = get_usize(f, &mut at)? == 1;
                let from = if copy { get_usize(f, &mut at)? } else { 0 };
                let len = get_usize(f, &mut at)?;
                if plan.len() + len > slots.len() || (copy && from + len > bc.len()) {
                    return Err(Error::Decode("ir: bad diff"));
                }
                plan.extend((0..len).map(|k| copy.then_some(from + k)));
            }
        }
        if plan.len() != slots.len() {
            return Err(Error::Decode("ir: cell count"));
        }
        for (k, p) in plan.iter().enumerate() {
            let v = match p {
                Some(i) => all_cells[base - 1][*i].clone(),
                None => {
                    let (slot, first) = slots[k];
                    let line: Vec<&[u8]> = if o.predict {
                        cells[first..k].iter().map(Vec::as_slice).collect()
                    } else {
                        Vec::new()
                    };
                    let first = if first < k {
                        cells[first].clone()
                    } else {
                        Vec::new()
                    };
                    cur.next(slot, &first, o.predict.then_some(&line[..]))?
                }
            };
            cells.push(v);
        }
        let mut bytes = Vec::new();
        let mut ci = cells.iter();
        for &id in &ids {
            for (k, seg) in skels[id].iter().enumerate() {
                if k > 0 {
                    bytes.extend_from_slice(ci.next().unwrap());
                }
                bytes.extend_from_slice(seg);
            }
        }
        all_cells.push(cells);
        out.push((name, bytes));
    }
    Ok(out)
}

// ---- blocks (E197) ----

/// Files cut into blocks of a fixed number of files, in order. One skeleton
/// table is shared by every block; each block's cells, streams and diffs are
/// its own, so reading one file decodes one block, not every file. Siblings
/// (`Options::siblings`) only reach files in the same block.
///
/// With `keyframe`, every block after the first goes on from the streams as
/// the first block left them (its strings, its last numbers), as a video
/// frame is coded against a keyframe: a read decodes the first block (once,
/// it can be kept) and the block it wants, and no block restarts its deltas
/// from nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Blocks {
    pub per: usize,
    pub keyframe: bool,
    pub skeletons: Vec<Vec<u8>>,
    /// Each block's parts, with no skeletons of their own.
    pub blocks: Vec<Parts>,
}

/// Encodes `files` as blocks of `per` files (at least 1).
pub fn encode_blocks(
    files: &[(String, Vec<u8>)],
    o: Options,
    per: usize,
    keyframe: bool,
) -> Blocks {
    let per = per.max(1);
    let mut table = Skeletons::default();
    let mut key = EncState::default();
    let mut blocks = Vec::new();
    for (i, c) in files.chunks(per).enumerate() {
        let mut state = if keyframe && i > 0 {
            key.clone()
        } else {
            EncState::default()
        };
        blocks.push(encode_with(c, o, &mut table, &mut state));
        if keyframe && i == 0 {
            key = state;
        }
    }
    Blocks {
        per,
        keyframe,
        skeletons: table.list,
        blocks,
    }
}

type Files = Vec<(String, Vec<u8>)>;

/// Reads blocks one at a time, keeping the decoded keyframe (if any) between
/// reads.
pub struct Reader<'a> {
    b: &'a Blocks,
    key: Option<(Files, DecState)>,
}

/// A reader of `b`'s blocks, which decodes the keyframe at most once.
pub fn reader(b: &Blocks) -> Reader<'_> {
    Reader { b, key: None }
}

impl Reader<'_> {
    /// The files of block `i`, byte-exact.
    pub fn block(&mut self, i: usize) -> Result<Vec<(String, Vec<u8>)>, Error> {
        let b = self.b;
        let p = b.blocks.get(i).ok_or(Error::Decode("ir: no block"))?;
        if !b.keyframe {
            return decode_with(p, &b.skeletons, &mut DecState::default());
        }
        if self.key.is_none() {
            let mut st = DecState::default();
            let first = b.blocks.first().ok_or(Error::Decode("ir: no block"))?;
            let files = decode_with(first, &b.skeletons, &mut st)?;
            self.key = Some((files, st));
        }
        let (files, st) = self.key.as_ref().expect("set above");
        if i == 0 {
            return Ok(files.clone());
        }
        decode_with(p, &b.skeletons, &mut st.clone())
    }
}

/// The files of block `i`, byte-exact (decoding the keyframe first if `b`
/// has one).
pub fn decode_block(b: &Blocks, i: usize) -> Result<Vec<(String, Vec<u8>)>, Error> {
    reader(b).block(i)
}

/// Every file of every block, in order.
pub fn decode_blocks(b: &Blocks) -> Result<Vec<(String, Vec<u8>)>, Error> {
    let mut r = reader(b);
    let mut out = Vec::new();
    for i in 0..b.blocks.len() {
        out.extend(r.block(i)?);
    }
    Ok(out)
}

/// Parts as one byte string (length-prefixed), for a solid compressor.
pub fn pack(p: &Parts) -> Vec<u8> {
    let mut out = Vec::new();
    put_var(&mut out, p.skeletons.len() as u128);
    p.skeletons.iter().for_each(|s| put_bytes(&mut out, s));
    put_bytes(&mut out, &p.files);
    put_bytes(&mut out, &p.directory);
    put_var(&mut out, p.streams.len() as u128);
    p.streams.iter().for_each(|s| put_bytes(&mut out, s));
    // E198: the raw part only when there is one, so packs without it read
    // as before
    if !p.raw.is_empty() {
        put_bytes(&mut out, &p.raw);
    }
    out
}

/// The inverse of `pack`.
pub fn unpack(b: &[u8]) -> Result<Parts, Error> {
    let mut at = 0;
    let list = |at: &mut usize| -> Result<Vec<Vec<u8>>, Error> {
        let n = get_usize(b, at)?;
        if n > b.len() {
            return Err(Error::Decode("ir: bad count"));
        }
        (0..n)
            .map(|_| get_bytes(b, at).map(<[u8]>::to_vec))
            .collect()
    };
    let skeletons = list(&mut at)?;
    let files = get_bytes(b, &mut at)?.to_vec();
    let directory = get_bytes(b, &mut at)?.to_vec();
    let streams = list(&mut at)?;
    let raw = if at < b.len() {
        get_bytes(b, &mut at)?.to_vec()
    } else {
        Vec::new()
    };
    Ok(Parts {
        skeletons,
        files,
        directory,
        streams,
        raw,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_render_back_exactly() {
        for s in ["0", "-3", "12.50", "$1,650,000", "-$2.5", "12.5%", "0.031"] {
            assert_eq!(
                render_num(parse_num(s.as_bytes()).unwrap()),
                s.as_bytes(),
                "{s}"
            );
        }
        for s in ["007", "-0", "1,23", "1.", ".5", "$", "1e5"] {
            assert!(parse_num(s.as_bytes()).is_none(), "{s}");
        }
    }
}
