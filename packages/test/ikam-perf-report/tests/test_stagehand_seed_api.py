
from __future__ import annotations
import pytest
pytest.importorskip('playwright')

from fastapi.testclient import TestClient

from ikam_perf_report.main import app


def test_seed_scenario_endpoint_is_gated_without_env_flags(monkeypatch) -> None:
    monkeypatch.delenv("IKAM_PERF_REPORT_TEST_MODE", raising=False)
    monkeypatch.delenv("IKAM_ALLOW_DEBUG_INJECTION", raising=False)

    client = TestClient(app)
    response = client.post(
        "/benchmarks/test/seed-scenario",
        json={"scenario_key": "core_stream_baseline"},
    )
    assert response.status_code == 403
    payload = response.json()
    assert payload["detail"]["status"] == "forbidden"


def test_seed_scenario_endpoint_accepts_allowed_fixture_with_gates(monkeypatch) -> None:
    monkeypatch.setenv("IKAM_PERF_REPORT_TEST_MODE", "1")
    monkeypatch.setenv("IKAM_ALLOW_DEBUG_INJECTION", "1")

    client = TestClient(app)
    response = client.post(
        "/benchmarks/test/seed-scenario",
        json={
            "scenario_key": "core_stream_baseline",
            "overrides": {"run_id": "seed-run-1"},
        },
    )
    assert response.status_code == 200
    payload = response.json()
    assert payload["status"] == "ok"
    assert payload["scenario_key"] == "core_stream_baseline"
    assert payload["run"]["run_id"] == "seed-run-1"


def test_inject_verify_fail_control_requires_debug_gates(monkeypatch) -> None:
    monkeypatch.delenv("IKAM_PERF_REPORT_TEST_MODE", raising=False)
    monkeypatch.delenv("IKAM_ALLOW_DEBUG_INJECTION", raising=False)

    client = TestClient(app)
    response = client.post(
        "/benchmarks/runs/seed-run-1/control",
        json={
            "command_id": "cmd-inject-1",
            "action": "inject_verify_fail",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-seed-1",
        },
    )
    assert response.status_code == 403
    payload = response.json()
    assert payload["detail"]["status"] == "forbidden"


def test_seed_scenario_initializes_immediate_debug_stream_event(monkeypatch) -> None:
    monkeypatch.setenv("IKAM_PERF_REPORT_TEST_MODE", "1")
    monkeypatch.setenv("IKAM_ALLOW_DEBUG_INJECTION", "1")

    client = TestClient(app)
    response = client.post(
        "/benchmarks/test/seed-scenario",
        json={
            "scenario_key": "core_stream_baseline",
            "overrides": {
                "run_id": "seed-run-stream-1",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": "pipe-seed-run-stream-1",
            },
        },
    )
    assert response.status_code == 200

    stream = client.get(
        "/benchmarks/runs/seed-run-stream-1/debug-stream",
        params={
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-seed-run-stream-1",
        },
    )
    assert stream.status_code == 200
    from ikam_perf_report.api.benchmarks import _STEP_TRANSITION_ID
    payload = stream.json()
    assert payload["status"] == "ok"
    assert isinstance(payload["events"], list)
    assert len(payload["events"]) >= 1
    first = payload["events"][0]
    expected_step = _STEP_TRANSITION_ID.get("prepare_case", "prepare_case")
    assert first["step_name"] == expected_step
    assert first["attempt_index"] == 1
