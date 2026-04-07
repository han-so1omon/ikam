# IKAM Repo Extraction Manifest

This repository is a fresh snapshot extraction from the larger Narraciones monorepo.

Included:
- `AGENTS.md`
- `packages/ikam`
- `packages/modelado`
- `packages/interacciones`
- `packages/mcp-ikam`
- `packages/ikam-graph-viewer`
- `packages/test/ikam-perf-report`
- `tests/fixtures`
- Curated IKAM docs under `docs/ikam`
- `docs/benchmarks/ikam-perf-report.md`
- `pytest.ini`

Excluded:
- `packages/narraciones`
- `services/`
- `frontend/`
- `admin-frontend/`
- Generated benchmark reports under `packages/test/ikam-perf-report/reports/`
- Local virtualenvs, cache directories, egg-info metadata, and OS junk files
- Historical root patch scripts and unrelated monorepo artifacts

Migration rules:
- Preserve current `packages/`, `tests/fixtures`, and `packages/test/ikam-perf-report/preseed` paths during stabilization.
- Treat this repo as the forward-only source of truth for IKAM work after extraction.
- Use the old monorepo only as historical reference during transition debugging.
