# IKAM Perf Report API

Minimal FastAPI service for IKAM performance reporting.

Use the dedicated stack file to run this package independently:

`docker compose -f docker-compose.yml up -d`

The stack now includes a one-shot `ikam-perf-report-init` service that bootstraps the DB schema and preloads the perf-report preseed registry/workflow/operator fragments before `ikam-perf-report-api` starts. For normal local browser smoke tests, you no longer need to run separate manual schema/preload commands first.

## Executor sidecar in the stack

- `ikam-executor-sidecar` runs `modelado.executors.sidecar:app` inside the perf-report Compose stack.
- `ikam-perf-report-api` reaches it over the internal Compose network via `IKAM_EXECUTOR_SIDECAR_URL=http://ikam-executor-sidecar:8000`.
- The sidecar exposes `GET /health` and the Compose healthcheck uses that readiness endpoint.
- Sidecar-path preload/stack checks now point at the consolidated preseed root `packages/test/ikam-perf-report/preseed`, and `modelado.plans.preload.preload_fixtures(...)` loads both generated declaration/workflow fragments and `preseed/compiled/*.yaml` sidecar fixtures from that root.
- Stagehand coverage is still centered on the existing debug execution flow, not end-to-end sidecar dispatch yet.
- The current perf-report step runner flows through `ikam_perf_report/api/benchmarks.py` and `ikam.forja.debug_execution`, so Stagehand validates debug-step orchestration and graph rendering rather than the graph-compiler sidecar path.

## Parallel broker-backed executor runtimes

- The perf-report stack now also includes parallel broker-backed runtimes:
  - `ikam-python-executor-runtime`
  - `ikam-ml-executor-runtime`
- These runtimes use Redpanda topics instead of the legacy HTTP `/execute` path.
- The stack now provisions:
  - `redpanda`
  - `kafka-init`
  - broker topics: `execution.requests`, `workflow.events`, `execution.progress`, `execution.results`
- Runtime handlers live in `packages/test/ikam-perf-report/ikam_perf_report/executor_handlers.py`.
- The legacy `ikam-executor-sidecar` remains in place for compatibility with the current HTTP-based perf-stack flows.

### Running executor runtime broker E2E tests

Bring up the minimal broker/runtime stack, run the opt-in Kafka E2E tests, then tear it down:

```bash
sh packages/test/ikam-perf-report/run_executor_runtime_e2e.sh
sh packages/test/ikam-perf-report/stop_executor_runtime_e2e.sh
```

The start script:
- starts `redpanda`, `kafka-init`, `ikam-python-executor-runtime`, and `ikam-ml-executor-runtime`
- enables `ENABLE_EXECUTOR_RUNTIME_KAFKA_E2E_TESTS=1`
- points tests at `KAFKA_BOOTSTRAP_SERVERS=127.0.0.1:19092`
- runs `packages/test/ikam-perf-report/tests/test_executor_runtime_broker_e2e.py`

The broker E2E test file currently covers:
- `python.load_documents` over `executor://python-primary`
- `ml.embed` over `executor://ml-primary`

## Reset and apply behavior

- `POST /benchmarks/run?reset=true` routes graph reset through `modelado.ikam_reset.reset_ikam_graph_state`.
- Postgres reset is always attempted.
- Optional backend resets are attempted only when configured:
  - HugeGraph: `HUGEGRAPH_URL` (or `HUGEGRAPH_BASE_URL`)
  - Redis: `REDIS_URL`
  - MinIO: `NARRACIONES_STORAGE_ENDPOINT`, `NARRACIONES_STORAGE_ACCESS_KEY`, `NARRACIONES_STORAGE_SECRET_KEY`, `NARRACIONES_STORAGE_BUCKET`

- `POST /benchmarks/merge?graph_ids=...&apply=true` persists merge updates to in-memory graph state.
- Persisted updates include:
  - edge upserts (`edge_updates`)
  - relational fragment upserts (`relational_fragment_updates`)

### IKAM ingestion PoC (package-only)

- `POST /benchmarks/ingest-poc` runs Stage 2 + 3 + Process C entirely from packages:
  - Stage source/normalized/enriched fragments into `ikam_staging_fragments`
  - Validate staged rows
  - Promote into `ikam_fragments` twice (first insert, second CAS hit pass)
  - Return `ikam_metrics` deltas (`cas_hits_delta`, `cas_misses_delta`, `cas_hit_rate`)

### Case-suite ingestion report

- Run each case individually from `tests/fixtures/cases` and generate:
  - per-case JSON stats (ingestion size, graph size, dedup, normalization, enrichment)
  - per-case Mermaid graph files (`.mmd`)
  - suite summary JSON

```bash
PYTHONPATH="packages/ikam/src:packages/modelado/src:packages/interacciones/src:packages/test/ikam-perf-report" \
uv run --with "psycopg[binary]" --with fastapi --with pydantic --with prometheus-client --with python-dotenv \
python packages/test/ikam-perf-report/scripts/run_case_suite_ingestion.py
```

## Cases and UI flow

- `GET /benchmarks/cases` returns discoverable benchmark fixtures used by the perf report UI:
  - `case_id`
  - `domain`
  - `size_tier`
- `POST /benchmarks/run?case_ids=...` executes one or more selected cases and returns `runs[]` with explicit `run_id` and `graph_id`.

The graph viewer follows a tabbed workflow:

1. **Runs**: select fixture cases, optionally reset, execute run(s).
2. **Graph**: inspect metrics and decision trace scoped to selected run/graph.
3. **Merge**: select graph IDs from prior runs and run proposal or persisted apply flow.

### Ad-hoc evaluation report (manual run)

Use the same Oráculo pipeline used by the UI.

```bash
PYTHONPATH=packages/ikam/src:packages/modelado/src:packages/test/ikam-perf-report \
python - <<'PY'
from dotenv import load_dotenv
from ikam_perf_report.api.evaluations import run_evaluation
import json

load_dotenv("packages/test/ikam-perf-report/.env")
payload = run_evaluation("s-local-retail-v01")

print("\n=== Rendered Report ===\n")
print(payload["rendered"])
print("\n=== Summary (JSON) ===\n")
print(json.dumps(payload["report"], indent=2))
PY
```

## Soft Glass visual system notes

- The UI uses a hybrid visual model:
  - WebGL graph primitives use soft-glass-adjacent rendering (halo emphasis, translucent group bubbles, readable dimming floor).
  - DOM overlays use glassmorphism (`backdrop-filter`, frosted surfaces, soft border highlights).
- Safe visual tuning points:
  - `packages/ikam-graph-viewer/src/theme.ts` for graph palette, dimmed visibility floor, and group bubble style.
  - `packages/ikam-graph-viewer/src/app/styles.css` for glass panel tokens and overlay chrome.
  - `packages/ikam-graph-viewer/src/GraphView.ts` for WebGL material/opacities and camera motion polish.
- Guardrails:
  - Keep pan/zoom/select/search/focus semantics unchanged while tuning visuals.
  - Preserve readability in dense graph states (dimmed context visible, active path dominant).
  - Do not move blur logic into raw WebGL primitives; CSS blur is overlay-only.

## Stagehand QA

Use Stagehand/Playwright validation for graph explainability and wiki behavior:

```bash
python packages/test/ikam-perf-report/scripts/stagehand_perf_report.py
```

Expected artifacts:
- output JSON report
- full-page screenshot
- console log
- screen recording (video)

Run matrix across case tiers:

```bash
bash packages/test/ikam-perf-report/scripts/qa/run_perf_stagehand_matrix.sh
```

See `packages/test/ikam-perf-report/scripts/qa/README.md` for full details.

## Stagehand Debug Validation Suite

The debug validation suite validates compression-rerender debug behavior using six independent scenarios with deterministic seeding and dual-layer checks (deterministic contracts + agentic checkpoints).

### Required environment gates

Set both gates before running scenario seeding or injection controls:

```bash
export IKAM_PERF_REPORT_TEST_MODE=1
export IKAM_ALLOW_DEBUG_INJECTION=1
```

### Scenario seed endpoint

- `POST /benchmarks/test/seed-scenario`
  - deterministic `scenario_key`
  - narrow `overrides` only

### Debug control endpoint

- `POST /benchmarks/runs/{run_id}/control`
  - `set_mode`, `pause`, `resume`, `next_step`
  - `inject_verify_fail` (gated)

### Contract-focused test bundle

```bash
PYTHONPATH=packages/ikam/src:packages/modelado/src:packages/interacciones/src:packages/test/ikam-perf-report \
python -m pytest \
  packages/test/ikam-perf-report/tests/test_stagehand_seed_api.py \
  packages/test/ikam-perf-report/tests/test_stagehand_validation_contracts.py \
  packages/test/ikam-perf-report/tests/test_stagehand_director_structure.py \
  packages/test/ikam-perf-report/tests/test_stagehand_scenarios_core_retry.py \
  packages/test/ikam-perf-report/tests/test_stagehand_scenarios_env_eval.py \
  packages/test/ikam-perf-report/tests/test_stagehand_agentic_mismatch.py \
  packages/test/ikam-perf-report/tests/test_stagehand_artifact_bundle.py
```

### Full stagehand run (interactive/browser)

```bash
python packages/test/ikam-perf-report/scripts/stagehand_perf_report.py
```

The script writes scenario artifacts and verdict metadata under the configured output directory.
