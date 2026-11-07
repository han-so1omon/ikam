# IKAM

This repository is the workspace for the Internal Knowledge Artifacts Model (IKAM).

IKAM takes in artifacts (files and other renderable data), breaks them into dedup-able fragments, organizes the fragments into a graph, and re-renders the original artifacts from it, byte-exactly. The thesis is that lossless re-renderability will help AI agent systems automate complex processes over real corpora.

The goal (`docs/plans/2026-10-06-system-architecture.md`): take in data from any source and organize it into **one graph** that reproduces every original byte and stores information shared across sources once, including across file types (the same facts in a note, a document, a PDF and a spreadsheet).

## What is here

The Rust kernel, `packages/kernel`, is a content-addressed store whose files reconstruct byte-for-byte:
- dedup at data-driven boundaries, templates, zip and deflate-stream unpacking, compression with a trained dictionary and related-content contexts;
- a ledger of verified derivations (`output = func(args)`), including deterministic WASM functions;
- git-like history, gc, fsck and store-wide repacking;
- claims, an unverified relation layer whose weights are measured.

Every derivation is verified by reconstructing the exact bytes before it is recorded, so planners (including AI ones) can cost space but never correctness. Usage and design: `packages/kernel/README.md`, `docs/plans/2026-10-02-rust-kernel.md`.

The earlier Python packages (`ikam`, `modelado`, `interacciones`, `mcp-ikam`, `ikam-graph-viewer`, `test/ikam-perf-report`) were removed in November 2026; they are in git history. The benchmark's repo-history corpora read them from past commits, so they still need a full clone.

Docs:
- `docs/ikam/`: the normative IKAM specifications (fragment algebra, graph, staging, mutation).
- `docs/plans/`: plans, including the system architecture and the kernel design.
- `docs/experiments/`: how the kernel is improved (`README.md`), the experiment log and the registry of verdicts.
- `docs/research/`: notes on the papers experiments are based on.

Shared test fixtures are under `tests/fixtures`; the kernel's benchmark corpora are under `packages/kernel/fixtures` and are built from them.

## Kernel quick start

A C compiler is needed (zlib and zstd are built from source). The Rust toolchain is pinned in `packages/kernel/rust-toolchain.toml`.

```sh
cd packages/kernel
cargo test                                      # property tests for every law
cargo run --release -- put FILE                 # store a file, print its content id
cargo run --release -- cat ID > out             # reconstruct it exactly
cargo run --release --example bench -- --json   # the fixed experiment command
```

## Where it stands (E100, November 2026)

Experiments E001 to E100 are in `docs/experiments/log.md`. The benchmark prints two scores, both against zstd with a dictionary trained on the same files (lower is better; below 1.0 beats it):

- **Cost score, about 0.24:** monthly cost (storage, CPU, memory) under a heavy read and search workload, over eleven corpora.
- **North score, 0.3154:** bytes of three mixed-type corpora (`north/cases`, `north/multiformat`, `north/tables`) stored as one graph under an archive workload. The one graph is about a third of the baseline's size.

Every file must read back byte-exact and fsck must be clean, or a run fails.

The measured gap: storing all file types as one graph is only about 3% smaller than storing each type separately (7% on `north/tables`, the same records as xlsx, csv, json, markdown, docx and pdf). Sharing information across file types is the open work.

Known limits:
- About 30 experiment verdicts in `docs/experiments/registry.tsv` were measured under older objectives and have not been rechecked.

## Contributing

Read `AGENTS.md` first. Kernel changes to chunking, dedup, compression or reconstruction follow `docs/experiments/README.md`: one hypothesis per `exp/<id>-<slug>` branch, the fixed command, both scores, and every result recorded in the log.
