//! Range reads: rebuild only the bytes `start..start + len` of a content.
//!
//! A range is mapped backwards through storage plans: through `concat`
//! (slices) it reads only the arguments it overlaps, recursively; through
//! `fill` it decodes the template and fillers but produces only the pieces
//! it overlaps. Anything else (a stored leaf, `deflate-pack`, WASM) is
//! read whole, verified against its id, and sliced. A group member's read
//! already decodes its group only up to the member's end (`group.rs`).
//!
//! Exactness: every leaf is verified against its own id. A range through a
//! plan relies on that plan's derivation record, which was verified when
//! it was recorded (`put_derivation`, ingest and repack verify first) and
//! which fsck re-verifies; the whole output's id is not re-checked, since
//! that would need the whole output.

use crate::repo::{Cx, STACK_GUARD, select};
use crate::{Arg, Error, Id, Repo, Store, func, template};

/// Several reads that share what they decode (E128): a group, dictionary,
/// template or container part rebuilt for one read is kept for the next,
/// so a request that reads many small files, or many sections of a few
/// files, fetches each stored object once instead of once per read. Kept
/// bytes are bounded by `limit`; past it the session forgets everything
/// it holds after the read that crossed it. Nothing is written and the
/// format is unchanged; every result is checked exactly as a single read.
pub struct ReadSession<'a, S: Store> {
    repo: &'a Repo<S>,
    cx: std::cell::RefCell<Cx>,
    limit: usize,
}

impl<S: Store> ReadSession<'_, S> {
    /// The contents this session holds decoded, to hand to a write batch
    /// (`Repo::keep_read`, E145).
    pub fn into_contents(self) -> Vec<(Id, Vec<u8>)> {
        let mut out: Vec<(Id, Vec<u8>)> = self.cx.into_inner().known.into_iter().collect();
        out.sort_by_key(|a| a.0);
        out
    }

    /// Content `id`, whole.
    pub fn read_content(&self, id: &Id) -> Result<Vec<u8>, Error> {
        let mut cx = self.cx.borrow_mut();
        let out = self.repo.rebuild(id, &mut cx);
        self.trim(&mut cx);
        out
    }

    /// Bytes `start..start + len` of content `id` (see `Repo::read_range`).
    pub fn read_range(&self, id: &Id, start: u64, len: u64) -> Result<Vec<u8>, Error> {
        start.checked_add(len).ok_or(Error::Corrupt(*id))?;
        let mut cx = self.cx.borrow_mut();
        let out = self.repo.range(id, start, len, &mut cx);
        self.trim(&mut cx);
        out
    }

    /// `Repo::search` in this session (E142): the contents the scan
    /// decodes, matches included, stay for the reads that follow, so a
    /// goal that searches and then reads what it found fetches each once.
    /// The same hits as `Repo::search`.
    pub fn search(
        &self,
        pattern: &[u8],
        files: &[Id],
        index: Option<&crate::SearchIndex>,
    ) -> Result<(Vec<Id>, usize), Error> {
        let mut cx = self.cx.borrow_mut();
        let out = self
            .repo
            .search_in(pattern, files, index, &mut cx, Some(self.limit));
        self.trim(&mut cx);
        out
    }

    /// Decoded bytes the session holds now.
    pub fn held(&self) -> usize {
        self.cx.borrow().known.values().map(Vec::len).sum()
    }

    fn trim(&self, cx: &mut Cx) {
        cx.stack.clear();
        if cx.known.values().map(Vec::len).sum::<usize>() > self.limit {
            cx.known.clear();
        }
    }
}

impl<S: Store> Repo<S> {
    /// Bytes `start..start + len` of content `id`, and the decode work the
    /// read took (bytes produced by functions, as in `evaluate`). An error
    /// if the range is not inside the content.
    pub fn read_range(&self, id: &Id, start: u64, len: u64) -> Result<(Vec<u8>, usize), Error> {
        start.checked_add(len).ok_or(Error::Corrupt(*id))?;
        let mut cx = Cx::default();
        let out = self.range(id, start, len, &mut cx)?;
        Ok((out, cx.work))
    }

    /// A read session keeping up to `limit` decoded bytes between reads.
    pub fn session(&self, limit: usize) -> ReadSession<'_, S> {
        ReadSession {
            repo: self,
            cx: Default::default(),
            limit,
        }
    }

    fn range(&self, id: &Id, start: u64, len: u64, cx: &mut Cx) -> Result<Vec<u8>, Error> {
        // Presence only: reading the entry here fetched the whole object
        // once more than the rebuild below (E124).
        let stored = cx.known.contains_key(id) || self.store.has(id);
        if !stored && cx.stack.len() < STACK_GUARD && !cx.stack.contains(id) {
            cx.stack.push(*id);
            let walked = self.derivations(id)?.iter().find_map(|d| {
                let out = if d.func == func::concat() {
                    self.concat_range(&d.args, start, len, cx)
                } else if d.func == func::fill() {
                    self.fill_range(&d.args, start, len, cx)
                } else {
                    return None;
                };
                out.ok().filter(|b| b.len() as u64 == len)
            });
            cx.stack.pop();
            if let Some(out) = walked {
                cx.work += out.len();
                return Ok(out);
            }
        }
        let bytes = self.rebuild(id, cx)?;
        select(&bytes, start, len)
            .map(<[u8]>::to_vec)
            .ok_or(Error::Corrupt(*id))
    }

    fn concat_range(
        &self,
        args: &[Arg],
        start: u64,
        len: u64,
        cx: &mut Cx,
    ) -> Result<Vec<u8>, Error> {
        let (end, mut pos, mut out) = (start + len, 0, Vec::new());
        for arg in args {
            if pos >= end {
                break;
            }
            let (from, n) = match *arg {
                Arg::Range { start, len, .. } => (start, len),
                Arg::Whole(id) => (0, self.rebuild(&id, cx)?.len() as u64),
            };
            let (lo, hi) = (start.max(pos), end.min(pos + n));
            if lo < hi {
                out.extend(self.range(&arg.id(), from + lo - pos, hi - lo, cx)?);
            }
            pos += n;
        }
        Ok(out)
    }

    fn fill_range(
        &self,
        args: &[Arg],
        start: u64,
        len: u64,
        cx: &mut Cx,
    ) -> Result<Vec<u8>, Error> {
        let mut inputs = Vec::with_capacity(args.len());
        for arg in args {
            let bytes = self.rebuild(&arg.id(), cx)?;
            inputs.push(match *arg {
                Arg::Whole(_) => bytes,
                Arg::Range { id, start, len } => select(&bytes, start, len)
                    .ok_or(Error::Corrupt(id))?
                    .to_vec(),
            });
        }
        let (end, mut pos, mut out) = (start + len, 0, Vec::new());
        for piece in template::pieces(&inputs)? {
            let n = piece.len() as u64;
            let (lo, hi) = (start.max(pos), end.min(pos + n));
            if lo < hi {
                out.extend_from_slice(&piece[(lo - pos) as usize..(hi - pos) as usize]);
            }
            pos += n;
        }
        Ok(out)
    }
}
