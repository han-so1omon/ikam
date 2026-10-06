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

    fn range(&self, id: &Id, start: u64, len: u64, cx: &mut Cx) -> Result<Vec<u8>, Error> {
        let stored = cx.known.contains_key(id) || self.store.read(id).is_ok();
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
