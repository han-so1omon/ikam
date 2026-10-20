# Experiments

How we improve the kernel: one fixed command, one score, hard gates, one hypothesis per branch, and every hypothesis grounded in something we read or measured. Compatible with [OpenResearch](https://github.com/alphaXiv/OpenResearch) (`orx`), but it does not depend on it. Any agent or person can follow it with git alone.

## The contract

- **Run command (fixed, identical on every experiment):** `cd packages/kernel && cargo run --release --example bench -- --json`, also available as `mise run experiment`.
  - It prints one JSON line per corpus, then a `_summary` line.
  - Never pass extra flags or environment variables to vary behaviour. Change committed code instead.
- **Benchmark changes** (corpus or how it is committed) are their own log entries that re-measure the current kernel, and scores across them are not compared. Since E009 snapshots are committed as nested trees; since E012 there are 7 corpora, including a versioned graph (`repo-graph`); E021 added `generated` (8) and E040 `structured` (9: CSV, NDJSON and JSON exports of the repository's history), and E055 `transcripts` (10: a coding agent's JSONL session log quoting the kernel's own sources, snapshotted as it grows). E081 added `multiformat` (11: the same 24 texts as md, html, docx and pdf, stored as **one** graph).
- **North-star measurement (since E077, reported beside the score):** `cargo run --release --example cross [read_weight]` stores the fixture cases' documents and `fixtures/multiformat` both as one graph and as one store per file type, at read weight 0.001 (storage only) unless given. It reports the kernel's and the baseline's bytes. Under the heavy workload, storage is a small share of cost, so the score barely sees compression or sharing across file types. This measurement does.
- **Score:** the `_summary.score`. Since E064 it is the geometric mean over corpora of the kernel's **monthly cost** over the baseline's (zstd + trained dictionary), under a stated workload at list prices:
  - `cost = stored bytes x storage price + sum over operations (times per month x (CPU time x CPU price + peak memory x wall time x memory price))`.
  - Prices (`examples/bench.rs`): storage $0.023 per GB-month, CPU $0.04 per vCPU-hour, memory $0.005 per GB-hour (approximate cloud on-demand list prices).
  - Workload `MONTHLY` ("heavy", per corpus per month):
    - ingest once;
    - repack and fsck daily (30 each);
    - 1,000 read passes over every file and 1,000 range reads of each;
    - 100 searches per read pass (100,000);
    - for a graph corpus, a read pass reads every version's graph.
  - Everything is measured, not modelled:
    - CPU time is process CPU time across all threads;
    - memory is peak heap from a counting allocator;
    - storage is stored bytes;
    - the baseline's operations are measured the same way: dictionary training and compression for ingest, decompression for reads, decompress and scan for search, decompress and hash for fsck.
  - Per corpus: `vs_baseline_cost`, with each side's cost by part under `ops.kernel_usd` and `ops.baseline_usd`.
  - The planner is given the exchange rate the score implies (E069). `read_weight = (1,000 + 100,000 decodes per month) x 1 ns per decoded byte x the CPU price / the storage price`, about 49 stored bytes per decoded byte. Decode cost counts zstd bytes at 1, bytes read as stored at 1/8 (BLAKE3 verification 0.25 ns/B against zstd's 2.2), bytes stored with a CRC-32 ("W", E074) at 1/64, and the other codecs by their measured weights.
  - **Timing noise:** repeated operations run at least 5 times and at least 300 ms, and the median is used. Two runs agree within about 5% per corpus and 1.5% overall, but never byte-for-byte.
  - A change is kept only if the score improves by more than the spread of two runs on both runs. Sizes and the deterministic proxies below must still be identical across runs.
  - `proxy_score` (E062: bytes + 0.001 x (decoded bytes + 100 x search)), `read_score` (E034–E061) and `bytes_score` (before E034) are deterministic and still reported. Scores across E034, E062 and E064 are not comparable.
  - Lower is better; below 1.0 means the kernel beats a strong conventional baseline.
  - Every corpus weighs the same, whatever its size.
- **Hard gates:** every file must reconstruct byte-for-byte, every search result must equal a scan of the original bytes, and `fsck` must be clean, or the run aborts. A failing gate is a failed experiment, never a score.
- **Reported, not scored:** per-corpus bytes, `vs_dict` ratios and times; since E028, read costs (`read_whole`, `read_4k`: bytes decompressed + bytes produced by functions, over every distinct file, for whole reads and 4 KiB range reads; deterministic).
  - Sizes are deterministic and reproduce exactly. Times are noisy.
  - A change that worsens time by more than 2× must say so in its result.
- **Tunables live in code** (constants such as `DEFAULT_READ_WEIGHT`, `GAP_DIFF`, `STAR_CANDIDATES`, dictionary sizing, the zstd levels `MEASURE_LEVEL` and `STORE_LEVEL`). An experiment edits them on its branch.

## The loop

1. **Question.** Name the gap, using a corpus and a `vs_dict` from the latest log entry.
2. **Read.** Search and read papers before changing code (see `docs/research/README.md`). Record what you read in a research note.
3. **Hypothesis.** One per experiment. Say what changes, which paper or measurement motivates it, and the expected effect on which corpora.
4. **Branch.** In git, `exp/<id>-<slug>` from the current best; in OpenResearch, an experiment node.
   - Never edit an experiment after its run has answered it. To try a variant, branch a child.
   - Grow downward: try a few options for one decision, then continue from the winner.
5. **Run** the fixed command and save its output.
   - For an external proposer (embedding order, LLM output, codebook), also run the swap test (E051). Rerun with the proposal removed, and with it shuffled. If every size is unchanged, the proposer carries no information for this corpus, whatever it costs to produce.
6. **Record** a result entry in `docs/experiments/log.md` (format below), add or update its row in `docs/experiments/registry.tsv` (see "Relevance"), and update the research note's hypothesis status: confirmed, refuted or not applicable.
7. **Keep or discard.**
   - A change is kept only if the score improves, or holds while adding a capability.
   - Kept changes merge into the working branch.
   - Discarded ones stay recorded: a refuted hypothesis is a result.

## Relevance (since E069)

The score has changed meaning three times: bytes only (E000–E033), bytes plus reads (E034–E061), a search proxy (E062–E063), and monthly cost (E064 on). A verdict reached under one objective may not hold under another. `registry.tsv` records, for every experiment:
- its kind and verdict;
- where it is recorded (`main`, or the branch holding its only log entry);
- the objective it was measured under;
- the mechanisms it touched;
- whether its verdict hinges on the objective's storage-versus-read weights (`weight_sensitive`).

From those, `examples/registry.rs` derives:
- **relevance:** `current` (measured under the current objective), `holds` (no storage-versus-read trade-off in its verdict, such as a fix or a pure speedup), `stale`, or `reference` (benchmark changes);
- **action for stale entries:** `ablate` a kept mechanism (measure what it still contributes under the current objective, and delete it if nothing), or `retest` a rejected one;
- **priority:** `high` if it touches a mechanism that matters under the current objective (`WEIGHTY`).

The registry is also stored in IKAM itself: `cargo run --release --example registry [STORE_DIR]` (default `target/experiments.ikam`) commits it as a versioned graph. Experiments, objectives, mechanisms, verdicts, relevance and actions are nodes; each experiment's log entry is stored as content. The tool then prints the stale list read back from the stored graph and checks the store with fsck. Each run with a changed registry commits a new version, so how relevance shifts over time is kept as history.

Workflow:
- Every new experiment adds a row.
- A rerun updates the row's `objective` (and its verdict, if it changed) and adds a log entry that names the original.
- When the score's objective changes, set `CURRENT` (and `WEIGHTY`) in `examples/registry.rs`.

## Result entry format (`log.md`)

```
### E<nnn> <slug>
- Branch / parent: exp/<nnn>-<slug> / E<parent>
- Hypothesis: <one sentence> (source: <paper id or measurement>)
- Change: <files and what changed>
- Result: score <old> -> <new>; per corpus vs_dict: md a->b, pdf ..., office ..., invoices ..., synthetic ..., repo ...; time <total_ms>
- Verdict: kept | discarded | inconclusive, and why
```

## With OpenResearch (local machine)

1. Install it (`curl -LsSf https://openresearch.sh/install.sh | sh`) and run `orx up`. Import this repository as a project.
2. Set the project's run command once: `orx project edit <projectId> --run-command 'cd packages/kernel && cargo run --release --example bench -- --json'`.
3. Add the IKAM experiment skill: `orx skills add docs/experiments/SKILL.md`.
4. Use `orx create-experiment` for each hypothesis, and `orx discover` / `orx paper` for reading.
5. Copy results into `log.md` so they live with the code, not only in OpenResearch's local database.

In a cloud session without the dashboard, follow the same loop with git branches. `orx` builds from source with `cargo build --release` if its literature commands are wanted.
