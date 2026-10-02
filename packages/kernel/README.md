# ikam-kernel

A content-addressed object store (L0) and codecs whose decompositions are guaranteed to reconstruct their input byte-for-byte (L1). The design and laws are in `docs/plans/2026-10-02-rust-kernel.md`.

```sh
cargo test                                   # property tests for every law
cargo run --release -- put --codec md FILE   # prints the root id (stats on stderr)
cargo run --release -- cat ID > out          # reconstructs FILE exactly
cargo run --release -- tree ID               # shows the object tree
```

Codecs: `raw` (one blob), `cdc` (FastCDC content-defined chunks), `md` (heading sections, then paragraphs). A codec only proposes how to split. If its proposal does not tile the input, the input is stored as one blob, so a buggy or AI-driven codec can cost space but never lose data.
