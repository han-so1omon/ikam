# ikam-kernel

A content-addressed store whose files are guaranteed to reconstruct byte-for-byte, with dedup at data-driven boundaries and git-like versioning. The design, laws and measurements are in `docs/plans/2026-10-02-rust-kernel.md`.

```sh
cargo test                                         # property tests for every law
cargo run --release -- put FILE                    # prints the file's content id
cargo run --release -- cat ID > out                # reconstructs FILE exactly
cargo run --release -- commit DIR -m "msg"         # snapshot DIR onto ref main
cargo run --release -- log                         # history of main
cargo run --release -- checkout main DEST          # restore a snapshot
cargo run --release -- gc                          # drop objects no ref reaches
```

The store lives in `.ikam` by default; override it with `--store DIR`.

A file's id is the BLAKE3 hash of its content and never depends on how it is stored. New content is matched against stored blobs, and matches are extended byte-by-byte, so reused slices start and end exactly where the content differs. Any storage plan is verified before it is written, and an invalid plan falls back to a plain blob. A future AI planner can therefore cost space, but never correctness.
