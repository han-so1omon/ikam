# Research note: intermediate representations as compression (E192 to E196, 2026-11-11)

**[read]** means read in this repository (notes, log, code). **[inf]** means my own inference. OpenResearch cannot be installed in the cloud environment; no new paper was read for this note.

## Question

Eric (Nov 11): how can abstractions (intermediate representations) find connections that aid dedup and compression, and what other benefits follow? The test case is Mercante's run reports (markdown lead-lag reports, one per research run), which the kernel stored at 0.16 of raw against xz -9's 0.11 in the report-copy demo.

## Sources

- LogShrink (Li et al., ICSE 2024), already summarised in `kernel-direction.md` #15 [read there]: template plus variables, variables by column; column layout 36 to 103% smaller than row layout. H1 there (columnar fillers in repack) is still open; E193 measures the same idea outside repack.
- E161, E165 [read]: a typed TableIR reproduces csv, json and md exactly and stores them in about half the bytes; cell types are what it must keep.
- E143, E144, `template.rs` [read]: the kernel's byte-level templates (anti-unification at repack). They find shared layout, but not that a value repeats in the same row of the next run.
- mercante-personal `leadlag/cli.py render()` (branch exp/ikam-reports) [read]: the report layout the synthetic corpora copy.

## Hypotheses and status

- Templates over lexed lines beat xz: **refuted for single-format reports** (1.03, E192), confirmed across formats (0.69).
- Columns and numeric deltas: **confirmed by E193** (0.67 to 0.84 on reports), not for very small tables.
- Row keys for reruns: **confirmed by E193** on reruns over the same data (0.591), neutral to slightly worse on random values.
- Canonical forms by cell diff across same-stem files: **confirmed by E194** (exports 0.483).
- In the kernel: **confirmed by E195** for reports and tables; one-file reads become full decodes.

## Unknowns

- Real Mercante reports: the corpora are synthetic. How often a real rerun repeats its numbers decides whether row keys pay [inf]. The demo run on the Mac (asked for Nov 10) can answer it with the real reports.
- Prose: the codec loses 7 to 25% there; an encoder that keeps a file out of the IR when it has few cells per line would avoid it [inf], not built.
- Reads: streams cut into blocks (say 32 runs) would bound a one-file read; not built.
