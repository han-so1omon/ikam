---
name: ikam-kernel-experiments
description: Run storage experiments on the IKAM Rust kernel (packages/kernel). Use when proposing or testing a change to how the kernel chunks, deduplicates, compresses or reconstructs data. Covers the fixed run command, the score and hard gates, reading papers before changing code, and how to record results.
---

# IKAM kernel experiments

Read `docs/experiments/README.md` (the contract and the loop) and the latest entries of `docs/experiments/log.md` before starting.

Non-negotiables:
1. **One fixed command:** `cd packages/kernel && cargo run --release --example bench -- --json`. Never vary behaviour with flags or environment variables; change committed code on the experiment's branch.
2. **Scores:** `_summary.score` (monthly cost under the heavy workload, against zstd with a trained dictionary) and `_north.north_score` (one-graph bytes of mixed-type corpora under the archive workload, against the same baseline). Lower is better. Keep a change when one improves beyond its tolerance (cost about 2%, north 0.5%) and the other holds.
3. **Hard gates:** byte-exact reconstruction and a clean `fsck` for every file. Never weaken a test, the verification, or the gates to make a number move.
4. **Read before you change.** Every hypothesis cites a paper note in `docs/research/` or a prior measurement. Use `orx discover` / `orx paper` (or arXiv directly) and write what you actually read, not titles.
5. **One hypothesis per experiment.** Branch children for variants and grow downward from the winner. Never edit an experiment after its run.
6. **Record every result in `docs/experiments/log.md`**, including refuted ones, and update the research note's hypothesis status.
7. **Claims match evidence:** report times honestly, mark inferences as inferences, and do not call a change better on one corpus if the score got worse.
