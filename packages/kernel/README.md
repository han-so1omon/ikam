# ikam-kernel

A content-addressed store whose files are guaranteed to reconstruct byte-for-byte. It provides:
- dedup at data-driven boundaries
- compression
- zip-container unpacking
- pure (WASM) functions whose applications are both a storage form and provenance
- a queryable object graph
- git-like versioning

The design, laws and measurements are in `docs/plans/2026-10-02-rust-kernel.md`.

```sh
cargo test                                         # property tests for every law
cargo run --release -- put FILE                    # prints the file's content id
cargo run --release -- cat ID > out                # reconstructs FILE exactly
cargo run --release -- commit DIR -m "msg"         # snapshot DIR onto ref main
cargo run --release -- log                         # history of main
cargo run --release -- checkout main DEST          # restore a snapshot
cargo run --release -- apply FUNC ARG...           # run a stored WASM function (memoized)
cargo run --release -- links ID                    # outgoing graph edges
cargo run --release -- used-by ID                  # incoming graph edges
cargo run --release -- fsck                        # reconstruct and verify everything
cargo run --release -- gc                          # drop objects no ref reaches
```

The store lives in `.ikam` by default; override it with `--store DIR`.

A file's id is the BLAKE3 hash of its content and never depends on how it is stored. A file is stored as one of:
- **bytes** (zstd-compressed when smaller)
- **slices of other stored bytes**, found by extending matches byte-by-byte
- **a pure function applied to other stored content**, e.g. a zip rebuilt from its uncompressed members

Every proposed form is verified by reconstructing the exact bytes before it is written. Planners, including future AI ones, can therefore cost space but never correctness.

WASM functions export `memory`, `alloc(len) -> ptr` and `run(ptr, len) -> (out_ptr << 32 | out_len)`. They may not import anything or use floats, and they run under fixed fuel and memory limits.
