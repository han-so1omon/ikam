//! Function identity and dispatch. A builtin's id is the hash of
//! `"\0ikam/builtin/<name>"`. Every stored object's encoding starts with a
//! letter tag, so a builtin id can only equal a stored id by a BLAKE3
//! collision. Any other function id names a WASM module stored as content.

use crate::{Error, Id, container, template, wasm};

pub fn builtin(name: &str) -> Id {
    Id::of(format!("\0ikam/builtin/{name}").as_bytes())
}

/// Concatenation of all arguments; with range selectors, this is the slice
/// form ("copy these spans of other content").
pub fn concat() -> Id {
    builtin("concat/1")
}

pub fn deflate_pack() -> Id {
    builtin("deflate-pack/1")
}

/// Template instantiation: `fill(template, fillers)`; see `template.rs`.
pub fn fill() -> Id {
    builtin("fill/1")
}

/// A file rendered from its fragments (E155, `fragment.rs`): the arguments
/// concatenated, like `concat`, but a fact rather than a storage plan, so
/// repack keeps it and the file stays addressable by its fragments.
pub fn render() -> Id {
    builtin("render/1")
}

/// One file of an IR pack (E223, `ir.rs`): `ir_member(pack, index)`, the
/// index as decimal text. A fact like `render`: repack keeps it.
pub fn ir_member() -> Id {
    builtin("ir-member/1")
}

/// One file of an IR block (E223): `ir_block_member(skeletons, block,
/// index)`, the block encoded without a keyframe, the index as decimal text.
pub fn ir_block_member() -> Id {
    builtin("ir-block-member/1")
}

/// Builtins by index, for short derivation records (`abbrev.rs`). Stored
/// records depend on this order: only append.
const BUILTINS: [&str; 6] = [
    "concat/1",
    "deflate-pack/1",
    "fill/1",
    "render/1",
    "ir-member/1",
    "ir-block-member/1",
];

pub(crate) fn builtin_index(func: &Id) -> Option<u8> {
    BUILTINS
        .iter()
        .position(|n| builtin(n) == *func)
        .map(|k| k as u8)
}

pub(crate) fn builtin_at(k: u8) -> Option<Id> {
    BUILTINS.get(usize::from(k)).map(|n| builtin(n))
}

/// Storage plans: derivations ingest and repack choose and may replace.
/// Every other derivation is a fact that repack preserves.
pub fn is_plan(func: &Id) -> bool {
    *func == concat() || *func == fill()
}

pub fn is_builtin(func: &Id) -> bool {
    *func == concat()
        || *func == deflate_pack()
        || *func == fill()
        || *func == render()
        || *func == ir_member()
        || *func == ir_block_member()
}

/// Run `func` on `args`: its output and the decode work it took (bytes
/// produced, plus measured WASM fuel; see `wasm::FUEL_PER_WORK`).
pub fn run(
    func: &Id,
    module: Option<&[u8]>,
    args: &[Vec<u8>],
    memo: Option<&container::DeflateMemo>,
) -> Result<(Vec<u8>, usize), Error> {
    let out = if *func == concat() || *func == render() {
        args.concat()
    } else if *func == deflate_pack() {
        container::pack(args, memo)?
    } else if *func == fill() {
        template::fill(args)?
    } else if *func == ir_member() {
        return crate::ir::member(args);
    } else if *func == ir_block_member() {
        return crate::ir::block_member(args);
    } else {
        let module = module.ok_or_else(|| Error::Exec(format!("{func}: no such function")))?;
        let (out, fuel) = wasm::run(module, args)?;
        let work = out.len() + (fuel / wasm::FUEL_PER_WORK) as usize;
        return Ok((out, work));
    };
    let work = out.len();
    Ok((out, work))
}
