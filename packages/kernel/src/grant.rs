//! Section grants (E140): what one goal may read and change, by byte range.
//!
//! A grant is issued by the server for one goal. For each file version it
//! names (a path and the content id of the version), it lists the byte
//! ranges the caller may read and the ranges it may change. Reads hand out
//! only the granted ranges, through range reads, so a goal that needs the
//! lines around a mention fetches about those lines. An edit is checked by
//! finding what it changes in the old version: every changed region must
//! sit inside a write range, and the old version must be the one granted.
//!
//! Grants are checked at the file layer, never on storage objects (E126):
//! a group, dictionary or delta object can hold bytes of several files.
//! Nothing here is stored; a grant is sealed with a keyed hash so that it
//! can be handed to a caller and checked when it comes back.

use std::ops::Range;

use similar::{Algorithm, capture_diff_slices_deadline};

use crate::{Error, Id, ReadSession, Store};

/// Granted ranges and their bytes.
pub type Sections = Vec<(Range<u64>, Vec<u8>)>;

/// One file version's ranges. Read ranges always include the write ranges:
/// a caller can see what it may change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileGrant {
    pub path: String,
    pub id: Id,
    pub read: Vec<Range<u64>>,
    pub write: Vec<Range<u64>>,
}

/// What one goal may read and change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grant {
    pub goal: String,
    pub files: Vec<FileGrant>,
}

/// Why a read or an edit was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Denied {
    /// The grant names no such path.
    NoFile,
    /// The grant is for another version of the file.
    Stale,
    /// This range of the old version is outside what the grant allows.
    Outside(Range<u64>),
}

/// Sorted, merged, non-empty ranges.
fn normalise(mut v: Vec<Range<u64>>) -> Vec<Range<u64>> {
    v.retain(|r| r.start < r.end);
    v.sort_by_key(|r| r.start);
    let mut out: Vec<Range<u64>> = Vec::with_capacity(v.len());
    for r in v {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// Whether `at` (a region of the old version; empty for an insertion) lies
/// inside one of the sorted, merged `ranges`.
pub(crate) fn inside(ranges: &[Range<u64>], at: &Range<u64>) -> bool {
    let i = ranges.partition_point(|r| r.end < at.end);
    ranges
        .get(i)
        .is_some_and(|r| r.start <= at.start && at.end <= r.end)
}

fn prefix(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn suffix(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(x, y)| x == y)
        .count()
}

/// The region of `old` that `new` replaces, as one range, aligned on the
/// left (longest common prefix first) or on the right. Both are minimal;
/// they differ when the change sits in repeated bytes.
fn region(old: &[u8], new: &[u8], left: bool) -> Range<usize> {
    let (p, s) = if left {
        let p = prefix(old, new);
        (p, suffix(&old[p..], &new[p..]))
    } else {
        let s = suffix(old, new);
        (prefix(&old[..old.len() - s], &new[..new.len() - s]), s)
    };
    p..old.len() - s
}

fn lines(b: &[u8]) -> Vec<&[u8]> {
    b.split_inclusive(|&c| c == b'\n').collect()
}

impl FileGrant {
    pub fn new(path: &str, id: Id, read: Vec<Range<u64>>, write: Vec<Range<u64>>) -> FileGrant {
        let write = normalise(write);
        let read = normalise(read.into_iter().chain(write.iter().cloned()).collect());
        FileGrant {
            path: path.into(),
            id,
            read,
            write,
        }
    }

    /// Granted bytes of this file version.
    pub fn granted_bytes(&self) -> u64 {
        self.read.iter().map(|r| r.end - r.start).sum()
    }

    /// The granted read ranges and their bytes, through range reads.
    pub fn read<S: Store>(&self, s: &ReadSession<S>) -> Result<Sections, Error> {
        self.read
            .iter()
            .map(|r| Ok((r.clone(), s.read_range(&self.id, r.start, r.end - r.start)?)))
            .collect()
    }

    /// The regions of the old version that the edit changes, if all of them
    /// are inside the write ranges. `old` must be the bytes of `self.id`.
    pub fn check_edit(&self, old: &[u8], new: &[u8]) -> Result<Vec<Range<u64>>, Denied> {
        let to64 = |r: Range<usize>| r.start as u64..r.end as u64;
        if old == new {
            return Ok(vec![]);
        }
        // One region covers most edits; try both alignments.
        let left = to64(region(old, new, true));
        if inside(&self.write, &left) {
            return Ok(vec![left]);
        }
        let right = to64(region(old, new, false));
        if inside(&self.write, &right) {
            return Ok(vec![right]);
        }
        // Several regions: changed lines, each narrowed to its bytes.
        let (lo, ln) = (lines(old), lines(new));
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
        let ops = capture_diff_slices_deadline(Algorithm::Myers, &lo, &ln, Some(deadline));
        let starts = |ls: &[&[u8]]| {
            let mut v = Vec::with_capacity(ls.len() + 1);
            let mut at = 0;
            v.push(0);
            for l in ls {
                at += l.len();
                v.push(at);
            }
            v
        };
        let (so, sn) = (starts(&lo), starts(&ln));
        let mut out = Vec::new();
        for op in ops {
            let (tag, o, n) = op.as_tag_tuple();
            if tag == similar::DiffTag::Equal {
                continue;
            }
            let (ob, nb) = (so[o.start]..so[o.end], sn[n.start]..sn[n.end]);
            let (oh, nh) = (&old[ob.clone()], &new[nb]);
            let mut first = None;
            let mut found = None;
            for left in [true, false] {
                let r = region(oh, nh, left);
                let at = (ob.start + r.start) as u64..(ob.start + r.end) as u64;
                if inside(&self.write, &at) {
                    found = Some(at);
                    break;
                }
                first.get_or_insert(at);
            }
            let Some(at) = found else {
                return Err(Denied::Outside(first.unwrap()));
            };
            out.push(at);
        }
        Ok(out)
    }
}

impl Grant {
    pub fn file(&self, path: &str) -> Option<&FileGrant> {
        self.files.iter().find(|f| f.path == path)
    }

    /// Whether bytes `range` of version `id` at `path` may be read.
    pub fn may_read(&self, path: &str, id: &Id, range: Range<u64>) -> Result<(), Denied> {
        let f = self.file(path).ok_or(Denied::NoFile)?;
        if f.id != *id {
            return Err(Denied::Stale);
        }
        if range.start < range.end && !inside(&f.read, &range) {
            return Err(Denied::Outside(range));
        }
        Ok(())
    }

    /// Check an edit of `path` from version `old_id` (bytes `old`) to `new`.
    pub fn check_edit(
        &self,
        path: &str,
        old_id: &Id,
        old: &[u8],
        new: &[u8],
    ) -> Result<Vec<Range<u64>>, Denied> {
        let f = self.file(path).ok_or(Denied::NoFile)?;
        if f.id != *old_id {
            return Err(Denied::Stale);
        }
        f.check_edit(old, new)
    }

    /// A canonical encoding: the goal, then each file by path.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        let put = |out: &mut Vec<u8>, b: &[u8]| {
            out.extend_from_slice(&(b.len() as u64).to_le_bytes());
            out.extend_from_slice(b);
        };
        put(&mut out, self.goal.as_bytes());
        let mut files: Vec<&FileGrant> = self.files.iter().collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        out.extend_from_slice(&(files.len() as u64).to_le_bytes());
        for f in files {
            put(&mut out, f.path.as_bytes());
            out.extend_from_slice(f.id.as_bytes());
            for ranges in [&f.read, &f.write] {
                out.extend_from_slice(&(ranges.len() as u64).to_le_bytes());
                for r in ranges {
                    out.extend_from_slice(&r.start.to_le_bytes());
                    out.extend_from_slice(&r.end.to_le_bytes());
                }
            }
        }
        out
    }

    /// A tag that only the holder of `key` can make for this grant.
    pub fn seal(&self, key: &[u8; 32]) -> [u8; 32] {
        *blake3::keyed_hash(key, &self.encode()).as_bytes()
    }

    /// Whether `tag` was made by `seal` with `key` for this grant
    /// (compared in constant time).
    pub fn verify(&self, key: &[u8; 32], tag: &[u8; 32]) -> bool {
        blake3::keyed_hash(key, &self.encode()) == blake3::Hash::from(*tag)
    }
}

/// Byte ranges of the lines within `context` lines of a line holding
/// `term`, merged: the sections a goal about `term` needs.
pub fn windows_around(bytes: &[u8], term: &[u8], context: usize) -> Vec<Range<u64>> {
    let ls = lines(bytes);
    let mut at = Vec::with_capacity(ls.len() + 1);
    let mut pos = 0u64;
    for l in &ls {
        at.push(pos);
        pos += l.len() as u64;
    }
    at.push(pos);
    let mut out = Vec::new();
    for (i, l) in ls.iter().enumerate() {
        if !term.is_empty() && memchr::memmem::find(l, term).is_some() {
            let (a, b) = (i.saturating_sub(context), (i + context + 1).min(ls.len()));
            out.push(at[a]..at[b]);
        }
    }
    normalise(out)
}
