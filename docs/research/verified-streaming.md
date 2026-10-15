# Verified partial reads (BLAKE3 tree, Bao)

Question: a range read through a stored leaf (a slice source) decodes the whole leaf, because the only check is the leaf's id, a hash of all of it (`src/range.rs`, `Repo::eval`). Could a leaf be stored in blocks, each verifiable on its own, so a slice decodes and verifies only the blocks it covers? Benchmark gap: E041 found slice sources in structured and repo-history being decoded whole many times.

## Sources read

- **BLAKE3 specification** (O'Connor, Aumasson, Neves, Wilcox-O'Hearn), https://github.com/BLAKE3-team/BLAKE3-specs/blob/master/blake3.pdf. Full text of §2.1 (tree structure) and §6.4 (verified streaming); §7 notes on the domain flags.
  - The input is split into 1024-byte chunks. The left subtree of a node is always a complete binary tree with a power-of-two number of chunks, at least as large as the right one.
  - §6.4: to verify one chunk without rehashing the file, verify the parent nodes on the path from the root to that chunk. Storing every parent node in pre-order costs "two CVs per chunk, or 6.25%".
  - An implementation that seeks must not trust an unverified length. It verifies the final chunk first.
- **Bao specification**, https://github.com/oconnor663/bao/blob/master/docs/spec.md. Read via a fetched summary, not line by line.
  - Combined encoding: tree nodes and chunks in pre-order.
  - Outboard encoding: tree nodes only, with the data kept separately.
  - Slices carry only the nodes on the requested path.
  - Guarantees every output byte is authentic and EOF is reported correctly.
- **bao-tree** (n0-computer) docs, https://docs.rs/bao-tree. Overview page only.
  - "Chunk groups" store hashes only per group of 2^k chunks, with 16 KiB suggested "as a good default".
  - The page doesn't give the overhead numbers.
- **blake3 1.8.7 `hazmat` module** (local crate source; docs and `test_grouped_hash`).
  - `HasherExt::set_input_offset` and `finalize_non_root` give the chaining value of a subtree at a chunk-aligned offset.
  - `merge_subtrees_non_root` and `merge_subtrees_root` join chaining values.
  - The test hashes fixed power-of-two groups and merges them layer by layer, reproducing `blake3::hash`.

## Mapping onto the kernel

- A content id is `blake3("B" ++ bytes)` (`src/id.rs`), a plain BLAKE3 hash. The tree therefore covers `"B" ++ bytes`, and block boundaries sit on that input, offset by one byte from the content.
- **Design considered.** Store a leaf as zstd frames, one per power-of-two group of chunks, plus each group's 32-byte chaining value. A range read would:
  1. merge all group chaining values into the root and compare it with the id (cheap, no decoding);
  2. decode only the covered groups;
  3. check each covered group against its chaining value.

  This is a group-level outboard, as in bao-tree. Collision resistance of the tree binds each group's bytes and length, so no stored length is trusted.
- **Costs.** 32 bytes per block, plus the compression lost by restarting each zstd frame at a block boundary.

## Hypotheses

- H1: blocked, verified slice sources lower the scored read cost by more than they add in bytes. Source: §6.4 and chunk groups. **Refuted by E046** (offline): slices cover 88–100% of a leaf's blocks per whole-file read, so the read saved is 100–200× smaller than the bytes added, on every corpus that has partial leaf reads.
- H2: blocked leaves cut the cost of small range reads (`read_4k`), where a slice covers few blocks. **Open.** `read_4k` is reported, not scored, so the benchmark doesn't measure this.

## Unknowns

- The E046 estimate compresses blocks without the shared dictionary, so it inferably overstates the per-block loss. The gap it finds is two orders of magnitude, so this doesn't change the verdict.
