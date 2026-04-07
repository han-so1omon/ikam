"""Task 10: End-to-end integration validation.

Full workflow: run cases → step through entire pipeline → verify completion →
check debug-state endpoint returns terminal state.

Requires Docker Postgres at port 55432 (packages/test/ikam-perf-report/docker-compose.yml).
Skipped automatically when DB is not reachable.
"""
from __future__ import annotations

import os
import uuid
from time import perf_counter

import psycopg
import pytest
from fastapi.testclient import TestClient

from modelado.core.execution_scope import DefaultExecutionScope
from ikam_perf_report.benchmarks.store import STORE
from ikam_perf_report.main import app

_DB_URL = "postgresql://narraciones:narraciones@localhost:55432/ikam_perf_report"


def _db_available() -> bool:
    try:
        with psycopg.connect(_DB_URL, connect_timeout=3) as conn:
            conn.execute("SELECT 1")
        return True
    except Exception:
        return False


@pytest.fixture(autouse=True)
def _reset_store() -> None:
    STORE.reset()
    yield
    STORE.reset()


@pytest.fixture()
def db_with_schema():
    """Set DATABASE_URL to the Docker Postgres and verify ikam_fragment_store exists.

    Skips the test if the DB is not reachable or ikam_fragment_store is missing
    (the table is created by test_benchmark_runner.py::test_candidate_search_handler_uses_pgvector_hnsw
    or by running the docker-compose stack).  Resets the modelado connection
    pool before and after so it picks up the patched DATABASE_URL.
    """
    if not _db_available():
        pytest.skip("pgvector Postgres not available on port 55432")

    # Verify ikam_fragment_store exists; skip if not (needs prior schema init)
    with psycopg.connect(_DB_URL) as conn:
        row = conn.execute(
            "SELECT to_regclass('public.ikam_fragment_store')"
        ).fetchone()
        table_exists = row is not None and row[0] is not None
    if not table_exists:
        pytest.skip("ikam_fragment_store table not found; run test_candidate_search_handler_uses_pgvector_hnsw first")

    from modelado.db import reset_pool_for_pytest

    old_url = os.environ.get("DATABASE_URL")
    os.environ["DATABASE_URL"] = _DB_URL
    reset_pool_for_pytest()

    yield _DB_URL

    # Teardown: restore env and pool
    if old_url is None:
        os.environ.pop("DATABASE_URL", None)
    else:
        os.environ["DATABASE_URL"] = old_url
    reset_pool_for_pytest()


def _post_next_step(client: TestClient, run_id: str, pipeline_id: str, pipeline_run_id: str) -> dict:
    """Send a next_step control command and return the parsed JSON body."""
    resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": str(uuid.uuid4()),
            "action": "next_step",
            "pipeline_id": pipeline_id,
            "pipeline_run_id": pipeline_run_id,
        },
    )
    assert resp.status_code == 200, f"control next_step failed: {resp.text}"
    return resp.json()


def test_full_debug_pipeline_workflow(case_fixtures_root, db_with_schema) -> None:
    """Complete workflow:

    1. POST /benchmarks/run → prepare-only, <500ms
    2. POST /control next_step × 10 → all steps after prepare_case complete
    3. Step stream has 11 events in canonical order (prepare_case … project_graph)
    4. /runs/{run_id}/debug-state returns terminal execution_state
    """
    client = TestClient(app)

    # --- 1. Run cases (prepare only) ---
    t0 = perf_counter()
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    elapsed_ms = (perf_counter() - t0) * 1000
    assert run_resp.status_code == 200, f"run failed: {run_resp.text}"
    assert elapsed_ms < 500, f"prepare-only run took {elapsed_ms:.0f}ms, must be <500ms"

    body = run_resp.json()
    runs = body["runs"]
    assert len(runs) >= 1, "Expected at least one run"
    run = runs[0]

    run_id = run["run_id"]
    debug_state_init = run.get("debug_state", {})
    assert debug_state_init.get("current_step_name") == "prepare_case"
    assert debug_state_init.get("execution_state") == "paused"

    # pipeline_id and pipeline_run_id are always these values (see runner.py)
    pipeline_id = "compression-rerender/v1"
    pipeline_run_id = run_id  # runner sets pipeline_run_id == run_id

    # --- 2. Step through the 10 remaining steps (decompose … project_graph) ---
    # Note: prepare_case was already executed during run setup and recorded as
    # the first event in the debug stream. We send next_step 10 more times.
    dynamic_steps = DefaultExecutionScope().get_dynamic_execution_steps()
    steps_after_prepare = list(dynamic_steps[1:])
    assert len(steps_after_prepare) == len(dynamic_steps) - 1

    step_events: list[dict] = []
    for _ in steps_after_prepare:
        result = _post_next_step(client, run_id, pipeline_id, pipeline_run_id)
        assert result["status"] == "ok", f"next_step returned non-ok: {result}"
        step_events.append(result)  # kept for debugging if assertions fail below

    # --- 3. Canonical order via debug-stream ---
    # The stream includes prepare_case (recorded at run time) + the 10 stepped events.
    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": pipeline_id, "pipeline_run_id": pipeline_run_id},
    )
    assert stream_resp.status_code == 200
    stream_body = stream_resp.json()
    assert stream_body["status"] == "ok", f"debug-stream status: {stream_body}"

    events = stream_body["events"]
    assert len(events) == len(dynamic_steps), (
        f"Expected {len(dynamic_steps)} events (all dynamic steps), got {len(events)}"
    )

    expected_step_names = list(dynamic_steps)

    step_names = [e["step_name"] for e in events]
    assert step_names == expected_step_names, (
        f"Step names out of dynamic order.\n"
        f"  got:      {step_names}\n"
        f"  expected: {expected_step_names}"
    )

    # All events must have succeeded
    for ev in events:
        assert ev["status"] == "succeeded", (
            f"Step {ev['step_name']} did not succeed: status={ev['status']}"
        )

    # --- 4. Terminal state via /debug-state ---
    state_resp = client.get(f"/benchmarks/runs/{run_id}/debug-state")
    assert state_resp.status_code == 200
    state_body = state_resp.json()
    assert state_body["status"] == "ok", f"debug-state status: {state_body}"
    assert state_body["execution_state"] in ("completed", "succeeded"), (
        f"Expected terminal execution_state, got: {state_body['execution_state']!r}"
    )
    expected_terminal_step = dynamic_steps[-1]
    assert state_body["current_step_name"] == expected_terminal_step, (
        f"Expected current_step_name={expected_terminal_step!r}, "
        f"got {state_body['current_step_name']!r}"
    )
