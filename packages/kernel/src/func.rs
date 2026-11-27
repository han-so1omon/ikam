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

/// A pinned-float step (E441): `pinned_float(module, args...)` runs the
/// WASM module given as the first argument on the rest with floats on and
/// every NaN canonical (`wasm::Floats::Pinned`). A pure derivation like any
/// other: repack may drop its output and rebuild it anywhere.
pub fn pinned_float() -> Id {
    builtin("wasm-float/1")
}

/// A float check (E441): `within_f64(first, now, spec)` compares two
/// outputs read as little-endian f64 values under a tolerance (`spec`, text
/// such as `ulp=4 abs=0 rel=1e-12`). Its output is a verdict, one line:
/// `same`, `within ...` or `outside ...` with the largest ULP, absolute and
/// relative differences. A pure builtin, so a verdict is a derivation, and
/// a check is a step in the graph like the steps it checks.
pub fn within_f64() -> Id {
    builtin("within-f64/1")
}

/// Design A of a recorded step (E441, measured against `Run`): a
/// derivation `recorded(func, head, check args..., args...)` whose output is kept and
/// never rebuilt. Evaluating it is refused.
pub fn recorded() -> Id {
    builtin("recorded/1")
}

/// Builtins by index, for short derivation records (`abbrev.rs`). Stored
/// records depend on this order: only append.
const BUILTINS: [&str; 9] = [
    "concat/1",
    "deflate-pack/1",
    "fill/1",
    "render/1",
    "ir-member/1",
    "ir-block-member/1",
    "wasm-float/1",
    "within-f64/1",
    "recorded/1",
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
    builtin_index(func).is_some()
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
    } else if *func == pinned_float() {
        let (module, rest) = args
            .split_first()
            .ok_or_else(|| Error::Exec("wasm-float: no module".into()))?;
        let (out, fuel) = wasm::run_with(module, rest, wasm::Floats::Pinned)?;
        let work = out.len() + (fuel / wasm::FUEL_PER_WORK) as usize;
        return Ok((out, work));
    } else if *func == within_f64() {
        crate::step::within(args)?
    } else if *func == recorded() {
        return Err(Error::Exec("a recorded step is kept, never rebuilt".into()));
    } else {
        let module = module.ok_or_else(|| Error::Exec(format!("{func}: no such function")))?;
        let (out, fuel) = wasm::run(module, args)?;
        let work = out.len() + (fuel / wasm::FUEL_PER_WORK) as usize;
        return Ok((out, work));
    };
    let work = out.len();
    Ok((out, work))
}
