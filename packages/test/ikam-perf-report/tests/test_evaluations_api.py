"""Tests for /evaluations/run endpoint.

Uses real OpenAI calls — OPENAI_API_KEY loaded by conftest.py.
"""
from __future__ import annotations

from fastapi.testclient import TestClient

from ikam_perf_report.main import app


def test_post_evaluations_run_returns_structured_report():
    client = TestClient(app)
    response = client.post("/evaluations/run", params={"case_id": "s-local-retail-v01"})
    assert response.status_code == 200
    payload = response.json()
    assert "report" in payload
    assert "rendered" in payload
    assert "details" in payload
    assert isinstance(payload["rendered"], str)
    assert len(payload["rendered"]) > 0
    # Report must contain all quality dimensions
    report = payload["report"]
    assert "compression" in report
    assert "entities" in report
    assert "predicates" in report
    assert "exploration" in report
    assert "query" in report

    details = payload["details"]
    assert "pipeline_steps" in details
    assert "entities" in details
    assert "predicates" in details
    assert "exploration_queries" in details
    assert "query_results" in details


def test_post_evaluations_run_unknown_case_returns_404():
    client = TestClient(app)
    response = client.post("/evaluations/run", params={"case_id": "nonexistent-case"})
    assert response.status_code == 404
