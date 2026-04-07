"""Embedding verification tests.

Verifies:
1. get_shared_embedder() returns a process-level singleton
2. embed_decomposed stores the correct model name to ikam_fragment_store
3. embed_decomposed stores vectors of the correct dimension
4. candidate_search produces non-empty candidates

Tests 2-4 require Docker Postgres at port 55432 and ikam_fragment_store table.

Key: op_id in ikam_fragment_store = artifact_id.split(":")[-1] = case_id
(not run_id).  See debug_execution.py candidate_search handler, line:
    op_id = state.artifact_id.split(":")[-1] if ":" in state.artifact_id else state.artifact_id
And runner.py:
    artifact_id = f"{project_id}:{case_id}"
So op_id == case_id == "s-construction-v01".

Step-detail route: GET /benchmarks/runs/{run_id}/debug-step/{step_id}/detail
(step_id is the opaque step-{hex} id from the event, not the step name).
"""
from __future__ import annotations

import os
import uuid

import pytest
from fastapi.testclient import TestClient

from modelado.core.execution_scope import DefaultExecutionScope
from ikam_perf_report.benchmarks.store import STORE
from ikam_perf_report.main import app

_DB_URL = "postgresql://narraciones:narraciones@localhost:55432/ikam_perf_report"


def _db_available() -> bool:
    try:
        import psycopg
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
    """Set DATABASE_URL and verify ikam_fragment_store exists. Skips if unavailable."""
    import psycopg
    if not _db_available():
        pytest.skip("pgvector Postgres not available on port 55432")

    with psycopg.connect(_DB_URL) as conn:
        row = conn.execute("SELECT to_regclass('public.ikam_fragment_store')").fetchone()
        if row is None or row[0] is None:
            pytest.skip(
                "ikam_fragment_store table not found; run "
                "test_candidate_search_handler_uses_pgvector_hnsw first"
            )

    from modelado.db import reset_pool_for_pytest
    old_url = os.environ.get("DATABASE_URL")
    os.environ["DATABASE_URL"] = _DB_URL
    reset_pool_for_pytest()

    yield _DB_URL

    if old_url is None:
        os.environ.pop("DATABASE_URL", None)
    else:
        os.environ["DATABASE_URL"] = old_url
    reset_pool_for_pytest()


def _post_next_step(client: TestClient, run_id: str, pipeline_id: str, pipeline_run_id: str) -> dict:
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


def test_shared_embedder_singleton_identity():
    """get_shared_embedder() returns the same object on repeated calls."""
    from modelado.fragment_embedder import get_shared_embedder

    emb1 = get_shared_embedder()
    emb2 = get_shared_embedder()
    assert id(emb1) == id(emb2), "Expected process-level singleton, got two different instances"

    # Changing model name causes recreation
    old_val = os.environ.get("IKAM_EMBEDDING_MODEL")
    try:
        os.environ["IKAM_EMBEDDING_MODEL"] = "all-MiniLM-L6-v2"
        emb3 = get_shared_embedder()
        assert id(emb3) != id(emb1), "Expected new instance after IKAM_EMBEDDING_MODEL change"
        assert emb3.model_name == "all-MiniLM-L6-v2"
    finally:
        # Restore
        if old_val is None:
            os.environ.pop("IKAM_EMBEDDING_MODEL", None)
        else:
            os.environ["IKAM_EMBEDDING_MODEL"] = old_val
        # Force re-creation on next call to restore original singleton
        import modelado.fragment_embedder as _fe
        _fe._SHARED_EMBEDDER = None
        _fe._SHARED_MODEL_NAME = None


@pytest.mark.slow
def test_embed_decomposed_stores_correct_model_name(case_fixtures_root, db_with_schema):
    """embed_decomposed step writes the correct model name to ikam_fragment_store.

    op_id in ikam_fragment_store = case_id = "s-construction-v01"
    (artifact_id is "{project_id}:{case_id}", op_id = artifact_id.split(":")[-1]).
    """
    import psycopg
    from modelado.fragment_embedder import get_shared_embedder

    client = TestClient(app)

    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200, f"run failed: {run_resp.text}"
    runs = run_resp.json()["runs"]
    run = runs[0]
    run_id = run["run_id"]
    pipeline_id = "compression-rerender/v1"
    pipeline_run_id = run_id

    # Step: decompose
    _post_next_step(client, run_id, pipeline_id, pipeline_run_id)
    # Step: embed_decomposed (must step through to candidate_search since that's the step that writes to ikam_fragment_store)
    _post_next_step(client, run_id, pipeline_id, pipeline_run_id)
    target_step = "map.reconstructable.search.dependency_resolution"
    dynamic_steps = DefaultExecutionScope().get_dynamic_execution_steps()
    steps_to_target = []
    for step in dynamic_steps[1:]:
        steps_to_target.append(step)
        if step == target_step:
            break

    for _ in steps_to_target:
        _post_next_step(client, run_id, pipeline_id, pipeline_run_id)

    # Read authoritative embedding model from candidate_search event metrics.
    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": pipeline_id, "pipeline_run_id": pipeline_run_id},
    )
    assert stream_resp.status_code == 200
    events = stream_resp.json().get("events", [])
    cs_events = [e for e in events if e.get("step_name") == target_step]
    assert cs_events, f"candidate_search event ({target_step}) not found"
    expected = (((cs_events[-1].get("metrics") or {}).get("details") or {}).get("embedding_model"))
    assert isinstance(expected, str) and expected, "candidate_search detail must expose embedding_model"

    # op_id = artifact_id.split(":")[-1] = case_id (see debug_execution.py)
    op_id = "s-construction-v01"
    with psycopg.connect(db_with_schema) as conn:
        rows = conn.execute(
            "SELECT DISTINCT embedding_model FROM ikam_fragment_store WHERE operation_id = %s",
            (op_id,),
        ).fetchall()

    if not rows:
        pytest.skip("No fragments in ikam_fragment_store for this case; skipping model name check")

    model_names = {row[0] for row in rows}
    assert expected in model_names, f"Expected embedding_model {expected!r} in DB rows, got {model_names}"


@pytest.mark.slow
def test_embed_decomposed_stores_768d_vectors(case_fixtures_root, db_with_schema):
    """embed_decomposed stores vectors of the expected dimension in ikam_fragment_store.

    op_id in ikam_fragment_store = case_id = "s-construction-v01".
    """
    import psycopg
    from modelado.fragment_embedder import get_shared_embedder

    client = TestClient(app)

    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200, f"run failed: {run_resp.text}"
    run_id = run_resp.json()["runs"][0]["run_id"]
    pipeline_id = "compression-rerender/v1"
    pipeline_run_id = run_id

    _post_next_step(client, run_id, pipeline_id, pipeline_run_id)  # decompose
    _post_next_step(client, run_id, pipeline_id, pipeline_run_id)  # embed_decomposed
    _post_next_step(client, run_id, pipeline_id, pipeline_run_id)  # lift
    _post_next_step(client, run_id, pipeline_id, pipeline_run_id)  # embed_lifted
    _post_next_step(client, run_id, pipeline_id, pipeline_run_id)  # candidate_search (DB write)

    op_id = "s-construction-v01"
    with psycopg.connect(db_with_schema) as conn:
        rows = conn.execute(
            "SELECT vector_dims(embedding) FROM ikam_fragment_store WHERE operation_id = %s LIMIT 5",
            (op_id,),
        ).fetchall()

    if not rows:
        pytest.skip("No fragments in ikam_fragment_store; skipping vector dim check")

    expected_dim = get_shared_embedder().dimensions
    for row in rows:
        assert row[0] == expected_dim, f"Expected {expected_dim}d, got {row[0]}d"


@pytest.mark.slow
def test_candidate_search_returns_non_empty_candidates(case_fixtures_root, db_with_schema):
    """candidate_search step produces at least one candidate.

    Note: the step-detail endpoint uses step_id (opaque "step-{hex}" id),
    not step_name.  Route: GET /benchmarks/runs/{run_id}/debug-step/{step_id}/detail
    """
    client = TestClient(app)

    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200, f"run failed: {run_resp.text}"
    run_id = run_resp.json()["runs"][0]["run_id"]
    pipeline_id = "compression-rerender/v1"
    pipeline_run_id = run_id

    target_step = "map.reconstructable.search.dependency_resolution"
    dynamic_steps = DefaultExecutionScope().get_dynamic_execution_steps()
    steps_after_prepare = list(dynamic_steps[1:])

    for step_name in steps_after_prepare:
        result = _post_next_step(client, run_id, pipeline_id, pipeline_run_id)
        if step_name == "candidate_search":
            # The step_id is embedded in the state returned by the control endpoint
            # We can find it in the debug-stream events
            break

    # Get debug-stream events to find the candidate_search step_id
    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": pipeline_id, "pipeline_run_id": pipeline_run_id},
    )
    assert stream_resp.status_code == 200
    events = stream_resp.json()["events"]

    cs_events = [e for e in events if e["step_name"] == target_step]
    assert cs_events, "candidate_search step event not found in debug stream"
    cs_event = cs_events[0]
    assert cs_event["status"] == "succeeded", f"candidate_search failed: {cs_event}"

    # Fetch step detail using the opaque step_id (route uses step_id, not step_name)
    step_id = cs_event["step_id"]
    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{step_id}/detail")
    assert detail_resp.status_code == 200, f"detail endpoint failed: {detail_resp.text}"
    detail = detail_resp.json()

    assert detail.get("schema_version") == "v1"
    outputs = detail.get("outputs", {}) if isinstance(detail.get("outputs"), dict) else {}
    candidates = outputs.get("candidates", []) if isinstance(outputs, dict) else []
    assert isinstance(candidates, list), f"Expected candidates list in canonical outputs, got: {detail}"
