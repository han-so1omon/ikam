# IKAM Performance Report

Minimal performance reporting stack for IKAM benchmarks, decision traces, and graph exploration.

## Overview

This stack provides:
- A minimal FastAPI service for running benchmarks and serving graph/decision APIs
- A standalone Vite app for graph exploration and dashboard views
- A docker-compose profile (`ikam-perf-report`) that runs Postgres, HugeGraph, MinIO, API, and graph viewer

## Run the stack

```bash
docker compose --profile ikam-perf-report up -d
```

Services:
- API: http://localhost:8040
- Graph Viewer: http://localhost:5179
- HugeGraph: http://localhost:28080
- MinIO: http://localhost:19000 (console: http://localhost:19001)
- Postgres: localhost:55432

## API endpoints

```text
GET  /health
GET  /graph/summary
GET  /graph/nodes
GET  /graph/edges
GET  /graph/decisions/{run_id}
POST /benchmarks/run?project_size=small
GET  /benchmarks/runs
GET  /benchmarks/runs/{run_id}
```

## Dataset benchmarks

The benchmark generator includes 4 SME project seeds and supports `small`, `medium`, and `large` sizes. Use `custom_prompt` to override the base doc prompt when needed.

## Report export

The report export helper returns Markdown and emits JSON/CSV payloads in-memory.

```python
from ikam_perf_report.reports.exporter import export_report

markdown = export_report(run_id="demo")
```

## Development notes

Environment defaults are set in the compose profile. Override ports using:

```bash
IKAM_PERF_REPORT_API_PORT=8041 IKAM_GRAPH_VIEWER_PORT=5180 \
  docker compose --profile ikam-perf-report up -d
```
