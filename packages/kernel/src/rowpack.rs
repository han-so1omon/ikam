//! Rowpack (E402), after Kumo's DiskGraph row codec: a context trained on a
//! sample of small rows (their shapes, and for each value slot a dictionary
//! of frequent values, a fixed digit width and templates of strings with
//! numbers in them), and each row then encoded on its own against it, with
//! two-bit tags per slot and a raw fallback for any value the context does
//! not know. No general-purpose compressor runs on a read.
//!
//! A row is cut into literals and slots: a JSON string in value position
//! (one not followed by `:`) is a string slot, and every other run of ASCII
//! digits outside strings is a digit slot; the rest, keys and punctuation
//! and any bytes that are not JSON, is literal. The literals and the kind of
//! each slot are the row's shape. A row whose shape is not in the context
//! cannot be encoded (`encode` returns `None`), so a caller keeps its usual
//! encoding.
//!
//! Byte-exact by construction: literals are kept as they are, a digit slot
//! is encoded as a number only when formatting the number back (at the
//! column's width, if it has one) gives its exact text, and `encode` decodes
//! its own output and refuses to return a row that does not give back the
//! bytes.
//!
//! Measured only (`examples/rowpack.rs`): nothing in the kernel stores rows
//! this way.

use std::collections::HashMap;

use crate::Error;

const MAGIC: &[u8] = b"ikam-rowpack/1\0";
/// Most values a column's dictionary holds (an index is one byte).
const DICT_MAX: usize = 255;
/// Most string templates a column holds (an index is one byte).
const TEMPLATES_MAX: usize = 255;
/// Most shapes a context holds.
const SHAPES_MAX: usize = 4096;

const T_DICT: u8 = 0;
const T_NUM: u8 = 1;
const T_TMPL: u8 = 2;
const T_RAW: u8 = 3;

const DIGITS: u8 = 0;
const STRING: u8 = 1;

fn put_varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 0x80 {
        out.push(n as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

fn varint(b: &[u8], at: &mut usize) -> Option<u64> {
    let mut n = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*at)?;
        *at += 1;
        n |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(n);
        }
    }
    None
}

fn bad() -> Error {
    Error::Decode("rowpack")
}

/// A row cut into `literals` (one more than `slots`) and slots.
struct Cut<'a> {
    literals: Vec<&'a [u8]>,
    slots: Vec<(u8, &'a [u8])>,
}

/// The end (index of the closing quote) of a JSON string opening at `at`.
fn string_end(row: &[u8], at: usize) -> Option<usize> {
    let mut i = at + 1;
    while i < row.len() {
        match row[i] {
            b'\\' => i += 2,
            b'"' => return Some(i),
            _ => i += 1,
        }
    }
    None
}

fn cut(row: &[u8]) -> Cut<'_> {
    let (mut literals, mut slots) = (Vec::new(), Vec::new());
    let (mut start, mut i) = (0, 0);
    while i < row.len() {
        match row[i] {
            b'"' => {
                let Some(end) = string_end(row, i) else {
                    break; // an unclosed string: the rest is literal
                };
                let mut k = end + 1;
                while k < row.len() && row[k] == b' ' {
                    k += 1;
                }
                if k < row.len() && row[k] == b':' {
                    i = end + 1; // a key: literal
                    continue;
                }
                literals.push(&row[start..i + 1]);
                slots.push((STRING, &row[i + 1..end]));
                start = end;
                i = end + 1;
            }
            b'0'..=b'9' => {
                let mut j = i;
                while j < row.len() && row[j].is_ascii_digit() {
                    j += 1;
                }
                literals.push(&row[start..i]);
                slots.push((DIGITS, &row[i..j]));
                start = j;
                i = j;
            }
            _ => i += 1,
        }
    }
    literals.push(&row[start..]);
    Cut { literals, slots }
}

/// The key a shape is found by: each literal and slot kind, length-prefixed.
fn shape_key(c: &Cut) -> Vec<u8> {
    let mut k = Vec::new();
    for (i, l) in c.literals.iter().enumerate() {
        put_varint(&mut k, l.len() as u64);
        k.extend_from_slice(l);
        if let Some((kind, _)) = c.slots.get(i) {
            k.push(*kind);
        }
    }
    k
}

/// The number whose text at `width` (0: canonical, no leading zero) is
/// exactly `digits`.
fn as_num(digits: &[u8], width: usize) -> Option<u64> {
    if digits.len() > 19 || (width == 0 && digits.len() > 1 && digits[0] == b'0') {
        return None;
    }
    if width != 0 && digits.len() != width {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}

fn put_num(out: &mut Vec<u8>, mut n: u64, width: usize) {
    let mut buf = [b'0'; 20];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    let start = buf.len() - width.max(buf.len() - i).min(buf.len());
    out.extend_from_slice(&buf[start..]);
}

/// A string with its digit runs taken out: (literal parts, digit widths, 0
/// where the run's width varied in the sample).
#[derive(Clone, Default)]
struct Template {
    parts: Vec<Vec<u8>>,
    widths: Vec<usize>,
}

/// `s` cut into the literal parts between its digit runs, and the runs.
fn split_digits(s: &[u8]) -> (Vec<&[u8]>, Vec<&[u8]>) {
    let (mut parts, mut runs, mut start, mut i) = (Vec::new(), Vec::new(), 0, 0);
    while i < s.len() {
        if s[i].is_ascii_digit() {
            let mut j = i;
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
            }
            parts.push(&s[start..i]);
            runs.push(&s[i..j]);
            start = j;
            i = j;
        } else {
            i += 1;
        }
    }
    parts.push(&s[start..]);
    (parts, runs)
}

#[derive(Default)]
struct Column {
    dict: Vec<Vec<u8>>,
    index: HashMap<Vec<u8>, u8>,
    /// For a digit slot: its width if every sampled value had one, else 0.
    width: usize,
    templates: Vec<Template>,
    by_parts: HashMap<Vec<Vec<u8>>, u8>,
}

struct Shape {
    literals: Vec<Vec<u8>>,
    kinds: Vec<u8>,
    cols: Vec<Column>,
}

/// What rows of one or more tables share, trained on a sample.
pub struct Context {
    shapes: Vec<Shape>,
    by_key: HashMap<Vec<u8>, usize>,
}

impl Context {
    /// Train on `sample`: every shape seen at least twice (the most common
    /// first, up to `SHAPES_MAX`), and for each of its slots the values seen
    /// at least twice, a digit width all sampled values share, and string
    /// templates seen at least twice.
    pub fn train(sample: &[&[u8]]) -> Context {
        let mut groups: HashMap<Vec<u8>, Vec<Cut>> = HashMap::new();
        for row in sample {
            let c = cut(row);
            groups.entry(shape_key(&c)).or_default().push(c);
        }
        let mut groups: Vec<(Vec<u8>, Vec<Cut>)> =
            groups.into_iter().filter(|g| g.1.len() >= 2).collect();
        groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
        groups.truncate(SHAPES_MAX);
        let mut shapes = Vec::new();
        for (_, cuts) in &groups {
            let first = &cuts[0];
            let kinds: Vec<u8> = first.slots.iter().map(|s| s.0).collect();
            let mut cols = Vec::new();
            for (s, kind) in kinds.iter().enumerate() {
                let values: Vec<&[u8]> = cuts.iter().map(|c| c.slots[s].1).collect();
                cols.push(train_column(*kind, &values));
            }
            shapes.push(Shape {
                literals: first.literals.iter().map(|l| l.to_vec()).collect(),
                kinds,
                cols,
            });
        }
        Context::from_shapes(shapes)
    }

    fn from_shapes(shapes: Vec<Shape>) -> Context {
        let by_key = shapes
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let literals: Vec<&[u8]> = s.literals.iter().map(|l| &l[..]).collect();
                let slots = s.kinds.iter().map(|k| (*k, &b""[..])).collect();
                (shape_key(&Cut { literals, slots }), i)
            })
            .collect();
        Context { shapes, by_key }
    }

    pub fn shapes(&self) -> usize {
        self.shapes.len()
    }

    /// `row` against this context, if its shape is known and the encoding
    /// gives the row back.
    pub fn encode(&self, row: &[u8]) -> Option<Vec<u8>> {
        let c = cut(row);
        let s = *self.by_key.get(&shape_key(&c))?;
        let shape = &self.shapes[s];
        let mut out = Vec::new();
        put_varint(&mut out, s as u64);
        let tags_at = out.len();
        out.resize(tags_at + c.slots.len().div_ceil(4), 0);
        for (i, (kind, v)) in c.slots.iter().enumerate() {
            let col = &shape.cols[i];
            let tag = if let Some(ix) = col.index.get(*v) {
                out.push(*ix);
                T_DICT
            } else if *kind == DIGITS
                && let Some(n) = as_num(v, col.width)
            {
                put_varint(&mut out, n);
                T_NUM
            } else if *kind == STRING
                && let Some(t) = template_payload(col, v)
            {
                out.extend(t);
                T_TMPL
            } else {
                put_varint(&mut out, v.len() as u64);
                out.extend_from_slice(v);
                T_RAW
            };
            out[tags_at + i / 4] |= tag << (2 * (i % 4));
        }
        (self.decode(&out).ok()? == row).then_some(out)
    }

    /// The row `encoded` (from `encode`) stands for.
    pub fn decode(&self, encoded: &[u8]) -> Result<Vec<u8>, Error> {
        let mut at = 0;
        let s = varint(encoded, &mut at).ok_or_else(bad)? as usize;
        let shape = self.shapes.get(s).ok_or_else(bad)?;
        let n = shape.kinds.len();
        let tags = encoded.get(at..at + n.div_ceil(4)).ok_or_else(bad)?;
        at += n.div_ceil(4);
        let mut out = Vec::with_capacity(encoded.len() * 3);
        for (i, lit) in shape.literals.iter().enumerate() {
            out.extend_from_slice(lit);
            if i == n {
                break;
            }
            let col = &shape.cols[i];
            match (tags[i / 4] >> (2 * (i % 4))) & 3 {
                T_DICT => {
                    let ix = *encoded.get(at).ok_or_else(bad)? as usize;
                    at += 1;
                    out.extend_from_slice(col.dict.get(ix).ok_or_else(bad)?);
                }
                T_NUM => put_num(
                    &mut out,
                    varint(encoded, &mut at).ok_or_else(bad)?,
                    col.width,
                ),
                T_TMPL => {
                    let t = col
                        .templates
                        .get(*encoded.get(at).ok_or_else(bad)? as usize);
                    let t = t.ok_or_else(bad)?;
                    at += 1;
                    for (j, part) in t.parts.iter().enumerate() {
                        out.extend_from_slice(part);
                        if let Some(w) = t.widths.get(j) {
                            put_num(&mut out, varint(encoded, &mut at).ok_or_else(bad)?, *w);
                        }
                    }
                }
                _ => {
                    let len = varint(encoded, &mut at).ok_or_else(bad)? as usize;
                    out.extend_from_slice(encoded.get(at..at + len).ok_or_else(bad)?);
                    at += len;
                }
            }
        }
        (at == encoded.len()).then_some(out).ok_or_else(bad)
    }

    /// The context as bytes (`from_bytes` reads them back), so its stored
    /// size can be counted.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        let bytes = |out: &mut Vec<u8>, b: &[u8]| {
            put_varint(out, b.len() as u64);
            out.extend_from_slice(b);
        };
        put_varint(&mut out, self.shapes.len() as u64);
        for s in &self.shapes {
            put_varint(&mut out, s.kinds.len() as u64);
            for l in &s.literals {
                bytes(&mut out, l);
            }
            for (k, c) in s.kinds.iter().zip(&s.cols) {
                out.push(*k);
                put_varint(&mut out, c.width as u64);
                put_varint(&mut out, c.dict.len() as u64);
                for v in &c.dict {
                    bytes(&mut out, v);
                }
                put_varint(&mut out, c.templates.len() as u64);
                for t in &c.templates {
                    put_varint(&mut out, t.widths.len() as u64);
                    for p in &t.parts {
                        bytes(&mut out, p);
                    }
                    for w in &t.widths {
                        put_varint(&mut out, *w as u64);
                    }
                }
            }
        }
        out
    }

    pub fn from_bytes(b: &[u8]) -> Result<Context, Error> {
        let mut at = b.strip_prefix(MAGIC).map(|_| MAGIC.len()).ok_or_else(bad)?;
        let num = |at: &mut usize| varint(b, at).map(|n| n as usize).ok_or_else(bad);
        let bytes = |at: &mut usize| -> Result<Vec<u8>, Error> {
            let n = varint(b, at).ok_or_else(bad)? as usize;
            let v = b.get(*at..*at + n).ok_or_else(bad)?.to_vec();
            *at += n;
            Ok(v)
        };
        let mut shapes = Vec::new();
        for _ in 0..num(&mut at)? {
            let n = num(&mut at)?;
            let literals = (0..=n).map(|_| bytes(&mut at)).collect::<Result<_, _>>()?;
            let (mut kinds, mut cols) = (Vec::new(), Vec::new());
            for _ in 0..n {
                kinds.push(*b.get(at).ok_or_else(bad)?);
                at += 1;
                let mut c = Column {
                    width: num(&mut at)?,
                    ..Column::default()
                };
                for _ in 0..num(&mut at)? {
                    c.dict.push(bytes(&mut at)?);
                }
                for _ in 0..num(&mut at)? {
                    let runs = num(&mut at)?;
                    let parts = (0..=runs)
                        .map(|_| bytes(&mut at))
                        .collect::<Result<_, _>>()?;
                    let widths = (0..runs).map(|_| num(&mut at)).collect::<Result<_, _>>()?;
                    c.templates.push(Template { parts, widths });
                }
                index_column(&mut c);
                cols.push(c);
            }
            shapes.push(Shape {
                literals,
                kinds,
                cols,
            });
        }
        (at == b.len())
            .then(|| Context::from_shapes(shapes))
            .ok_or_else(bad)
    }
}

fn index_column(c: &mut Column) {
    c.index = c
        .dict
        .iter()
        .enumerate()
        .map(|(i, v)| (v.clone(), i as u8))
        .collect();
    c.by_parts = c
        .templates
        .iter()
        .enumerate()
        .map(|(i, t)| (t.parts.clone(), i as u8))
        .collect();
}

/// A string slot's template index and digit values, if it fits one.
fn template_payload(col: &Column, v: &[u8]) -> Option<Vec<u8>> {
    let (parts, runs) = split_digits(v);
    if runs.is_empty() {
        return None;
    }
    let parts: Vec<Vec<u8>> = parts.iter().map(|p| p.to_vec()).collect();
    let ix = *col.by_parts.get(&parts)?;
    let t = &col.templates[ix as usize];
    let mut out = vec![ix];
    for (run, w) in runs.iter().zip(&t.widths) {
        put_varint(&mut out, as_num(run, *w)?);
    }
    Some(out)
}

fn train_column(kind: u8, values: &[&[u8]]) -> Column {
    let mut counts: HashMap<&[u8], usize> = HashMap::new();
    for v in values {
        *counts.entry(v).or_default() += 1;
    }
    // A value earns a dictionary entry if it repeats; the most repeated
    // first. Digit values that encode as one byte anyway gain nothing.
    let mut dict: Vec<(&[u8], usize)> = counts
        .iter()
        .filter(|(v, n)| **n >= 2 && v.len() > 1)
        .map(|(v, n)| (*v, *n))
        .collect();
    dict.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    dict.truncate(DICT_MAX);
    let mut c = Column {
        dict: dict.iter().map(|d| d.0.to_vec()).collect(),
        ..Column::default()
    };
    if kind == DIGITS {
        let w = values[0].len();
        if values.iter().all(|v| v.len() == w) && values.iter().any(|v| v.len() > 1 && v[0] == b'0')
        {
            c.width = w;
        }
    } else {
        // (count, each run's width if it never varied, whether it ever had
        // a leading zero)
        type Seen = (usize, Vec<Option<usize>>, Vec<bool>);
        let mut tmpl: HashMap<Vec<Vec<u8>>, Seen> = HashMap::new();
        for v in values {
            let (parts, runs) = split_digits(v);
            if runs.is_empty() {
                continue;
            }
            let e = tmpl
                .entry(parts.iter().map(|p| p.to_vec()).collect())
                .or_insert_with(|| {
                    (
                        0,
                        runs.iter().map(|r| Some(r.len())).collect(),
                        vec![false; runs.len()],
                    )
                });
            e.0 += 1;
            for ((w, z), r) in e.1.iter_mut().zip(e.2.iter_mut()).zip(&runs) {
                if *w != Some(r.len()) {
                    *w = None;
                }
                *z |= r.len() > 1 && r[0] == b'0';
            }
        }
        let mut tmpl: Vec<_> = tmpl.into_iter().filter(|t| t.1.0 >= 2).collect();
        tmpl.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
        tmpl.truncate(TEMPLATES_MAX);
        // A run keeps a width only if it never varied and was zero-padded;
        // otherwise its numbers are canonical (width 0).
        c.templates = tmpl
            .into_iter()
            .map(|(parts, (_, widths, padded))| Template {
                parts,
                widths: widths
                    .into_iter()
                    .zip(padded)
                    .map(|(w, z)| if z { w.unwrap_or(0) } else { 0 })
                    .collect(),
            })
            .collect();
    }
    index_column(&mut c);
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        for i in 0..500u64 {
            let r = format!(
                "{{\"commit\":\"{:040x}\",\"date\":\"2026-01-{:02}\",\"path\":\"src/f{}.rs\",\"added\":{},\"deleted\":{}}}",
                i * 7919,
                1 + i % 28,
                i % 37,
                i * 3,
                if i % 5 == 0 {
                    "null".to_string()
                } else {
                    (i % 9).to_string()
                }
            );
            out.push(r.into_bytes());
            out.push(
                format!(
                    "[\"2026-01-{:02}\",\"word{}\",{}.{},\"w{} w{}\"]",
                    1 + i % 28,
                    i % 50,
                    i,
                    i % 1000,
                    i,
                    i + 1
                )
                .into_bytes(),
            );
            let mut ev = b"ikam-event\0".to_vec();
            ev.extend_from_slice(&(5u32 + (i >= 10) as u32).to_be_bytes());
            ev.extend_from_slice(format!("e{i}").as_bytes());
            ev.extend_from_slice(format!("t/r/p{i:019}").as_bytes());
            out.push(ev);
        }
        // Odd rows: escapes, unclosed strings, leading zeros, huge numbers,
        // keys with digits, not JSON at all.
        for odd in [
            &br#"{"a\"b":"c\\\"d","n":007}"#[..],
            br#"{"x":"unclosed"#,
            br#"{"big":123456789012345678901234567890}"#,
            br#"{"k1":"v1","k2":"v2"}"#,
            b"\x00\x01\xff binary 12 34",
            b"",
            b"0",
            b"\"\"",
        ] {
            out.push(odd.to_vec());
            out.push(odd.to_vec());
        }
        out
    }

    #[test]
    fn rows_come_back_byte_for_byte() {
        let rows = rows();
        let sample: Vec<&[u8]> = rows.iter().step_by(2).map(|r| &r[..]).collect();
        let ctx = Context::train(&sample);
        let ctx = Context::from_bytes(&ctx.to_bytes()).unwrap();
        let mut encoded = 0;
        for r in &rows {
            if let Some(e) = ctx.encode(r) {
                assert_eq!(&ctx.decode(&e).unwrap(), r);
                encoded += 1;
            }
        }
        assert!(encoded > rows.len() * 9 / 10, "{encoded} of {}", rows.len());
        // A row of a shape it never saw is not encoded.
        assert!(ctx.encode(br#"{"never":"seen","shape":1}"#).is_none());
        // Damaged encodings fail rather than give other bytes silently.
        let e = ctx.encode(&rows[0]).unwrap();
        assert!(ctx.decode(&e[..e.len() - 1]).is_err());
        assert!(ctx.decode(&[e.clone(), vec![0]].concat()).is_err());
    }
}
