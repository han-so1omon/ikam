//! Pure functions as WASM modules, run in a deterministic interpreter.
//!
//! A module may import nothing (no clock, no randomness, no I/O) and may not
//! use floating point (NaN bit patterns are platform-dependent). Execution is
//! bounded by fuel and memory. Under those rules the same module and inputs
//! give the same output everywhere, which reconstruction requires.
//!
//! ABI: export `memory`, `alloc(len: i32) -> ptr: i32`, and
//! `run(ptr: i32, len: i32) -> i64` returning `(out_ptr << 32) | out_len`.
//! The input is `count:u32 { len:u32 bytes }` (big-endian), one per argument.

use wasmi::{Config, Engine, Linker, Module, StoreLimits, StoreLimitsBuilder};

use crate::Error;

/// Fuel budget per call: a hard bound on any one read (about 30 s at
/// measured wasmi speeds). Part of a function's semantics: a call that runs
/// out of fuel has no output. Raising it only lets more calls succeed; every
/// recorded derivation already succeeded. Whether a slow function is worth
/// using is the evaluator's call (`run` reports the fuel spent).
const FUEL: u64 = 1 << 38;
/// wasmi fuel per unit of decode work (one unit = one byte produced by a
/// builtin). Measured: interpreted programs spend ~1,000-2,000 fuel per byte
/// they output, so an interpreted generator costs 1-2x its output in work.
pub const FUEL_PER_WORK: u64 = 1_000;
const MAX_MEMORY: usize = 256 << 20;

/// Run `module` on `args`: its output and the fuel it spent.
pub fn run(module: &[u8], args: &[Vec<u8>]) -> Result<(Vec<u8>, u64), Error> {
    let fail = |e: &dyn std::fmt::Display| Error::Exec(format!("wasm: {e}"));
    let mut config = Config::default();
    config.floats(false).consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, module).map_err(|e| fail(&e))?;
    if module.imports().next().is_some() {
        return Err(Error::Exec("wasm: modules may not import anything".into()));
    }
    let limits = StoreLimitsBuilder::new().memory_size(MAX_MEMORY).build();
    let mut store = wasmi::Store::new(&engine, limits);
    store.limiter(|limits: &mut StoreLimits| limits);
    store.set_fuel(FUEL).map_err(|e| fail(&e))?;
    let instance = Linker::new(&engine)
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| fail(&e))?;
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or_else(|| fail(&"no exported memory"))?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&store, "alloc")
        .map_err(|e| fail(&e))?;
    let entry = instance
        .get_typed_func::<(i32, i32), i64>(&store, "run")
        .map_err(|e| fail(&e))?;

    let mut input = Vec::new();
    input.extend_from_slice(&(args.len() as u32).to_be_bytes());
    for a in args {
        input.extend_from_slice(&(a.len() as u32).to_be_bytes());
        input.extend_from_slice(a);
    }
    let len = i32::try_from(input.len()).map_err(|_| fail(&"input too large"))?;
    let ptr = alloc.call(&mut store, len).map_err(|e| fail(&e))?;
    memory
        .write(&mut store, ptr as u32 as usize, &input)
        .map_err(|e| fail(&e))?;
    let packed = entry.call(&mut store, (ptr, len)).map_err(|e| fail(&e))? as u64;
    let mut out = vec![0; (packed & 0xffff_ffff) as usize];
    memory
        .read(&store, (packed >> 32) as usize, &mut out)
        .map_err(|e| fail(&e))?;
    let spent = FUEL - store.get_fuel().map_err(|e| fail(&e))?;
    Ok((out, spent))
}
