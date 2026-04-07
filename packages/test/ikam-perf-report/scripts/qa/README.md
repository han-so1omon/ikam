# Perf Report Stagehand QA

This directory contains QA automation for the IKAM perf report viewer.

## Single run

```bash
python packages/test/ikam-perf-report/scripts/stagehand_perf_report.py
```

Key assertions:
- Runs -> Graph -> Wiki workflow succeeds
- Graph render is non-blank
- Viewport remains stable (no runaway growth)
- Explainability panels are visible (legend, inspector, semantic explorer)
- Wiki generation succeeds and includes `IKAM Breakdown`
- Wiki cards render `model` and `harness` metadata
- A non-empty video artifact is produced

Artifacts are written under `${IKAM_STAGEHAND_OUT:-/tmp/ikam-perf-stagehand}`.

## Case matrix

```bash
bash packages/test/ikam-perf-report/scripts/qa/run_perf_stagehand_matrix.sh
```

Defaults to:
- `s-construction-v01`
- `m-construction-v01`
- `l-construction-v01`

Override with:

```bash
IKAM_STAGEHAND_CASES="s-construction-v01,l-construction-v01" \
bash packages/test/ikam-perf-report/scripts/qa/run_perf_stagehand_matrix.sh
```

Summary output is written to `summary.jsonl` and includes case-level output JSON and video artifact paths.

## Smoke mode

```bash
IKAM_STAGEHAND_MODE=smoke python packages/test/ikam-perf-report/scripts/stagehand_perf_report.py
```

Smoke mode runs a shorter viewport sampling path while preserving the same correctness checks.
