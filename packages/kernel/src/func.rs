//! Function identity and dispatch. A builtin's id is the hash of
//! `"\0ikam/builtin/<name>"`. Every stored object's encoding starts with a
//! letter tag, so a builtin id can only equal a stored id by a BLAKE3
//! collision. Any other function id names a WASM module stored as content.

use crate::{Error, Id, container, wasm};

pub fn builtin(name: &str) -> Id {
    Id::of(format!("\0ikam/builtin/{name}").as_bytes())
}

pub fn deflate_pack() -> Id {
    builtin("deflate-pack/1")
}

/// Run `func` on `args`. `load` fetches a module's bytes by id.
pub fn run(
    func: &Id,
    args: &[Vec<u8>],
    load: impl Fn(&Id) -> Result<Vec<u8>, Error>,
) -> Result<Vec<u8>, Error> {
    if *func == deflate_pack() {
        return container::pack(args);
    }
    wasm::run(&load(func)?, args)
}
