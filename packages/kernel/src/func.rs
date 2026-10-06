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

/// Storage plans: derivations ingest and repack choose and may replace.
/// Every other derivation is a fact that repack preserves.
pub fn is_plan(func: &Id) -> bool {
    *func == concat() || *func == fill()
}

pub fn is_builtin(func: &Id) -> bool {
    *func == concat() || *func == deflate_pack() || *func == fill()
}

/// Run `func` on `args`: its output and the decode work it took (bytes
/// produced, plus measured WASM fuel; see `wasm::FUEL_PER_WORK`).
pub fn run(func: &Id, module: Option<&[u8]>, args: &[Vec<u8>]) -> Result<(Vec<u8>, usize), Error> {
    let out = if *func == concat() {
        args.concat()
    } else if *func == deflate_pack() {
        container::pack(args)?
    } else if *func == fill() {
        template::fill(args)?
    } else {
        let module = module.ok_or_else(|| Error::Exec(format!("{func}: no such function")))?;
        let (out, fuel) = wasm::run(module, args)?;
        let work = out.len() + (fuel / wasm::FUEL_PER_WORK) as usize;
        return Ok((out, work));
    };
    let work = out.len();
    Ok((out, work))
}
