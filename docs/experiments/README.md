# Experiments

How we improve the kernel: one fixed command, one score, hard gates, one hypothesis per branch, and every hypothesis grounded in something we read or measured. Compatible with [OpenResearch](https://github.com/alphaXiv/OpenResearch) (`orx`), but it does not depend on it. Any agent or person can follow it with git alone.

## The contract

- **Run command (fixed, identical on every experiment):** `cd packages/kernel && cargo run --release --example bench -- --json`, also available as `mise run experiment`.
  - It prints one JSON line per corpus, then a `_summary` line.
  - Never pass extra flags or environment variables to vary behaviour. Change committed code instead.
- **Benchmark changes** (corpus or how it is committed) are their own log entries that re-measure the current kernel, and scores across them are not compared. Since E009 snapshots are committed as nested trees; since E012 there are 7 corpora, including a versioned graph (`repo-graph`).
- **Score:** the `_summary.score`, which is the geometric mean over corpora of `repack bytes / zstd+trained-dictionary baseline bytes`.
  - Lower is better; below 1.0 means the kernel beats a strong conventional baseline.
  - Every corpus weighs the same, whatever its size.
- **Hard gates:** every file must reconstruct byte-for-byte and `fsck` must be clean, or the run aborts. A failing gate is a failed experiment, never a score.
- **Reported, not scored:** per-corpus bytes, `vs_dict` ratios and times; since E028, read costs (`read_whole`, `read_4k`: bytes decompressed + bytes produced by functions, over every distinct file, for whole reads and 4 KiB range reads; deterministic).
  - Sizes are deterministic and reproduce exactly. Times are noisy.
  - A change that worsens time by more than 2× must say so in its result.
- **Tunables live in code** (constants such as `DEFAULT_READ_WEIGHT`, `GAP_DIFF`, `STAR_CANDIDATES`, dictionary sizing). An experiment edits them on its branch.

## The loop

1. **Question.** Name the gap, using a corpus and a `vs_dict` from the latest log entry.
2. **Read.** Search and read papers before changing code (see `docs/research/README.md`). Record what you read in a research note.
3. **Hypothesis.** One per experiment. Say what changes, which paper or measurement motivates it, and the expected effect on which corpora.
4. **Branch.** In git, `exp/<id>-<slug>` from the current best; in OpenResearch, an experiment node.
   - Never edit an experiment after its run has answered it. To try a variant, branch a child.
   - Grow downward: try a few options for one decision, then continue from the winner.
5. **Run** the fixed command and save its output.
6. **Record** a result entry in `docs/experiments/log.md` (format below), and update the research note's hypothesis status: confirmed, refuted or not applicable.
7. **Keep or discard.**
   - A change is kept only if the score improves, or holds while adding a capability.
   - Kept changes merge into the working branch.
   - Discarded ones stay recorded: a refuted hypothesis is a result.

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
