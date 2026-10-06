# Reconstruction functions

- `interp/`: a deterministic integer stack machine (no floats, no imports), built to `interp.wasm` with `cargo build --release --target wasm32-unknown-unknown` in `interp/`. The committed `interp.wasm` is what the kernel stores; its content id names the function in every derivation, so rebuild only on purpose.
- `asm.py`: assembler for the machine's programs (`python3 asm.py prog.asm prog.bin`). The op list is in `interp/src/lib.rs`.
- `programs/`: programs (`.asm` source, `.bin` bytecode). Each is a small argument to the shared interpreter.
- `proposals.txt`: each program's output id, size and measured decode work, written by `cargo run --release --example programs`. The benchmark offers only programs whose output is a corpus file and whose decode work could pay; the kernel verifies each and repack keeps it only if it saves bytes.
