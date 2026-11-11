//! E203: read one column out of an IR without rebuilding the files.
//!
//! The IR keeps every column (line skeleton, slot) in a stream of its own,
//! and which skeleton each line of each file has in the files part. So the
//! values of one column, with the file and line of each, come from the
//! files part and that column's stream alone: no other stream is decoded
//! and no file is put back together. With `rows`, a column is cut into one
//! stream per row key (the line's first cell), so one row's history across
//! files is the key column's stream and that row's stream.
//!
//! This needs the cells in their streams: an encode with `siblings` or
//! `nearest` (cells copied from another file) is refused with an error.
//! With `predict` (and `step`), a cell follows from the first cells of its
//! line (E200, E204), so those columns are decoded too, line by line, and
//! no others (E205).

use std::collections::HashMap;

use super::{Error, PREDICTORS, Parts, Stream, directory, get_bytes, get_usize};

/// A place in the files: file index, line index within the file.
pub type Place = (usize, usize);

/// Which skeleton each line of each file has, from the files part.
#[derive(Clone, Debug, Default)]
pub struct Index {
    pub names: Vec<String>,
    /// Skeleton id per line; empty for a file stored as it is (`prose`).
    pub lines: Vec<Vec<usize>>,
    columns: bool,
    rows: bool,
    predict: bool,
    step: bool,
    /// Stream index by key (skeleton, slot, row key).
    streams: HashMap<(usize, usize, Vec<u8>), usize>,
    /// Cells per skeleton (its slots).
    slots: Vec<usize>,
}

impl Index {
    /// The places of every line with skeleton `skeleton`, in file order.
    pub fn places(&self, skeleton: usize) -> Vec<Place> {
        let mut out = Vec::new();
        for (fi, ls) in self.lines.iter().enumerate() {
            for (li, &s) in ls.iter().enumerate() {
                if s == skeleton {
                    out.push((fi, li));
                }
            }
        }
        out
    }

    /// How many lines each skeleton has.
    pub fn counts(&self) -> Vec<usize> {
        let mut c = vec![0; self.slots.len()];
        self.lines.iter().flatten().for_each(|&s| c[s] += 1);
        c
    }
}

/// Reads the files part and the directory of `parts` (the streams are not
/// touched). `skeletons` is the skeleton table (`parts.skeletons`, or the
/// shared one of blocks).
pub fn index(parts: &Parts, skeletons: &[Vec<u8>]) -> Result<Index, Error> {
    let f = &parts.files;
    let flags = *f.first().ok_or(Error::Decode("ir: empty"))?;
    let slots = skeletons
        .iter()
        .map(|s| super::segments(s).map(|g| g.len() - 1))
        .collect::<Result<Vec<_>, _>>()?;
    let mut ix = Index {
        columns: flags & 1 != 0,
        rows: flags & 2 != 0,
        predict: flags & 8 != 0,
        step: flags & 16 != 0,
        slots,
        ..Index::default()
    };
    if !ix.columns {
        return Err(Error::Decode("ir query: one stream for every column"));
    }
    if flags & 32 != 0 {
        return Err(Error::Decode(
            "ir query: rows stored in another order (sort)",
        ));
    }
    let prose = flags & (4 | 64) != 0;
    let mut at = 1;
    let n = get_usize(f, &mut at)?;
    for _ in 0..n {
        let name = String::from_utf8(get_bytes(f, &mut at)?.to_vec())
            .map_err(|_| Error::Decode("ir: name"))?;
        ix.names.push(name);
        if prose {
            let kind = *f.get(at).ok_or(Error::Decode("ir: truncated"))?;
            at += 1;
            if kind & 4 != 0 {
                // a table laid out by column: its line count and newline
                get_usize(f, &mut at)?;
                at += 1;
            }
            if kind & 1 != 0 {
                get_usize(f, &mut at)?;
                ix.lines.push(Vec::new());
                continue;
            }
        }
        let nl = get_usize(f, &mut at)?;
        let ids: Vec<usize> = (0..nl)
            .map(|_| get_usize(f, &mut at))
            .collect::<Result<_, _>>()?;
        if ids.iter().any(|&i| i >= ix.slots.len()) {
            return Err(Error::Decode("ir: bad skeleton id"));
        }
        if get_usize(f, &mut at)? != 0 {
            return Err(Error::Decode("ir query: cells copied from another file"));
        }
        ix.lines.push(ids);
    }
    for (i, k) in directory(parts)?.into_iter().enumerate() {
        ix.streams.insert(k, i);
    }
    Ok(ix)
}

/// Gives the bytes of stream `i` (from memory, or from storage).
pub type Fetch<'a> = dyn FnMut(usize) -> Result<Vec<u8>, Error> + 'a;

/// A line's place and the cells asked for.
type Line = (Place, Vec<Vec<u8>>);

/// One stream being decoded: its state and where it is.
struct Open {
    bytes: Vec<u8>,
    at: usize,
    state: Stream,
}

/// The cells in `want` (sorted, and closed under what they follow from) of
/// every line with skeleton `skeleton`, decoded line by line. With `rows`
/// and `only`, the lines with another row key are skipped (their streams
/// are their own).
fn lines_of(
    ix: &Index,
    skeleton: usize,
    want: &[usize],
    only: Option<&[u8]>,
    fetch: &mut Fetch,
) -> Result<Vec<Line>, Error> {
    let places = ix.places(skeleton);
    if !ix.predict && !ix.rows {
        // each column on its own: decode each stream straight through
        let mut cols = Vec::with_capacity(want.len());
        for &slot in want {
            let si = *ix
                .streams
                .get(&(skeleton, slot, Vec::new()))
                .ok_or(Error::Decode("ir query: no stream"))?;
            let b = fetch(si)?;
            let (mut st, mut at, mut vs) = (Stream::default(), 0, Vec::with_capacity(places.len()));
            while at < b.len() {
                vs.push(st.get(&b, &mut at)?);
            }
            if vs.len() != places.len() {
                return Err(Error::Decode("ir query: stream length"));
            }
            cols.push(vs.into_iter());
        }
        return Ok(places
            .into_iter()
            .map(|p| (p, cols.iter_mut().map(|c| c.next().unwrap()).collect()))
            .collect());
    }
    if !ix.rows {
        // one stream per wanted slot: open them all, then go line by line
        let mut open = Vec::with_capacity(want.len());
        for &slot in want {
            let si = *ix
                .streams
                .get(&(skeleton, slot, Vec::new()))
                .ok_or(Error::Decode("ir query: no stream"))?;
            open.push(Open {
                bytes: fetch(si)?,
                at: 0,
                state: Stream::default(),
            });
        }
        let mut out = Vec::with_capacity(places.len());
        for at in places {
            let mut cells: Vec<Vec<u8>> = Vec::with_capacity(want.len());
            for (wi, o) in open.iter_mut().enumerate() {
                let line = &cells[..wi.min(PREDICTORS)];
                let v = o
                    .state
                    .get_in(&o.bytes, &mut o.at, ix.predict.then_some(line), ix.step)?;
                cells.push(v);
            }
            out.push((at, cells));
        }
        return Ok(out);
    }
    let mut open: HashMap<(usize, Vec<u8>), Open> = HashMap::new();
    let mut out = Vec::new();
    for at in places {
        let mut cells: Vec<Vec<u8>> = Vec::with_capacity(want.len());
        let mut skip = false;
        for (wi, &slot) in want.iter().enumerate() {
            let row = if ix.rows && slot > 0 {
                // slot 0 comes first in `want` whenever rows are on
                cells[0].clone()
            } else {
                Vec::new()
            };
            if wi == 1
                && ix.rows
                && let Some(k) = only
                && cells[0] != k
            {
                skip = true;
                break;
            }
            let key = (slot, row);
            if !open.contains_key(&key) {
                let si = *ix
                    .streams
                    .get(&(skeleton, key.0, key.1.clone()))
                    .ok_or(Error::Decode("ir query: no stream"))?;
                let bytes = fetch(si)?;
                open.insert(
                    key.clone(),
                    Open {
                        bytes,
                        at: 0,
                        state: Stream::default(),
                    },
                );
            }
            let o = open.get_mut(&key).unwrap();
            let line = &cells[..wi.min(PREDICTORS)];
            let v = o
                .state
                .get_in(&o.bytes, &mut o.at, ix.predict.then_some(line), ix.step)?;
            cells.push(v);
        }
        if !skip {
            out.push((at, cells));
        }
    }
    Ok(out)
}

/// The slots a read of `slot` decodes: with `predict`, the line's first
/// cells it may follow from; with `rows`, the row key.
fn closure(ix: &Index, slot: usize) -> Vec<usize> {
    let mut want: Vec<usize> = if ix.predict {
        (0..slot.min(PREDICTORS)).collect()
    } else {
        Vec::new()
    };
    if ix.rows && slot > 0 && want.first() != Some(&0) {
        want.insert(0, 0);
    }
    want.push(slot);
    want
}

/// The value of column (`skeleton`, `slot`) on every line that has it, with
/// its place. Reads the column's stream (and with `rows`, the key column's
/// and every row's stream of this column; with `predict`, the streams of
/// the line's first cells).
pub fn column(
    ix: &Index,
    skeleton: usize,
    slot: usize,
    fetch: &mut Fetch,
) -> Result<Vec<(Place, Vec<u8>)>, Error> {
    if ix.slots.get(skeleton).is_none_or(|&n| slot >= n) {
        return Err(Error::Decode("ir query: no such column"));
    }
    Ok(lines_of(ix, skeleton, &closure(ix, slot), None, fetch)?
        .into_iter()
        .map(|(p, mut c)| (p, c.pop().unwrap()))
        .collect())
}

/// Column (`skeleton`, `slot`) on the lines whose first cell is `key`: one
/// row's history across the files. With `rows` this reads the key column's
/// stream and that row's streams only.
pub fn row(
    ix: &Index,
    skeleton: usize,
    slot: usize,
    key: &[u8],
    fetch: &mut Fetch,
) -> Result<Vec<(Place, Vec<u8>)>, Error> {
    if ix.slots.get(skeleton).is_none_or(|&n| slot >= n) {
        return Err(Error::Decode("ir query: no such column"));
    }
    let mut want = closure(ix, slot);
    if want[0] != 0 {
        want.insert(0, 0);
    }
    Ok(lines_of(ix, skeleton, &want, Some(key), fetch)?
        .into_iter()
        .filter(|(_, c)| c[0] == key)
        .map(|(p, mut c)| (p, c.pop().unwrap()))
        .collect())
}
