# ikam-kernel

A content-addressed store whose files are guaranteed to reconstruct byte-for-byte. It provides:
- dedup at data-driven boundaries
- compression
- zip-container unpacking
- a ledger of verified derivations (`output = func(args)`, with byte-range selectors). The ledger is storage, provenance and the object graph at once.
- pure (WASM) functions, memoized through the ledger
- git-like versioning with store-wide repacking

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
cargo run --release -- claim A "revision of" B     # semantic claim, weight measured in bits
cargo run --release -- relate ID 5                 # contents most informative about ID
cargo run --release -- promote CLAIM               # turn a claim into verified storage
cargo run --release -- fsck                        # reconstruct and verify everything
cargo run --release -- gc                          # drop objects no ref reaches
cargo run --release -- repack                      # re-plan all storage (only if smaller and verified)
```

The store lives in `.ikam` by default; override it with `--store DIR`.

Benchmark (deterministic corpora from this repo, verified exact, with zstd baselines):

```sh
cargo run --release --example bench               # table
cargo run --release --example bench -- --json     # one JSON line per corpus
cargo run --release --example bench -- --read-weight 0.01 invoices
```

A file's id is the BLAKE3 hash of its content and never depends on how it is stored. Its bytes may be stored (zstd-compressed when smaller), or it may exist only through derivations, for example:
- `concat` of byte ranges of other content, found by extending matches byte-by-byte
- `deflate-pack` rebuilding a zip from its uncompressed members
- `fill(template, fillers)`: similar documents share one template; each keeps only what differs
- a WASM function of other content

One file may have several derivations. A corrupt stored copy falls through to them.

Claims are the descriptive half of the graph: `subject predicate object` over any selections, with a weight the kernel measures (bits the object saves on the subject). They never affect reconstruction, but `promote` turns one into storage when that is verifiably smaller.

Stored bytes use the smallest of raw, zstd, or zstd against a shared dictionary trained by `repack` (the dictionary is content too). Plans are chosen by an evaluator: bytes written plus `read_weight` × decode work.

Every derivation is verified by reconstructing the exact bytes before it is recorded. Planners, including future AI ones, can therefore cost space but never correctness. Which bytes stay stored is a cost decision that `repack` re-makes store-wide.

WASM functions export `memory`, `alloc(len) -> ptr` and `run(ptr, len) -> (out_ptr << 32 | out_len)`. They may not import anything or use floats, and they run under fixed fuel and memory limits.
