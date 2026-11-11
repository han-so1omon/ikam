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
    /// E204 (with `predict`): a number column also learns, per earlier
    /// cell of its line, the step its value took beside each of that cell's
    /// values last time, so a counter or a steady trend (one more each run)
    /// is predicted as one byte too.
    pub step: bool,
    /// E208: each kind of file (extension and first-line template) is
    /// stored as it is when that compresses smaller than its IR, judged by
    /// xz over the pack, so the IR never costs more than storing the files
    /// plain. The decoder needs nothing new (the files go in the raw part
    /// of `prose`).
    pub route: bool,
    /// E209: each column (stream) chooses for itself whether it stores
    /// numbers as deltas, predicted cells as one byte, and stepped numbers
    /// as one byte, among what the other options turn on. The encoder tries
    /// each choice on every stream, keeps per stream the one whose bytes xz
    /// judges smallest, and keeps the result only if the whole pack is
    /// smaller. The decoder needs nothing new (each token says what it is).
    pub tune: bool,
    /// E210 (with `tune`): a stream may also write every string out in
    /// full instead of as an index into its dictionary, which xz can match
    /// across the whole column better than an index.
    pub literal: bool,
    /// E211: a CSV table's rows are stored sorted by the column whose order
    /// xz judges smallest by column (a code, a city), with that column also
    /// kept in file order, from which the decoder recovers the rows' order
    /// (a stable sort of it), so no permutation is stored.
    pub sort: bool,
    /// E212: a CSV table stored as it is (by `route`, or as prose) is laid
    /// out by column instead: the header, each line's field count, then
    /// each column's fields one per line, so xz sees a column as one run.
    /// A table `sort` rewrote is laid out by column after it is sorted.
    pub transpose: bool,
    /// E214: `route` and `tune` judge on a sample of about this many bytes
    /// (0: on every file): every file, each over its share (and 4 KB) cut
    /// to its first line and an even spread of its other lines. What they
    /// choose then applies to every file.
    pub sample: usize,
    /// E216 (with `route`): `route` also judges, per kind of file, whether
    /// its files diff against a base file (`siblings`, `nearest`) or store
    /// every cell, since a base helps exports that repeat each other and
    /// costs reports whose columns the predictors already catch. The
    /// decoder needs nothing new (each file says whether it has a base).
    pub bases: bool,
}

/// How `route` stores a file: through the IR (0), as it is (`PLAIN`), or
/// through the IR with no base file (`UNBASED`, E216).
const PLAIN: u8 = 1;
const UNBASED: u8 = 2;

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
    let scale = usize::from(n.scale);
    let mut out = Vec::with_capacity(scale + 24);
    if n.m < 0 {
        out.push(b'-');
    }
    if n.fmt & DOLLAR != 0 {
        out.push(b'$');
    }
    // the digits of |m|, at least scale + 1 of them
    let mut buf = [0u8; 300];
    let mut i = buf.len();
    let mut v = n.m.unsigned_abs();
    while v > u128::from(u64::MAX) {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    let mut w = v as u64;
    loop {
        i -= 1;
        buf[i] = b'0' + (w % 10) as u8;
        w /= 10;
        if w == 0 {
            break;
        }
    }
    while buf.len() - i < scale + 1 {
        i -= 1;
        buf[i] = b'0';
    }
    let digits = &buf[i..];
    let (int, frac) = digits.split_at(digits.len() - scale);
    for (k, &c) in int.iter().enumerate() {
        if n.fmt & COMMAS != 0 && k > 0 && (int.len() - k) % 3 == 0 {
            out.push(b',');
        }
        out.push(c);
    }
    if scale > 0 {
        out.push(b'.');
        out.extend_from_slice(frac);
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
    let (mut m, mut count) = (0i128, 0usize);
    for &c in int.iter().chain(frac) {
        if c == b',' {
            continue;
        }
        if !c.is_ascii_digit() {
            return None;
        }
        count += 1;
        if count > 30 {
            return None;
        }
        m = m * 10 + i128::from(c - b'0');
    }
    if count == 0 {
        return None;
    }
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
// string; 5 (E200, `predict`) the value the column's predictor gives; 6
// (E204, `step`) the number its step predictor gives.

/// A fast, fixed hash for the predictors' short keys (FxHash): the maps
/// are rebuilt the same way by the decoder, so no seed is needed.
#[derive(Clone, Copy, Default)]
struct Fx(u64);

impl std::hash::Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        const K: u64 = 0x517c_c1b7_2722_0a95;
        let mut c = bytes.chunks_exact(8);
        for w in &mut c {
            let w = u64::from_le_bytes(w.try_into().unwrap());
            self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(K);
        }
        for &x in c.remainder() {
            self.0 = (self.0.rotate_left(5) ^ u64::from(x)).wrapping_mul(K);
        }
    }

    fn write_usize(&mut self, n: usize) {
        self.0 = (self.0.rotate_left(5) ^ n as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

type FxMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<Fx>>;

/// What a column last had beside one value of an earlier cell: its value,
/// and (E204, with `step`) its last number and the step it took then.
#[derive(Clone)]
struct Seen {
    value: Vec<u8>,
    num: Option<(Num, i128)>,
}

/// One earlier cell's map: each of its values to what this column had
/// beside it, and how often the value repeated and the step repeated.
#[derive(Clone, Default)]
struct Predictor {
    map: FxMap<Vec<u8>, Seen>,
    hits: usize,
    step_hits: usize,
}

/// E209 stream modes: what a column's encoder leaves out, though the options
/// allow it.
const NO_NUMBERS: u8 = 1;
const NO_PREDICT: u8 = 2;
const NO_STEP: u8 = 4;
/// E210: strings always written out, never as an index into the stream's
/// dictionary (xz finds the repeats itself).
const NO_DICT: u8 = 8;

/// Per stream key, the mode `tune` chose (absent: 0).
type Modes = FxMap<StreamKey, u8>;

/// How many of a line's first cells a column may be predicted from (E200).
const PREDICTORS: usize = 4;

#[derive(Clone, Default)]
struct Stream {
    out: Vec<u8>,
    prev: Option<Vec<u8>>,
    num: Option<Num>,
    dict: FxMap<Vec<u8>, usize>,
    seen: Vec<Vec<u8>>,
    /// For each of the line's first `PREDICTORS` cells, by place: what this
    /// column last had beside each of its values (E200, E204).
    pred: Vec<Predictor>,
}

impl Stream {
    /// The value the best-scoring earlier cell of the line maps to.
    fn predicted<L: AsRef<[u8]>>(&self, line: &[L]) -> Option<&Vec<u8>> {
        let mut best: Option<(usize, &Vec<u8>)> = None;
        for (p, cell) in self.pred.iter().zip(line.iter().take(PREDICTORS)) {
            if let Some(e) = p.map.get(cell.as_ref())
                && best.is_none_or(|b| p.hits > b.0)
            {
                best = Some((p.hits, &e.value));
            }
        }
        best.map(|b| b.1)
    }

    /// The last number beside the best-scoring earlier cell's value plus
    /// the step it took then (E204).
    fn stepped<L: AsRef<[u8]>>(&self, line: &[L]) -> Option<Num> {
        let mut best: Option<(usize, Num)> = None;
        for (p, cell) in self.pred.iter().zip(line.iter().take(PREDICTORS)) {
            if let Some(&(n, d)) = p.map.get(cell.as_ref()).and_then(|e| e.num.as_ref())
                && d != 0
                && best.is_none_or(|b| p.step_hits > b.0)
            {
                best = Some((p.step_hits, Num { m: n.m + d, ..n }));
            }
        }
        best.map(|b| b.1)
    }

    fn learn<L: AsRef<[u8]>>(&mut self, line: &[L], v: &[u8], step: bool) {
        let line = &line[..line.len().min(PREDICTORS)];
        if self.pred.len() < line.len() {
            self.pred.resize_with(line.len(), Default::default);
        }
        let n = if step { parse_num(v) } else { None };
        for (p, cell) in self.pred.iter_mut().zip(line) {
            match p.map.get_mut(cell.as_ref()) {
                Some(e) => {
                    if e.value == v {
                        p.hits += 1;
                    } else {
                        e.value.clear();
                        e.value.extend_from_slice(v);
                    }
                    if let Some(n) = n {
                        e.num = Some(match e.num {
                            Some((old, d)) => {
                                let nd = if old.scale == n.scale && old.fmt == n.fmt {
                                    n.m - old.m
                                } else {
                                    0
                                };
                                if nd != 0 && nd == d {
                                    p.step_hits += 1;
                                }
                                (n, nd)
                            }
                            None => (n, 0),
                        });
                    }
                }
                None => {
                    p.map.insert(
                        cell.as_ref().to_vec(),
                        Seen {
                            value: v.to_vec(),
                            num: n.map(|n| (n, 0)),
                        },
                    );
                }
            }
        }
    }

    /// `put` with the line's earlier cells, when the column predicts.
    fn put_in<L: AsRef<[u8]>>(
        &mut self,
        c: &[u8],
        numbers: bool,
        line: Option<&[L]>,
        step: bool,
        mode: u8,
    ) {
        let numbers = numbers && mode & NO_NUMBERS == 0;
        let dict = mode & NO_DICT == 0;
        let Some(line) = line else {
            return self.put(c, numbers, dict);
        };
        let fresh = self.prev.as_deref() != Some(c);
        if fresh && mode & NO_PREDICT == 0 && self.predicted(line).map(Vec::as_slice) == Some(c) {
            self.out.push(5);
            if let Some(n) = parse_num(c).filter(|_| numbers) {
                self.num = Some(n);
            }
            self.prev = Some(c.to_vec());
        } else if let Some(n) = (step && fresh && mode & NO_STEP == 0)
            .then(|| self.stepped(line))
            .flatten()
            .filter(|n| render_num(*n) == c)
        {
            self.out.push(6);
            self.num = Some(n);
            self.prev = Some(c.to_vec());
        } else {
            self.put(c, numbers, dict);
        }
        self.learn(line, c, step);
    }

    /// `get` with the line's earlier cells, when the column predicts.
    fn get_in<L: AsRef<[u8]>>(
        &mut self,
        b: &[u8],
        at: &mut usize,
        line: Option<&[L]>,
        step: bool,
    ) -> Result<Vec<u8>, Error> {
        let Some(line) = line else {
            return self.get(b, at);
        };
        let v = if step && b.get(*at) == Some(&6) {
            *at += 1;
            let n = self
                .stepped(line)
                .ok_or(Error::Decode("ir: no step predicted"))?;
            let v = render_num(n);
            self.num = Some(n);
            self.prev = Some(v.clone());
            v
        } else if b.get(*at) == Some(&5) {
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
        self.learn(line, &v, step);
        Ok(v)
    }

    fn put(&mut self, c: &[u8], numbers: bool, dict: bool) {
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
        } else if let Some(&i) = self.dict.get(c).filter(|_| dict) {
            self.out.push(3);
            put_var(&mut self.out, i as u128);
        } else {
            self.out.push(4);
            put_bytes(&mut self.out, c);
            if dict {
                self.dict.insert(c.to_vec(), self.dict.len());
            }
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
    let (files, sorted) = sort_files(files, o);
    let (plain, modes) = route_modes(&files, o);
    let mut table = Skeletons::default();
    let mut parts = encode_with(
        &files,
        o,
        plain.as_slice(),
        (&modes, 0),
        &sorted,
        &mut table,
        &mut EncState::default(),
    );
    parts.skeletons = table.list;
    parts
}

/// An encode for judging (`route`, `tune`): no table rewritten by `sort`
/// is marked as such, which changes a few bytes of the files part only.
fn encode_modes(
    files: &[(String, Vec<u8>)],
    o: Options,
    plain: &[u8],
    modes: (&Modes, u8),
) -> Parts {
    let mut table = Skeletons::default();
    let mut parts = encode_with(
        files,
        o,
        plain,
        modes,
        &[],
        &mut table,
        &mut EncState::default(),
    );
    parts.skeletons = table.list;
    parts
}

/// What `sort` did to a file: its row count and whether it ended in a
/// newline, or None when it is as given.
type Sorted = Option<(usize, bool)>;

/// E211: the files with each CSV table rewritten in its sorted order where
/// `sort` finds one.
fn sort_files(files: &[(String, Vec<u8>)], o: Options) -> (Vec<(String, Vec<u8>)>, Vec<Sorted>) {
    let mut out = Vec::with_capacity(files.len());
    let mut meta = Vec::with_capacity(files.len());
    for (n, b) in files {
        let t = (o.sort && ext(n) == "csv")
            .then(|| table_lines(b))
            .flatten()
            .and_then(|(header, rows, trailing)| {
                sort_key(b).map(|k| (sorted_table(header, &rows, k), rows.len(), trailing))
            });
        match t {
            Some((t, rows, trailing)) => {
                out.push((n.clone(), t));
                meta.push(Some((rows, trailing)));
            }
            None => {
                out.push((n.clone(), b.clone()));
                meta.push(None);
            }
        }
    }
    (out, meta)
}

/// The column each CSV table would be sorted by under `sort`, by file
/// (None: kept in its own order).
pub fn sort_keys(files: &[(String, Vec<u8>)], o: Options) -> Vec<Option<usize>> {
    files
        .iter()
        .map(|(n, b)| (o.sort && ext(n) == "csv").then(|| sort_key(b)).flatten())
        .collect()
}

/// E209: per stream, which of numbers, predict and step its encoder uses.
/// Each mode is tried on every stream at once; each stream keeps the mode
/// under which xz judges its bytes smallest; the tuned pack is kept only if
/// it judges smaller than the untuned one.
/// Returns, for the streams that left something out, their key and mode
/// (1 no number deltas, 2 no predicted cells, 4 no stepped numbers).
pub fn tune_modes(files: &[(String, Vec<u8>)], o: Options) -> Vec<(StreamKey, u8)> {
    let mut m: Vec<(StreamKey, u8)> = route_modes(files, o).1.into_iter().collect();
    m.sort();
    m
}

fn tune(files: &[(String, Vec<u8>)], o: Options, plain: &[u8]) -> Modes {
    let modes = Modes::default();
    if !o.tune || files.is_empty() || (!plain.is_empty() && plain.iter().all(|p| p & PLAIN != 0)) {
        return modes;
    }
    let predicts = o.predict && o.columns;
    let mut tries = vec![0u8];
    if o.numbers {
        tries.push(NO_NUMBERS);
    }
    if predicts {
        tries.push(NO_PREDICT | NO_STEP);
        if o.step {
            tries.push(NO_STEP);
        }
        if o.numbers {
            tries.push(NO_NUMBERS | NO_PREDICT | NO_STEP);
        }
    }
    // E210: each of those again with every string written out
    if o.literal {
        let with: Vec<u8> = tries.iter().map(|m| m | NO_DICT).collect();
        tries.extend(with);
    }
    if tries.len() == 1 {
        return modes;
    }
    // per stream, the best mode among the tries without NO_DICT ([0]) and
    // among all of them ([1])
    let mut best: [FxMap<StreamKey, (usize, u8)>; 2] = Default::default();
    let mut base = None;
    for &m in &tries {
        let p = encode_modes(files, o, plain, (&Modes::default(), m));
        for (k, s) in directory(&p)
            .unwrap_or_default()
            .into_iter()
            .zip(&p.streams)
        {
            let z = judged_bytes(s);
            for b in &mut best[usize::from(m & NO_DICT != 0)..] {
                let e = b.entry(k.clone()).or_insert((usize::MAX, 0));
                if z < e.0 {
                    *e = (z, m);
                }
            }
        }
        if m == 0 {
            base = Some(p);
        }
    }
    // streams judged alone can mislead about the whole pack, so the whole
    // pack decides among no tuning and each set of picks
    let untuned = base.map_or(usize::MAX, |p| judged_bytes(&pack(&p)));
    let mut picked: Vec<Modes> = Vec::new();
    for b in best {
        let m: Modes = b
            .into_iter()
            .filter(|(_, (_, m))| *m != 0)
            .map(|(k, (_, m))| (k, m))
            .collect();
        if !m.is_empty() && !picked.contains(&m) {
            picked.push(m);
        }
    }
    let mut keep = (untuned, Modes::default());
    for m in picked {
        let z = judged_bytes(&pack(&encode_modes(files, o, plain, (&m, 0))));
        if z < keep.0 {
            keep = (z, m);
        }
    }
    keep.1
}

/// The xz preset that judges `route`'s choices (E208 first judged with
/// zstd -9, which disagreed with xz -9 on which pack was smaller).
const ROUTE_PRESET: u32 = 6;

fn judged_bytes(b: &[u8]) -> usize {
    use std::io::Write;
    use xz2::stream::{Check, Filters, LzmaOptions, Stream};
    // the preset's settings with a window no larger than the input (the
    // same output for inputs under it, and far less to set up for small
    // ones)
    let Ok(mut opts) = LzmaOptions::new_preset(ROUTE_PRESET) else {
        return usize::MAX;
    };
    let window = b.len().next_power_of_two().clamp(4096, 8 << 20);
    opts.dict_size(u32::try_from(window).unwrap_or(8 << 20));
    let mut filters = Filters::new();
    filters.lzma2(&opts);
    let Ok(stream) = Stream::new_stream_encoder(&filters, Check::None) else {
        return usize::MAX;
    };
    let mut e = xz2::write::XzEncoder::new_stream(Vec::new(), stream);
    if e.write_all(b).is_err() {
        return usize::MAX;
    }
    e.finish().map_or(usize::MAX, |z| z.len())
}

/// The judged size of the pack with `plain` stored as it is; with `tune`,
/// of the pack as tuned (E209), so `route` weighs the IR at its best.
fn judged(files: &[(String, Vec<u8>)], o: Options, plain: &[u8]) -> (usize, Modes) {
    let modes = tune(files, o, plain);
    let z = judged_bytes(&pack(&encode_modes(files, o, plain, (&modes, 0))));
    (z, modes)
}

/// E208: which files `route` stores as they are. Files are grouped by kind:
/// extension and the skeleton of their first line, so reports from one
/// template are one group and a folder's prose of the same extension is
/// another (groups of one file are pooled per extension). Each group is
/// judged on its own, its IR against its files stored plain, and goes the
/// smaller way; then the whole set so routed is judged against all IR and
/// all plain, and the smallest of the three is kept.
pub fn route(files: &[(String, Vec<u8>)], o: Options) -> Vec<bool> {
    route_modes(files, o)
        .0
        .into_iter()
        .map(|p| p & PLAIN != 0)
        .collect()
}

/// E216: which files `route` stores through the IR with no base file.
pub fn unbased(files: &[(String, Vec<u8>)], o: Options) -> Vec<bool> {
    route_modes(files, o)
        .0
        .into_iter()
        .map(|p| p & UNBASED != 0)
        .collect()
}

/// `route`, with the stream modes `tune` chose for the files so routed.
fn route_modes(files: &[(String, Vec<u8>)], o: Options) -> (Vec<u8>, Modes) {
    let n = files.len();
    if !o.route || n == 0 {
        let plain = vec![0; n];
        let (_, s) = sample(files, o.sample);
        let modes = tune(&s, o, &[]);
        return (plain, modes);
    }
    let keys: Vec<(&str, Option<Vec<u8>>)> = files
        .iter()
        .map(|(name, b)| {
            let first = lines(b, &lex(name, b))
                .first()
                .map(|l| skeleton_bytes(&l.segs));
            (ext(name), first)
        })
        .collect();
    let mut count: HashMap<&(&str, Option<Vec<u8>>), usize> = HashMap::new();
    for k in &keys {
        *count.entry(k).or_default() += 1;
    }
    let mut groups: HashMap<(&str, Option<&[u8]>), Vec<usize>> = HashMap::new();
    for (i, k) in keys.iter().enumerate() {
        let key = if count[k] > 1 {
            (k.0, Some(k.1.as_deref().unwrap_or(&[])))
        } else {
            (k.0, None)
        };
        groups.entry(key).or_default().push(i);
    }
    let mut plain = vec![0; n];
    let mut last = Modes::default();
    let ways: &[u8] = if o.bases && (o.siblings || o.nearest > 0) {
        &[PLAIN, UNBASED]
    } else {
        &[PLAIN]
    };
    for members in groups.values() {
        let g: Vec<(String, Vec<u8>)> = members.iter().map(|&i| files[i].clone()).collect();
        let (_, g) = sample(&g, o.sample);
        let mut best = judged(&g, o, &[]);
        let mut way = 0;
        for &w in ways {
            let (z, modes) = judged(&g, o, &vec![w; g.len()]);
            if z < best.0 {
                best = (z, modes);
                way = w;
            }
        }
        for &i in members {
            plain[i] = way;
        }
        if way != PLAIN {
            last = best.1;
        }
    }
    // one group: its judgment was already of the whole set
    if groups.len() == 1 {
        if plain.iter().all(|p| p & PLAIN != 0) {
            last = Modes::default();
        }
        return (plain, last);
    }
    let mut tries = vec![plain, vec![0; n], vec![PLAIN; n]];
    tries.dedup();
    let (kept, s) = sample(files, o.sample);
    tries
        .into_iter()
        .map(|p| {
            let sp: Vec<u8> = kept.iter().map(|&i| p[i]).collect();
            let (z, m) = judged(&s, o, &sp);
            (z, p, m)
        })
        .min_by_key(|(z, _, _)| *z)
        .map(|(_, p, m)| (p, m))
        .unwrap_or_default()
}

/// E214: what `route` and `tune` judge when `budget` is set (bytes, 0 is
/// everything): the index of each file kept and its bytes. Every file is
/// kept, since the IR's gain is across files (a first try that kept every
/// `r`th file routed drift reports plain); a file over its share of the
/// budget (and over 4 KB) keeps its first line and an even spread of the
/// others, so only large files are cut.
fn sample(files: &[(String, Vec<u8>)], budget: usize) -> (Vec<usize>, Vec<(String, Vec<u8>)>) {
    let total: usize = files.iter().map(|(_, b)| b.len()).sum();
    let kept = (0..files.len()).collect();
    if budget == 0 || total <= budget {
        return (kept, files.to_vec());
    }
    let out = files
        .iter()
        .map(|(name, b)| {
            let share = (b.len() as u128 * budget as u128 / total as u128) as usize;
            if b.len() <= share.max(4096) {
                return (name.clone(), b.clone());
            }
            let m = b.len().div_ceil(share.max(4096));
            let mut it = b.split_inclusive(|&c| c == b'\n');
            let mut cut = it.next().unwrap_or_default().to_vec();
            for l in it.step_by(m) {
                cut.extend_from_slice(l);
            }
            (name.clone(), cut)
        })
        .collect();
    (kept, out)
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
pub type StreamKey = (usize, usize, Vec<u8>);

#[derive(Clone, Default)]
struct EncState {
    index: FxMap<StreamKey, usize>,
    streams: Vec<Stream>,
}

/// `encode` against a skeleton table that outlives it, going on from the
/// streams in `state`: the returned parts have no skeletons of their own,
/// and their directory names only the streams this encode added.
fn encode_with(
    files: &[(String, Vec<u8>)],
    o: Options,
    plain: &[u8],
    (modes, modes_all): (&Modes, u8),
    sorted: &[Sorted],
    table: &mut Skeletons,
    state: &mut EncState,
) -> Parts {
    let sorted_any = sorted.iter().any(Option::is_some);
    let mut parts = Parts::default();
    let EncState { index, streams } = state;
    let first_new = streams.len();
    let lexed: Vec<Vec<Line>> = files.iter().map(|(n, b)| lines(b, &lex(n, b))).collect();
    // prose: under a tenth of the file's bytes are cells (E198 measured 2 to
    // 5% for prose, 14% and up for reports and tables)
    let raw: Vec<bool> = files
        .iter()
        .zip(&lexed)
        .enumerate()
        .map(|(i, ((_, b), fl))| {
            let cells: usize = fl.iter().flat_map(|l| &l.cells).map(|c| c.len()).sum();
            plain.get(i).is_some_and(|p| p & PLAIN != 0)
                || (o.prose && !b.is_empty() && cells * 10 < b.len())
        })
        .collect();
    let mut all_cells: Vec<Vec<&[u8]>> = Vec::new();
    let mut by_stem: HashMap<&str, usize> = HashMap::new();
    let mut by_ext: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut cell_sets: Vec<HashSet<&[u8]>> = Vec::new();
    parts.files.push(
        u8::from(o.columns)
            | u8::from(o.rows) << 1
            | u8::from(o.prose || o.route) << 2
            | u8::from(o.predict && o.columns) << 3
            | u8::from(o.step && o.predict && o.columns) << 4
            | u8::from(sorted_any) << 5
            | u8::from(o.transpose) << 6,
    );
    put_var(&mut parts.files, files.len() as u128);
    for (fi, ((name, b), ls)) in files.iter().zip(lexed).enumerate() {
        put_bytes(&mut parts.files, name.as_bytes());
        if o.prose || o.route || sorted_any || o.transpose {
            let st = sorted.get(fi).copied().flatten();
            let tr = (raw[fi] && o.transpose && ext(name) == "csv")
                .then(|| table_lines(b))
                .flatten();
            parts.files.push(
                u8::from(raw[fi]) | u8::from(st.is_some()) << 1 | u8::from(tr.is_some()) << 2,
            );
            if let Some((rows, trailing)) = st {
                put_var(&mut parts.files, rows as u128);
                parts.files.push(u8::from(trailing));
            }
            if let Some((header, lines, trailing)) = &tr {
                put_var(&mut parts.files, lines.len() as u128);
                parts.files.push(u8::from(*trailing));
                let t = transposed(header, lines);
                put_var(&mut parts.files, t.len() as u128);
                parts.raw.extend_from_slice(&t);
                all_cells.push(Vec::new());
                cell_sets.push(HashSet::new());
                continue;
            }
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
        let based = plain.get(fi).is_none_or(|p| p & UNBASED == 0);
        let sibling = by_stem
            .get(stem(name))
            .copied()
            .filter(|_| o.siblings && based);
        let recent = by_ext
            .get(ext(name))
            .filter(|_| based)
            .map_or(&[][..], |v: &Vec<usize>| {
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
            let mode = modes.get(&key).copied().unwrap_or(modes_all);
            streams[si].put_in(c, o.numbers, line, o.step && o.predict, mode);
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
    index: FxMap<StreamKey, usize>,
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
        line: Option<&[Vec<u8>]>,
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
        st.get_in(&self.streams[i], &mut self.pos[i], line, self.o.step)
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
        step: flags & 16 != 0,
        ..Options::default()
    };
    let sorted_any = flags & 32 != 0;
    let transpose = flags & 64 != 0;
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
        let mut st = None;
        if o.prose || sorted_any || transpose {
            let kind = *f.get(at).ok_or(Error::Decode("ir: truncated"))?;
            at += 1;
            if kind > 7
                || (kind & 2 != 0 && !sorted_any)
                || (kind & 4 != 0 && (!transpose || kind & 1 == 0))
            {
                return Err(Error::Decode("ir: file kind"));
            }
            if kind & 2 != 0 {
                let rows = get_usize(f, &mut at)?;
                let trailing = *f.get(at).ok_or(Error::Decode("ir: truncated"))? != 0;
                at += 1;
                st = Some((rows, trailing));
            }
            let mut tr = None;
            if kind & 4 != 0 {
                let lines = get_usize(f, &mut at)?;
                let trailing = *f.get(at).ok_or(Error::Decode("ir: truncated"))? != 0;
                at += 1;
                tr = Some((lines, trailing));
            }
            if kind & 1 != 0 {
                let len = get_usize(f, &mut at)?;
                let bytes = parts
                    .raw
                    .get(raw_at..raw_at.saturating_add(len))
                    .ok_or(Error::Decode("ir: raw"))?;
                raw_at += len;
                all_cells.push(Vec::new());
                let bytes = match tr {
                    Some((lines, trailing)) => untransposed(bytes, lines, trailing)?,
                    None => bytes.to_vec(),
                };
                let bytes = match st {
                    Some((rows, trailing)) => unsort_table(&bytes, rows, trailing)?,
                    None => bytes,
                };
                out.push((name, bytes));
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
                    let line = &cells[first..k];
                    let key = line.first().map_or(&[][..], Vec::as_slice);
                    cur.next(slot, key, o.predict.then_some(line))?
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
        let bytes = match st {
            Some((rows, trailing)) => unsort_table(&bytes, rows, trailing)?,
            None => bytes,
        };
        out.push((name, bytes));
    }
    Ok(out)
}

// ---------------------------------------------------------------- E211 rows

/// The fewest rows a table needs before `sort` tries an order for it.
const SORT_MIN_ROWS: usize = 64;
/// The rows of a table `sort` judges each candidate key on.
const SORT_SAMPLE: usize = 4000;

/// A CSV table's lines: header, rows, and whether the file ends in a
/// newline. None when a quoted field could span lines (an odd number of
/// quotes on some line) or there are too few rows.
/// A table's header line, its rows, and whether it ends in a newline.
type Table<'a> = (&'a [u8], Vec<&'a [u8]>, bool);

fn table_lines(b: &[u8]) -> Option<Table<'_>> {
    let trailing = b.last() == Some(&b'\n');
    let body = if trailing { &b[..b.len() - 1] } else { b };
    let mut it = body.split(|&c| c == b'\n');
    let header = it.next()?;
    let rows: Vec<&[u8]> = it.collect();
    if rows.len() < SORT_MIN_ROWS
        || std::iter::once(header)
            .chain(rows.iter().copied())
            .any(|l| l.iter().filter(|&&c| c == b'"').count() % 2 == 1)
    {
        return None;
    }
    Some((header, rows, trailing))
}

/// The `k`th field of a CSV line, as written (quotes included); empty when
/// the line has fewer fields.
fn field(line: &[u8], k: usize) -> &[u8] {
    let (mut start, mut n, mut quoted) = (0, 0, false);
    for (i, &c) in line.iter().enumerate() {
        match c {
            b'"' => quoted = !quoted,
            b',' if !quoted => {
                if n == k {
                    return &line[start..i];
                }
                n += 1;
                start = i + 1;
            }
            _ => {}
        }
    }
    if n == k { &line[start..] } else { &[] }
}

/// The rows' order once stably sorted by their keys.
fn sorted_order(keys: &[&[u8]]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..keys.len()).collect();
    order.sort_by(|&a, &b| keys[a].cmp(keys[b]));
    order
}

/// The table rewritten for column `k`: header, the rows sorted by their
/// `k`th field, then that field of every row in file order, one per line,
/// so the order needs no permutation stored. Every line ends in a newline.
fn sorted_table(header: &[u8], rows: &[&[u8]], k: usize) -> Vec<u8> {
    let keys: Vec<&[u8]> = rows.iter().map(|r| field(r, k)).collect();
    let mut out = Vec::with_capacity(rows.iter().map(|r| r.len() + 1).sum::<usize>() * 11 / 10);
    out.extend_from_slice(header);
    out.push(b'\n');
    for i in sorted_order(&keys) {
        out.extend_from_slice(rows[i]);
        out.push(b'\n');
    }
    for k in keys {
        out.extend_from_slice(k);
        out.push(b'\n');
    }
    out
}

/// Each column of the first `SORT_SAMPLE` rows, one after another: what
/// `sort` judges an order by, since the IR stores a table by column.
fn by_column(header: &[u8], rows: &[&[u8]], cols: usize) -> Vec<u8> {
    let mut out = header.to_vec();
    for k in 0..cols {
        for r in rows {
            out.extend_from_slice(field(r, k));
            out.push(b'\n');
        }
    }
    out
}

/// E211: the column to sort a CSV table's rows by, if any order judges
/// smaller by column than the file's own.
fn sort_key(b: &[u8]) -> Option<usize> {
    let (header, rows, _) = table_lines(b)?;
    let cols = field_count(header);
    let sample = &rows[..rows.len().min(SORT_SAMPLE)];
    let mut best = (judged_bytes(&by_column(header, sample, cols)), None);
    for k in 0..cols {
        let keys: Vec<&[u8]> = sample.iter().map(|r| field(r, k)).collect();
        let mut distinct: Vec<&[u8]> = keys.clone();
        distinct.sort_unstable();
        distinct.dedup();
        // a key that is near unique or constant buys no runs
        if distinct.len() < 2 || distinct.len() * 4 > keys.len() {
            continue;
        }
        let order = sorted_order(&keys);
        let sorted: Vec<&[u8]> = order.iter().map(|&i| sample[i]).collect();
        let mut t = by_column(header, &sorted, cols);
        for key in &keys {
            t.extend_from_slice(key);
            t.push(b'\n');
        }
        let z = judged_bytes(&t);
        // a hundredth smaller at least, so noise picks no order
        if z * 100 < best.0 * 99 {
            best = (z, Some(k));
        }
    }
    best.1
}

fn field_count(line: &[u8]) -> usize {
    let mut quoted = false;
    1 + line
        .iter()
        .filter(|&&c| {
            if c == b'"' {
                quoted = !quoted;
            }
            c == b',' && !quoted
        })
        .count()
}

/// The table `sorted_table` rewrote, given its row count and whether the
/// file ended in a newline.
fn unsort_table(t: &[u8], rows: usize, trailing: bool) -> Result<Vec<u8>, Error> {
    let body = t
        .strip_suffix(b"\n")
        .ok_or(Error::Decode("ir: sorted table"))?;
    let lines: Vec<&[u8]> = body.split(|&c| c == b'\n').collect();
    if lines.len() != 1 + 2 * rows {
        return Err(Error::Decode("ir: sorted table lines"));
    }
    let (sorted, keys) = lines[1..].split_at(rows);
    let order = sorted_order(keys);
    let mut back: Vec<&[u8]> = vec![&[]; rows];
    for (j, &i) in order.iter().enumerate() {
        back[i] = sorted[j];
    }
    let mut out = Vec::with_capacity(t.len());
    out.extend_from_slice(lines[0]);
    for r in back {
        out.push(b'\n');
        out.extend_from_slice(r);
    }
    if trailing {
        out.push(b'\n');
    }
    Ok(out)
}

// ---------------------------------------------------------------- E212

/// E212: a table's lines laid out by column, for storing plain: the
/// header, each line's field count, then each column's fields, one per
/// line (a line with fewer fields has none in the later columns). Lines
/// are rebuilt by joining their fields with commas, so it is exact.
/// E213: after the header, one byte per column, `q` where every field of
/// the column is quoted (the quotes are left out of its fields) and `-`
/// elsewhere, since an export often quotes every text field.
fn transposed(header: &[u8], lines: &[&[u8]]) -> Vec<u8> {
    let counts: Vec<usize> = lines.iter().map(|l| field_count(l)).collect();
    let mut out =
        Vec::with_capacity(lines.iter().map(|l| l.len() + 2).sum::<usize>() + header.len());
    out.extend_from_slice(header);
    out.push(b'\n');
    let fields: Vec<Vec<&[u8]>> = lines.iter().map(|l| fields(l)).collect();
    let most = counts.iter().copied().max().unwrap_or(0);
    let quoted: Vec<bool> = (0..most)
        .map(|k| {
            fields
                .iter()
                .filter_map(|f| f.get(k))
                .all(|v| v.len() >= 2 && v[0] == b'"' && v[v.len() - 1] == b'"')
        })
        .collect();
    out.extend(quoted.iter().map(|&q| if q { b'q' } else { b'-' }));
    out.push(b'\n');
    for c in &counts {
        out.extend_from_slice(c.to_string().as_bytes());
        out.push(b'\n');
    }
    for (k, &q) in quoted.iter().enumerate() {
        for f in &fields {
            if let Some(v) = f.get(k) {
                out.extend_from_slice(if q { &v[1..v.len() - 1] } else { v });
                out.push(b'\n');
            }
        }
    }
    out
}

/// A CSV line's fields as written (quotes included).
fn fields(line: &[u8]) -> Vec<&[u8]> {
    let (mut out, mut start, mut quoted) = (Vec::new(), 0, false);
    for (i, &c) in line.iter().enumerate() {
        match c {
            b'"' => quoted = !quoted,
            b',' if !quoted => {
                out.push(&line[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&line[start..]);
    out
}

/// The table `transposed` laid out, given its line count and whether it
/// ended in a newline.
fn untransposed(t: &[u8], n: usize, trailing: bool) -> Result<Vec<u8>, Error> {
    let bad = || Error::Decode("ir: transposed table");
    let mut it = t.split(|&c| c == b'\n');
    let header = it.next().ok_or_else(bad)?;
    let quoted = it.next().ok_or_else(bad)?;
    if quoted.iter().any(|&c| c != b'q' && c != b'-') {
        return Err(bad());
    }
    let counts: Vec<usize> = (0..n)
        .map(|_| {
            let c = it.next().ok_or_else(bad)?;
            std::str::from_utf8(c)
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|&c: &usize| c >= 1 && c <= t.len())
                .ok_or_else(bad)
        })
        .collect::<Result<_, _>>()?;
    let mut lines: Vec<Vec<u8>> = vec![Vec::new(); n];
    let most = counts.iter().copied().max().unwrap_or(0);
    if quoted.len() != most {
        return Err(bad());
    }
    for (k, &q) in quoted.iter().enumerate() {
        for (l, &c) in lines.iter_mut().zip(&counts) {
            if c > k {
                if k > 0 {
                    l.push(b',');
                }
                let v = it.next().ok_or_else(bad)?;
                if q == b'q' {
                    l.push(b'"');
                    l.extend_from_slice(v);
                    l.push(b'"');
                } else {
                    l.extend_from_slice(v);
                }
            }
        }
    }
    // what is left is the empty piece after the last newline
    if it.next() != Some(&[][..]) || it.next().is_some() {
        return Err(bad());
    }
    let mut out = Vec::with_capacity(t.len());
    out.extend_from_slice(header);
    for l in lines {
        out.push(b'\n');
        out.extend_from_slice(&l);
    }
    if trailing {
        out.push(b'\n');
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
    let (files, sorted) = sort_files(files, o);
    let files = files.as_slice();
    let (plain, modes) = route_modes(files, o);
    let mut table = Skeletons::default();
    let mut key = EncState::default();
    let mut blocks = Vec::new();
    for (i, ((c, p), so)) in files
        .chunks(per)
        .zip(plain.chunks(per))
        .zip(sorted.chunks(per))
        .enumerate()
    {
        let mut state = if keyframe && i > 0 {
            key.clone()
        } else {
            EncState::default()
        };
        blocks.push(encode_with(
            c,
            o,
            p,
            (&modes, 0),
            so,
            &mut table,
            &mut state,
        ));
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

    /// E209: every stream mode, on every stream, decodes exactly, alone and
    /// in blocks, and mixed per stream.
    #[test]
    fn transposed_tables_come_back() {
        let lines: Vec<&[u8]> = vec![
            b"\"1\",\"a, b\",x,\"\"",
            b"\"2\",\"\"\"q\"\"\",\"y\",\"\"",
            b"\"3\"",
            b"\"4\",\"\",z,\"\",extra",
        ];
        let t = transposed(b"H1,H2,H3,H4", &lines);
        assert!(
            t.starts_with(b"H1,H2,H3,H4\nqq-q-\n"),
            "{}",
            String::from_utf8_lossy(&t)
        );
        for trailing in [false, true] {
            let back = untransposed(&t, lines.len(), trailing).unwrap();
            let mut want = b"H1,H2,H3,H4\n".to_vec();
            want.extend(lines.join(&b'\n'));
            if trailing {
                want.push(b'\n');
            }
            assert_eq!(back, want);
        }
        assert!(untransposed(&t[..t.len() - 1], lines.len(), false).is_err());
        assert!(untransposed(&t, lines.len() + 1, false).is_err());
    }

    #[test]
    fn every_stream_mode_decodes() {
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for r in 0..12u32 {
            let rows: String = (0..6u32)
                .map(|i| {
                    format!(
                        "l{i},{},{}.{:02},{}\n",
                        ["up", "down"][(i % 2) as usize],
                        r * 3 + i,
                        (r * 7 + i) % 100,
                        if r % 4 == 0 { "x" } else { "y" }
                    )
                })
                .collect();
            files.push((
                format!("r{r:02}.csv"),
                format!("id,dir,n,v,f\n{rows}").into_bytes(),
            ));
        }
        let o = Options {
            columns: true,
            numbers: true,
            predict: true,
            step: true,
            ..Options::default()
        };
        for rows in [false, true] {
            let o = Options { rows, ..o };
            for m in 0..16u8 {
                let p = encode_modes(&files, o, &[], (&Modes::default(), m));
                assert_eq!(decode(&p).unwrap(), files, "mode {m}");
            }
            // a different mode per stream
            let keys = directory(&encode(&files, o)).unwrap();
            let modes: Modes = keys
                .into_iter()
                .enumerate()
                .map(|(i, k)| (k, (i % 16) as u8))
                .collect();
            let p = encode_modes(&files, o, &[], (&modes, 0));
            assert_eq!(decode(&p).unwrap(), files);
            let mut table = Skeletons::default();
            let mut state = EncState::default();
            let mut blocks = Vec::new();
            for c in files.chunks(5) {
                blocks.push(encode_with(
                    c,
                    o,
                    &[],
                    (&modes, 0),
                    &[],
                    &mut table,
                    &mut state.clone(),
                ));
                if blocks.len() == 1 {
                    state = EncState::default();
                }
            }
            let b = Blocks {
                per: 5,
                keyframe: false,
                skeletons: table.list,
                blocks,
            };
            assert_eq!(decode_blocks(&b).unwrap(), files);
        }
    }
}
