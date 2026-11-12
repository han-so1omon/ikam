//! Column blocks for large append-only tables (E241), after Apache Arrow's
//! columnar layout and its dictionary arrays: a block of rows is split
//! into fields at a separator byte, each field position becomes a column,
//! and each column is encoded by what its values are (canonical integers
//! and fixed-point decimals as zigzag deltas, few distinct values as a
//! dictionary, anything else as length-prefixed bytes) and compressed on
//! its own, so a scan of one column decodes only that column.
//!
//! Byte-exact by construction: splitting at a byte and joining at it is
//! lossless, a number is encoded as a number only if formatting it back
//! gives its exact text, and `encode` decodes its own output and refuses
//! to return a block that does not give back the rows.
//!
//! Opt-in: nothing in the kernel writes blocks unless a caller asks
//! (`Repo::put_column_block`, `Repo::db_append_block`).

use std::collections::HashMap;

use crate::group::{put_varint, varint};
use crate::{Error, Event, Id, Repo, Store};

const MAGIC: &[u8] = b"ikam-colblock/1\0";
/// zstd level for each column.
const LEVEL: i32 = 19;
/// Most distinct values a dictionary column holds.
const DICT_MAX: usize = 255;

const RAW: u8 = 0;
const INT: u8 = 1;
const DECIMAL: u8 = 2;
const DICT: u8 = 3;
/// E252: integers and decimals in a fast-field codec (after Tantivy's).
const FAST_INT: u8 = 4;
const FAST_DEC: u8 = 5;
/// On a stored column's kind: its payload is stored as is, no zstd frame,
/// so a value can be read without decoding the column.
const AS_IS: u8 = 0x80;
/// Rows a blockwise-linear block (Tantivy's `BLOCK_SIZE`).
const LINE_BLOCK: usize = 512;

/// Options for writing blocks (E252); the default is E241's encoding.
#[derive(Clone, Copy, Debug, Default)]
pub struct ColOpts {
    /// Integer and decimal columns in Tantivy's fast-field codecs: bitpacked
    /// with min and gcd, or blockwise linear, whichever is smaller.
    pub fast_numbers: bool,
    /// Compress those columns with zstd as the others are; off, they are
    /// stored as is and one value reads in constant time.
    pub compress_fast: bool,
}

fn width(v: u64) -> u8 {
    (64 - v.leading_zeros()) as u8
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

fn pack(out: &mut Vec<u8>, vals: impl Iterator<Item = u64>, bits: u8) {
    let (mut acc, mut held) = (0u128, 0u32);
    for v in vals {
        acc |= u128::from(v) << held;
        held += u32::from(bits);
        while held >= 8 {
            out.push(acc as u8);
            acc >>= 8;
            held -= 8;
        }
    }
    if held > 0 {
        out.push(acc as u8);
    }
}

/// Value `i` of `bits`-bit values packed from `b`.
fn unpack_at(b: &[u8], bits: u8, i: usize) -> u64 {
    if bits == 0 {
        return 0;
    }
    let pos = i * usize::from(bits);
    let mut w = [0u8; 16];
    let at = pos / 8;
    let end = (at + 16).min(b.len());
    w[..end - at].copy_from_slice(&b[at..end]);
    let v = u128::from_le_bytes(w) >> (pos % 8);
    (v & (u128::MAX >> (128 - u32::from(bits)))) as u64
}

fn le(b: &[u8], at: usize) -> Result<u64, Error> {
    Ok(u64::from_le_bytes(
        b.get(at..at + 8)
            .ok_or_else(bad)?
            .try_into()
            .map_err(|_| bad())?,
    ))
}

/// Bitpacked: `[0][min][gcd][bits][packed (v - min) / gcd]`.
fn fast_bitpacked(nums: &[i64]) -> Vec<u8> {
    let min = nums.iter().copied().min().unwrap_or(0);
    let offs: Vec<u64> = nums.iter().map(|&v| v.wrapping_sub(min) as u64).collect();
    let g = offs.iter().fold(0, |a, &b| gcd(a, b)).max(1);
    let bits = width(offs.iter().max().copied().unwrap_or(0) / g);
    let mut out = vec![0];
    out.extend_from_slice(&min.to_le_bytes());
    out.extend_from_slice(&g.to_le_bytes());
    out.push(bits);
    pack(&mut out, offs.iter().map(|o| o / g), bits);
    out
}

/// Blockwise linear: `[1]`, then a header a block (first value, slope,
/// residual minimum, bits, payload offset: 29 bytes), then each block's
/// residuals bitpacked against its line.
fn fast_linear(nums: &[i64]) -> Vec<u8> {
    let blocks: Vec<&[i64]> = nums.chunks(LINE_BLOCK).collect();
    let (mut heads, mut body) = (vec![1u8], Vec::new());
    for blk in &blocks {
        let (a, n) = (blk[0], blk.len() as i64);
        let s = if n > 1 {
            blk[blk.len() - 1].wrapping_sub(a) / (n - 1)
        } else {
            0
        };
        let res: Vec<i64> = blk
            .iter()
            .enumerate()
            .map(|(i, &v)| v.wrapping_sub(a.wrapping_add(s.wrapping_mul(i as i64))))
            .collect();
        let m = res.iter().copied().min().unwrap_or(0);
        let bits = width(
            res.iter()
                .map(|&r| r.wrapping_sub(m) as u64)
                .max()
                .unwrap_or(0),
        );
        for x in [a, s, m] {
            heads.extend_from_slice(&x.to_le_bytes());
        }
        heads.push(bits);
        heads.extend_from_slice(&(body.len() as u32).to_le_bytes());
        pack(
            &mut body,
            res.iter().map(|&r| r.wrapping_sub(m) as u64),
            bits,
        );
    }
    heads.extend_from_slice(&body);
    heads
}

/// The smaller of the two codecs, as Tantivy picks by estimated size.
fn fast_encode(nums: &[i64]) -> Vec<u8> {
    let (b, l) = (fast_bitpacked(nums), fast_linear(nums));
    if l.len() < b.len() { l } else { b }
}

/// Value `i` of a fast payload of `n` values.
fn fast_get(p: &[u8], n: usize, i: usize) -> Result<i64, Error> {
    if i >= n {
        return Err(bad());
    }
    match p.first() {
        Some(0) => {
            let (min, g, bits) = (le(p, 1)? as i64, le(p, 9)?, *p.get(17).ok_or_else(bad)?);
            Ok(min.wrapping_add((unpack_at(&p[18..], bits, i).wrapping_mul(g)) as i64))
        }
        Some(1) => {
            let blocks = n.div_ceil(LINE_BLOCK);
            let h = 1 + (i / LINE_BLOCK) * 29;
            let (a, s, m) = (
                le(p, h)? as i64,
                le(p, h + 8)? as i64,
                le(p, h + 16)? as i64,
            );
            let bits = *p.get(h + 24).ok_or_else(bad)?;
            let off = u32::from_le_bytes(
                p.get(h + 25..h + 29)
                    .ok_or_else(bad)?
                    .try_into()
                    .map_err(|_| bad())?,
            ) as usize;
            let body = p.get(1 + blocks * 29 + off..).ok_or_else(bad)?;
            let j = i % LINE_BLOCK;
            let r = m.wrapping_add(unpack_at(body, bits, j) as i64);
            Ok(a.wrapping_add(s.wrapping_mul(j as i64)).wrapping_add(r))
        }
        _ => Err(bad()),
    }
}

fn fast_all(p: &[u8], n: usize) -> Result<Vec<i64>, Error> {
    (0..n).map(|i| fast_get(p, n, i)).collect()
}

fn bad() -> Error {
    Error::Decode("column block")
}

/// The integer whose canonical text is exactly `v`, if there is one.
fn as_int(v: &[u8]) -> Option<i64> {
    let s = std::str::from_utf8(v).ok()?;
    let n: i64 = s.parse().ok()?;
    (n.to_string() == s).then_some(n)
}

/// `v` as an integer scaled by 10^`places`, if formatting it back with that
/// many places gives `v` exactly.
fn as_decimal(v: &[u8], places: usize) -> Option<i64> {
    let s = std::str::from_utf8(v).ok()?;
    let (whole, frac) = s.split_once('.')?;
    if frac.len() != places || !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let neg = whole.starts_with('-');
    let w = as_int(whole.trim_start_matches('-').as_bytes())?;
    let n = w
        .checked_mul(10i64.checked_pow(places as u32)?)?
        .checked_add(frac.parse::<i64>().ok()?)?;
    let n = if neg { -n } else { n };
    (format_decimal(n, places).as_bytes() == v).then_some(n)
}

fn format_decimal(n: i64, places: usize) -> String {
    let p = 10i64.pow(places as u32);
    let sign = if n < 0 { "-" } else { "" };
    let a = n.unsigned_abs();
    format!("{sign}{}.{:0places$}", a / p as u64, a % p as u64)
}

fn zigzag(n: i64) -> usize {
    ((n << 1) ^ (n >> 63)) as u64 as usize
}

fn unzigzag(n: usize) -> i64 {
    ((n >> 1) as i64) ^ -((n & 1) as i64)
}

fn put_deltas(out: &mut Vec<u8>, nums: &[i64]) {
    let mut prev = 0i64;
    for &n in nums {
        put_varint(out, zigzag(n.wrapping_sub(prev)));
        prev = n;
    }
}

/// One column's kind byte and uncompressed encoding.
fn encode_column(values: &[&[u8]]) -> (u8, Vec<u8>) {
    let mut out = Vec::new();
    if let Some(nums) = values.iter().map(|v| as_int(v)).collect::<Option<Vec<_>>>() {
        put_deltas(&mut out, &nums);
        return (INT, out);
    }
    let places = values
        .first()
        .and_then(|v| v.iter().rposition(|&b| b == b'.').map(|i| v.len() - i - 1));
    if let Some(p) = places.filter(|&p| p > 0 && p < 18)
        && let Some(nums) = values
            .iter()
            .map(|v| as_decimal(v, p))
            .collect::<Option<Vec<_>>>()
    {
        out.push(p as u8);
        put_deltas(&mut out, &nums);
        return (DECIMAL, out);
    }
    let mut dict: HashMap<&[u8], usize> = HashMap::new();
    let mut order: Vec<&[u8]> = Vec::new();
    for v in values {
        if !dict.contains_key(v) && order.len() <= DICT_MAX {
            dict.insert(v, order.len());
            order.push(v);
        }
    }
    if order.len() <= DICT_MAX && values.len() >= 4 * order.len() {
        put_varint(&mut out, order.len());
        for v in &order {
            put_varint(&mut out, v.len());
            out.extend_from_slice(v);
        }
        out.extend(values.iter().map(|v| dict[v] as u8));
        return (DICT, out);
    }
    for v in values {
        put_varint(&mut out, v.len());
    }
    for v in values {
        out.extend_from_slice(v);
    }
    (RAW, out)
}

fn take<'a>(b: &mut &'a [u8], n: usize) -> Result<&'a [u8], Error> {
    if b.len() < n {
        return Err(bad());
    }
    let (head, rest) = b.split_at(n);
    *b = rest;
    Ok(head)
}

fn get_deltas(b: &mut &[u8], n: usize) -> Result<Vec<i64>, Error> {
    let mut prev = 0i64;
    (0..n)
        .map(|_| {
            prev = prev.wrapping_add(unzigzag(varint(b).ok_or_else(bad)?));
            Ok(prev)
        })
        .collect()
}

fn decode_column(kind: u8, mut b: &[u8], n: usize) -> Result<Vec<Vec<u8>>, Error> {
    let b = &mut b;
    let out = match kind {
        INT => get_deltas(b, n)?
            .into_iter()
            .map(|v| v.to_string().into_bytes())
            .collect(),
        DECIMAL => {
            let p = take(b, 1)?[0] as usize;
            let nums = get_deltas(b, n)?;
            nums.into_iter()
                .map(|v| format_decimal(v, p).into_bytes())
                .collect()
        }
        FAST_INT => {
            let nums = fast_all(b, n)?;
            *b = &[];
            nums.into_iter()
                .map(|v| v.to_string().into_bytes())
                .collect()
        }
        FAST_DEC => {
            let p = take(b, 1)?[0] as usize;
            let nums = fast_all(b, n)?;
            *b = &[];
            nums.into_iter()
                .map(|v| format_decimal(v, p).into_bytes())
                .collect()
        }
        DICT => {
            let mut dict = Vec::new();
            for _ in 0..varint(b).ok_or_else(bad)? {
                let len = varint(b).ok_or_else(bad)?;
                dict.push(take(b, len)?.to_vec());
            }
            let idx = take(b, n)?;
            idx.iter()
                .map(|&i| dict.get(i as usize).cloned().ok_or_else(bad))
                .collect::<Result<_, _>>()?
        }
        RAW => {
            let lens: Vec<usize> = (0..n)
                .map(|_| varint(b).ok_or_else(bad))
                .collect::<Result<_, _>>()?;
            lens.into_iter()
                .map(|l| Ok(take(b, l)?.to_vec()))
                .collect::<Result<_, Error>>()?
        }
        _ => return Err(bad()),
    };
    if !b.is_empty() {
        return Err(bad());
    }
    Ok(out)
}

/// Encode `rows` as one block: magic, row count, separator, the per-row
/// field counts as an integer column, then each field column. Each column
/// is its kind, its row count, its compressed length and its zstd frame.
pub fn encode(rows: &[&[u8]], sep: u8) -> Result<Vec<u8>, Error> {
    encode_with(rows, sep, &ColOpts::default())
}

/// `encode` with E252's options.
pub fn encode_with(rows: &[&[u8]], sep: u8, opts: &ColOpts) -> Result<Vec<u8>, Error> {
    let split: Vec<Vec<&[u8]>> = rows
        .iter()
        .map(|r| r.split(|&b| b == sep).collect())
        .collect();
    let width = split.iter().map(Vec::len).max().unwrap_or(0);
    let counts: Vec<Vec<u8>> = split
        .iter()
        .map(|f| f.len().to_string().into_bytes())
        .collect();
    let mut columns = vec![counts.iter().map(Vec::as_slice).collect::<Vec<_>>()];
    for j in 0..width {
        columns.push(split.iter().filter_map(|f| f.get(j).copied()).collect());
    }
    let mut out = MAGIC.to_vec();
    put_varint(&mut out, rows.len());
    out.push(sep);
    put_varint(&mut out, columns.len());
    for (c, col) in columns.iter().enumerate() {
        let (mut kind, mut raw) = encode_column(col);
        if opts.fast_numbers && c > 0 && (kind == INT || kind == DECIMAL) {
            let mut b = &raw[usize::from(kind == DECIMAL)..];
            let nums = get_deltas(&mut b, col.len())?;
            let places = (kind == DECIMAL).then(|| raw[0]);
            raw = places.into_iter().chain(fast_encode(&nums)).collect();
            kind = if places.is_some() { FAST_DEC } else { FAST_INT };
        }
        let as_is = matches!(kind, FAST_INT | FAST_DEC) && !opts.compress_fast;
        let packed = match as_is {
            true => raw.clone(),
            false => zstd::bulk::compress(&raw, LEVEL).map_err(Error::Io)?,
        };
        out.push(kind | if as_is { AS_IS } else { 0 });
        put_varint(&mut out, col.len());
        put_varint(&mut out, raw.len());
        put_varint(&mut out, packed.len());
        out.extend_from_slice(&packed);
    }
    if decode(&out)? != rows {
        return Err(Error::Decode("column block does not round-trip"));
    }
    Ok(out)
}

/// One stored column: kind, value count, raw length and zstd frame.
type Packed<'a> = (u8, usize, usize, &'a [u8]);

/// The block's row count, separator and columns, decompressing none.
fn columns(block: &[u8]) -> Result<(usize, u8, Vec<Packed<'_>>), Error> {
    let mut b = block.strip_prefix(MAGIC).ok_or_else(bad)?;
    let b = &mut b;
    let rows = varint(b).ok_or_else(bad)?;
    let sep = take(b, 1)?[0];
    let mut out = Vec::new();
    for _ in 0..varint(b).ok_or_else(bad)? {
        let kind = take(b, 1)?[0];
        let n = varint(b).ok_or_else(bad)?;
        let raw = varint(b).ok_or_else(bad)?;
        let len = varint(b).ok_or_else(bad)?;
        out.push((kind, n, raw, take(b, len)?));
    }
    if !b.is_empty() || out.is_empty() {
        return Err(bad());
    }
    Ok((rows, sep, out))
}

fn raw_of(c: &Packed) -> Result<Vec<u8>, Error> {
    match c.0 & AS_IS {
        0 => zstd::bulk::decompress(c.3, c.2).map_err(Error::Io),
        _ => Ok(c.3.to_vec()),
    }
}

fn unpack(c: &Packed) -> Result<Vec<Vec<u8>>, Error> {
    decode_column(c.0 & !AS_IS, &raw_of(c)?, c.1)
}

/// Field `j` of every row as integers (decimals scaled by their places),
/// for a numeric column (E252's accessor; E241's kinds decode their deltas).
pub fn ints(block: &[u8], j: usize) -> Result<Vec<i64>, Error> {
    let (_, _, cols) = columns(block)?;
    let c = cols.get(j + 1).ok_or_else(bad)?;
    let raw = raw_of(c)?;
    let mut b = raw.as_slice();
    match c.0 & !AS_IS {
        INT => get_deltas(&mut b, c.1),
        DECIMAL => get_deltas(&mut &b[1..], c.1),
        FAST_INT => fast_all(b, c.1),
        FAST_DEC => fast_all(&b[1..], c.1),
        _ => Err(bad()),
    }
}

/// Field `j` of row `i` (rows that have it), as text. A fast column stored
/// as is reads the one value; any other column is decoded.
pub fn value(block: &[u8], j: usize, i: usize) -> Result<Vec<u8>, Error> {
    let (_, _, cols) = columns(block)?;
    let c = cols.get(j + 1).ok_or_else(bad)?;
    match c.0 {
        k if k == FAST_INT | AS_IS => Ok(fast_get(c.3, c.1, i)?.to_string().into_bytes()),
        k if k == FAST_DEC | AS_IS => {
            let p = *c.3.first().ok_or_else(bad)? as usize;
            Ok(format_decimal(fast_get(&c.3[1..], c.1, i)?, p).into_bytes())
        }
        _ => unpack(c)?.into_iter().nth(i).ok_or_else(bad),
    }
}

/// Every row of a block, byte for byte as given to `encode`.
pub fn decode(block: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    let (rows, sep, cols) = columns(block)?;
    let counts = unpack(&cols[0])?;
    let fields: Vec<Vec<Vec<u8>>> = cols[1..].iter().map(unpack).collect::<Result<_, _>>()?;
    let mut next = vec![0usize; fields.len()];
    let mut out = Vec::with_capacity(rows);
    for count in counts.iter().take(rows) {
        let count = as_int(count).ok_or_else(bad)? as usize;
        let mut row = Vec::new();
        if count > fields.len() {
            return Err(bad());
        }
        for (j, (f, at)) in fields.iter().zip(next.iter_mut()).take(count).enumerate() {
            if j > 0 {
                row.push(sep);
            }
            row.extend_from_slice(f.get(*at).ok_or_else(bad)?);
            *at += 1;
        }
        out.push(row);
    }
    if out.len() != rows || counts.len() != rows {
        return Err(bad());
    }
    Ok(out)
}

/// Field `j` (from 0) of every row that has one, decoding only that column.
pub fn column(block: &[u8], j: usize) -> Result<Vec<Vec<u8>>, Error> {
    let (_, _, cols) = columns(block)?;
    unpack(cols.get(j + 1).ok_or_else(bad)?)
}

impl<S: Store> Repo<S> {
    /// Store `rows` as one column block (E241) and return its content id.
    pub fn put_column_block(&mut self, rows: &[&[u8]], sep: u8) -> Result<Id, Error> {
        Ok(self.put_content(&encode(rows, sep)?)?.id)
    }

    /// Append `rows` to a database as one event whose value is their
    /// column block, under `key` (for example `table/blocks/` and the first
    /// row's number). The log holds the block, so the rows read back from
    /// it byte for byte.
    pub fn db_append_block(
        &mut self,
        db: &crate::Db,
        key: &str,
        rows: &[&[u8]],
        sep: u8,
        idempotency_key: &str,
    ) -> Result<crate::Db, Error> {
        let block = encode(rows, sep)?;
        self.db_append(db, &[Event::upsert(key, &block, idempotency_key)])
    }
}
