//! Function identity and dispatch. A builtin's id is the hash of
//! `"\0ikam/builtin/<name>"`. Every stored object's encoding starts with a
//! letter tag, so a builtin id can only equal a stored id by a BLAKE3
//! collision. Any other function id names a WASM module stored as content.

use crate::{Error, Id, container, wasm};

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

pub fn is_builtin(func: &Id) -> bool {
    *func == concat() || *func == deflate_pack()
}

/// Run `func` on `args`. `module` is the WASM module for non-builtins.
pub fn run(func: &Id, module: Option<&[u8]>, args: &[Vec<u8>]) -> Result<Vec<u8>, Error> {
    if *func == concat() {
        return Ok(args.concat());
    }
    if *func == deflate_pack() {
        return container::pack(args);
    }
    wasm::run(
        module.ok_or_else(|| Error::Exec(format!("{func}: no such function")))?,
        args,
    )
}
