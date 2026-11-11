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
//! `nearest` (cells copied from another file) or `predict` (cells that
//! follow from other columns) is refused with an error.

use std::collections::HashMap;

use super::{Error, Parts, Stream, directory, get_bytes, get_usize};

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
    if flags & 8 != 0 {
        return Err(Error::Decode(
            "ir query: cells predicted from other columns",
        ));
    }
    let slots = skeletons
        .iter()
        .map(|s| super::segments(s).map(|g| g.len() - 1))
        .collect::<Result<Vec<_>, _>>()?;
    let mut ix = Index {
        columns: flags & 1 != 0,
        rows: flags & 2 != 0,
        slots,
        ..Index::default()
    };
    if !ix.columns {
        return Err(Error::Decode("ir query: one stream for every column"));
    }
    let prose = flags & 4 != 0;
    let mut at = 1;
    let n = get_usize(f, &mut at)?;
    for _ in 0..n {
        let name = String::from_utf8(get_bytes(f, &mut at)?.to_vec())
            .map_err(|_| Error::Decode("ir: name"))?;
        ix.names.push(name);
        if prose {
            let kind = *f.get(at).ok_or(Error::Decode("ir: truncated"))?;
            at += 1;
            if kind == 1 {
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

/// A stream's values, decoded on their own.
fn values(b: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    let (mut s, mut at, mut out) = (Stream::default(), 0, Vec::new());
    while at < b.len() {
        out.push(s.get(b, &mut at)?);
    }
    Ok(out)
}

/// Gives the bytes of stream `i` (from memory, or from storage).
pub type Fetch<'a> = dyn FnMut(usize) -> Result<Vec<u8>, Error> + 'a;

/// The value of column (`skeleton`, `slot`) on every line that has it, with
/// its place. Reads the column's stream (and with `rows`, the key column's
/// and every row's stream of this column).
pub fn column(
    ix: &Index,
    skeleton: usize,
    slot: usize,
    fetch: &mut Fetch,
) -> Result<Vec<(Place, Vec<u8>)>, Error> {
    if ix.slots.get(skeleton).is_none_or(|&n| slot >= n) {
        return Err(Error::Decode("ir query: no such column"));
    }
    let places = ix.places(skeleton);
    if !ix.rows || slot == 0 {
        let Some(&si) = ix.streams.get(&(skeleton, slot, Vec::new())) else {
            return Ok(Vec::new());
        };
        let vs = values(&fetch(si)?)?;
        if vs.len() != places.len() {
            return Err(Error::Decode("ir query: stream length"));
        }
        return Ok(places.into_iter().zip(vs).collect());
    }
    let keys = column(ix, skeleton, 0, fetch)?;
    let mut per: HashMap<Vec<u8>, std::vec::IntoIter<Vec<u8>>> = HashMap::new();
    let mut out = Vec::with_capacity(keys.len());
    for (place, k) in keys {
        if !per.contains_key(&k) {
            let si = *ix
                .streams
                .get(&(skeleton, slot, k.clone()))
                .ok_or(Error::Decode("ir query: no stream"))?;
            per.insert(k.clone(), values(&fetch(si)?)?.into_iter());
        }
        let v = per
            .get_mut(&k)
            .and_then(Iterator::next)
            .ok_or(Error::Decode("ir query: stream length"))?;
        out.push((place, v));
    }
    Ok(out)
}

/// Column (`skeleton`, `slot`) on the lines whose first cell is `key`: one
/// row's history across the files. With `rows` this reads the key column's
/// stream and that row's stream only.
pub fn row(
    ix: &Index,
    skeleton: usize,
    slot: usize,
    key: &[u8],
    fetch: &mut Fetch,
) -> Result<Vec<(Place, Vec<u8>)>, Error> {
    let keys = column(ix, skeleton, 0, fetch)?;
    let mine: Vec<Place> = keys
        .iter()
        .filter(|(_, k)| k == key)
        .map(|(p, _)| *p)
        .collect();
    if slot == 0 {
        return Ok(mine.into_iter().map(|p| (p, key.to_vec())).collect());
    }
    if !ix.rows {
        return Ok(column(ix, skeleton, slot, fetch)?
            .into_iter()
            .zip(&keys)
            .filter(|(_, (_, k))| k == key)
            .map(|(x, _)| x)
            .collect());
    }
    let Some(&si) = ix.streams.get(&(skeleton, slot, key.to_vec())) else {
        return Ok(Vec::new());
    };
    let vs = values(&fetch(si)?)?;
    if vs.len() != mine.len() {
        return Err(Error::Decode("ir query: stream length"));
    }
    Ok(mine.into_iter().zip(vs).collect())
}
