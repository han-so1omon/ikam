# IKAM

This repository is the workspace for the Internal Knowledge Artifacts Model (IKAM).

The purpose of IKAM is to be a semantic graph for filesystem-like operations.

IKAM key features are:
- Takes in 'artifacts', which are useful render-able heads of data structures (e.g. files).
- Breaks down 'artifacts' into 'fragments', which are dedup-able components of 'artifacts'
- Organizes 'fragments' into a graph
- Re-renders 'fragments' back into the original 'artifacts', losslessly when possible

In essence, IKAM breaks down generic information corpuses into a computational graph. The thesis is that
lossless re-renderability will aid in automating complex processes via AI agent runtime systems.

Included here:
- Active IKAM packages under `packages/`
- The package-level integration stack in `packages/test/ikam-perf-report`
- Shared test fixtures under `tests/fixtures`
- Curated IKAM normative documentation under `docs/ikam`
