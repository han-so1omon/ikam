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
    for col in &columns {
        let (kind, raw) = encode_column(col);
        let packed = zstd::bulk::compress(&raw, LEVEL).map_err(Error::Io)?;
        out.push(kind);
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

fn unpack(c: &Packed) -> Result<Vec<Vec<u8>>, Error> {
    let raw = zstd::bulk::decompress(c.3, c.2).map_err(Error::Io)?;
    decode_column(c.0, &raw, c.1)
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
