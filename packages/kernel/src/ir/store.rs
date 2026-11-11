//! IR packs in the store (E223): files kept as members of one IR pack.
//!
//! `Repo::put_ir` stores one pack of `files` as content and each file as
//! the fact `ir_member(pack, index)`, so a repack keeps the pack and drops
//! the files' own bytes. Reading a file decodes the whole pack; the last
//! pack decoded on each thread is kept, so reading a pack's files one after
//! another decodes it once.

use std::cell::RefCell;
use std::rc::Rc;

use super::{DecState, Options, decode, decode_with, encode, encode_blocks, pack, unpack};
use crate::object::Arg;
use crate::{Error, Id, Repo, Store, func};

type Files = Vec<(String, Vec<u8>)>;

thread_local! {
    static LAST: RefCell<Option<(Id, Rc<Files>)>> = const { RefCell::new(None) };
}

fn parse_index(index: &[u8]) -> Result<usize, Error> {
    std::str::from_utf8(index)
        .ok()
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|s| s.parse().ok())
        .ok_or(Error::Decode("ir-member index"))
}

/// The files `packed` decodes to (with `decode`), from this thread's last
/// decode when it is the same pack, and the decode work (0 when cached).
fn cached(
    packed: &[u8],
    decode: impl FnOnce() -> Result<Files, Error>,
) -> Result<(Rc<Files>, usize), Error> {
    let id = Id::of_content(packed);
    let hit = LAST.with(|l| {
        l.borrow()
            .as_ref()
            .filter(|(k, _)| *k == id)
            .map(|(_, f)| f.clone())
    });
    if let Some(f) = hit {
        return Ok((f, 0));
    }
    let f = Rc::new(decode()?);
    let work = f.iter().map(|(_, b)| b.len()).sum();
    LAST.with(|l| *l.borrow_mut() = Some((id, f.clone())));
    Ok((f, work))
}

fn pick(files: &Files, index: usize, work: usize) -> Result<(Vec<u8>, usize), Error> {
    let out = files
        .get(index)
        .ok_or(Error::Decode("ir-member index past the pack"))?
        .1
        .clone();
    let work = work.max(out.len());
    Ok((out, work))
}

/// `ir_block_member(skeletons, block, index)`: the file at `index` of one
/// block (encoded without a keyframe), its skeletons in `skeletons` (a
/// pack of only skeletons).
pub(crate) fn block_member(args: &[Vec<u8>]) -> Result<(Vec<u8>, usize), Error> {
    let [skeletons, block, index] = args else {
        return Err(Error::Exec(
            "ir-block-member takes skeletons, a block and an index".into(),
        ));
    };
    let index = parse_index(index)?;
    let key = [skeletons.as_slice(), block.as_slice()].concat();
    let (files, work) = cached(&key, || {
        let skeletons = unpack(skeletons)?.skeletons;
        decode_with(&unpack(block)?, &skeletons, &mut DecState::default())
    })?;
    pick(&files, index, work)
}

/// `ir_member(pack, index)`: the file at `index` (decimal text) of the IR
/// pack, and the decode work (every byte the pack decodes to, unless this
/// thread decoded the same pack last).
pub(crate) fn member(args: &[Vec<u8>]) -> Result<(Vec<u8>, usize), Error> {
    let [packed, index] = args else {
        return Err(Error::Exec("ir-member takes a pack and an index".into()));
    };
    let index = parse_index(index)?;
    let (files, work) = cached(packed, || decode(&unpack(packed)?))?;
    pick(&files, index, work)
}

impl<S: Store> Repo<S> {
    /// Store `files` as one IR pack under `o` and each file as a member of
    /// it; returns each file's content id, in order. Every member is
    /// verified (it must decode to the file's bytes) before it is recorded.
    pub fn put_ir(&mut self, files: &[(String, Vec<u8>)], o: Options) -> Result<Vec<Id>, Error> {
        let packed = pack(&encode(files, o));
        let pack_id = self.put_content(&packed)?.id;
        let mut ids = Vec::with_capacity(files.len());
        for (i, (_, bytes)) in files.iter().enumerate() {
            let index = self.put_content(i.to_string().as_bytes())?.id;
            let put = self.put_derivation(
                bytes,
                func::ir_member(),
                vec![Arg::Whole(pack_id), Arg::Whole(index)],
            )?;
            ids.push(
                put.ok_or(Error::Exec("ir-member does not reproduce its file".into()))?
                    .id,
            );
        }
        Ok(ids)
    }

    /// `put_ir` in blocks of `per` files (E197, without a keyframe): one
    /// skeleton table and one content per block, so reading a file decodes
    /// only its block. Returns each file's content id, in the order given.
    pub fn put_ir_blocks(
        &mut self,
        files: &[(String, Vec<u8>)],
        o: Options,
        per: usize,
    ) -> Result<Vec<Id>, Error> {
        let b = encode_blocks(files, o, per, false);
        let skeletons = pack(&super::Parts {
            skeletons: b.skeletons.clone(),
            ..super::Parts::default()
        });
        let skeletons_id = self.put_content(&skeletons)?.id;
        // Blocks may hold files in another order than given (`sort`).
        let mut by_name: std::collections::HashMap<&str, Vec<usize>> = Default::default();
        for (i, (name, _)) in files.iter().enumerate().rev() {
            by_name.entry(name.as_str()).or_default().push(i);
        }
        let mut ids = vec![None; files.len()];
        for p in &b.blocks {
            let decoded = decode_with(p, &b.skeletons, &mut DecState::default())?;
            let block_id = self.put_content(&pack(p))?.id;
            for (k, (name, bytes)) in decoded.iter().enumerate() {
                let index = self.put_content(k.to_string().as_bytes())?.id;
                let put = self.put_derivation(
                    bytes,
                    func::ir_block_member(),
                    vec![
                        Arg::Whole(skeletons_id),
                        Arg::Whole(block_id),
                        Arg::Whole(index),
                    ],
                )?;
                let id = put
                    .ok_or(Error::Exec(
                        "ir-block-member does not reproduce its file".into(),
                    ))?
                    .id;
                let i = by_name
                    .get_mut(name.as_str())
                    .and_then(Vec::pop)
                    .ok_or(Error::Exec("ir block holds a file not given".into()))?;
                ids[i] = Some(id);
            }
        }
        ids.into_iter()
            .map(|id| id.ok_or(Error::Exec("ir blocks left a file out".into())))
            .collect()
    }
}
