"""Stochastic quality evaluation against idea.md oracle. Contract: section 8.2, P-S-*.

idea.md is the authoritative oracle for business identity questions.
It is NEVER ingested — only used for evaluation.
The evaluation computes coverage and grounded precision metrics
by comparing graph search results against oracle content.
"""

from __future__ import annotations

from fastapi.testclient import TestClient
from unittest.mock import AsyncMock, MagicMock, patch
from uuid import uuid4

from ikam_perf_report.benchmarks import runner
from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot
from ikam_perf_report.benchmarks.quality_signals import evaluate_commit_lane_gates
from ikam_perf_report.main import app


def _tokenize(text: str) -> set[str]:
    """Extract lowercase alpha-numeric tokens for overlap measurement."""
    cleaned = "".join(ch if ch.isalnum() else " " for ch in text.lower())
    return {t for t in cleaned.split() if len(t) > 2}


def _evidence_coverage(result_evidence: list[str], oracle_tokens: set[str]) -> float:
    """Fraction of oracle tokens covered by search evidence."""
    if not oracle_tokens:
        return 0.0
    evidence_tokens: set[str] = set()
    for ev in result_evidence:
        evidence_tokens |= _tokenize(ev)
    covered = oracle_tokens & evidence_tokens
    return len(covered) / len(oracle_tokens)


def _grounded_precision(result_evidence: list[str], oracle_tokens: set[str]) -> float:
    """Fraction of evidence tokens that appear in oracle (grounded, not hallucinated)."""
    evidence_tokens: set[str] = set()
    for ev in result_evidence:
        evidence_tokens |= _tokenize(ev)
    if not evidence_tokens:
        return 0.0
    grounded = evidence_tokens & oracle_tokens
    return len(grounded) / len(evidence_tokens)


def _run_case_and_wait_for_pipeline(client: TestClient, case_id: str) -> str:
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": case_id, "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run = run_resp.json()["runs"][0]
    run_id = run["run_id"]

    state_resp = client.get(f"/benchmarks/runs/{run_id}/debug-state")
    assert state_resp.status_code == 200
    state = state_resp.json()
    assert state.get("status") == "ok"

    control_resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": str(uuid4()),
            "action": "resume",
            "pipeline_id": state["pipeline_id"],
            "pipeline_run_id": state["pipeline_run_id"],
        },
    )
    assert control_resp.status_code == 200
    control = control_resp.json()
    assert control.get("status") == "ok"
    assert control.get("state", {}).get("execution_state") in {"completed", "paused"}
    return run["graph_id"]


def test_no_ingested_asset_is_idea_md(case_fixtures_root, monkeypatch):
    """Contract: idea.md must never appear in the ingested corpus."""
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [], "relations": []},
    )
    client = TestClient(app)
    graph_id = _run_case_and_wait_for_pipeline(client, "s-construction-v01")

    nodes_resp = client.get("/graph/nodes", params={"graph_id": graph_id})
    assert nodes_resp.status_code == 200
    nodes = nodes_resp.json()

    for node in nodes:
        label = str(node.get("label") or "").lower()
        node_id = str(node.get("id") or "").lower()
        assert "idea.md" not in label, f"idea.md leaked into node label: {node}"
        assert "idea.md" not in node_id, f"idea.md leaked into node id: {node}"


def test_business_identity_answer_quality_against_idea_oracle(
    case_fixtures_root, oracle_text, monkeypatch
):
    """Contract: section 8.2.

    Query the graph with a generic business identity question, then compare
    the returned evidence against the idea.md oracle.  Assert coverage and
    grounded precision exceed minimum thresholds.
    """
    STORE.reset()
    graph_id = "s-construction-v01"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-business-identity",
            project_id=graph_id,
            case_id=graph_id,
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id=graph_id,
                fragments=[
                    {
                        "id": "f1",
                        "label": "business identity",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report",
                            "fragment_id": "f1",
                            "profile": "modelado/reasoning@1",
                            "statement": {"subject": "business identity"},
                            "evidence_refs": [{"fragment_id": "f1"}],
                        },
                    }
                ],
                nodes=[{"id": "f1", "label": "business identity", "type": "proposition"}],
                edges=[],
            ),
        )
    )

    client = TestClient(app)
    anchor_id = "f1"
    citation_id = "f1"

    mock_ai_response = MagicMock()
    mock_ai_response.text = (
        '{"interpretation": "Revenue assumptions and plan ['
        + citation_id
        + '].", '
        '"attribution": [{"claim": "Revenue assumptions", "fragment_ids": ["'
        + citation_id
        + '"]}]}'
    )
    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)

    with patch("ikam_perf_report.api.graph.create_ai_client_from_env", return_value=mock_ai_client):
        resp = client.post(
            "/graph/search",
            json={"query": "what kind of business is this?", "graph_id": graph_id, "anchor_ids": [anchor_id]},
        )
    assert resp.status_code == 200, resp.text
    body = resp.json()
    assert body["results"], "Search must return results"

    # Collect evidence from SQI response + graph node metadata.
    all_evidence: list[str] = []
    interpretation = body.get("interpretation")
    if isinstance(interpretation, str) and interpretation:
        all_evidence.append(interpretation)
    for item in body.get("attribution", []):
        if isinstance(item, dict):
            for key in ("snippet", "text", "source"):
                value = item.get(key)
                if isinstance(value, str) and value:
                    all_evidence.append(value)

    # Include artifact labels and filenames from nodes.
    nodes_resp = client.get("/graph/nodes", params={"graph_id": graph_id})
    for node in nodes_resp.json():
        for value in (
            node.get("label"),
            node.get("file_name"),
            (node.get("meta") or {}).get("file_name"),
        ):
            if isinstance(value, str) and value:
                normalized = value.rsplit(".", 1)[0].replace("_", " ").replace("-", " ")
                all_evidence.append(normalized)

    oracle_tokens = _tokenize(oracle_text)

    coverage = _evidence_coverage(all_evidence, oracle_tokens)
    precision = _grounded_precision(all_evidence, oracle_tokens)

    # Thresholds are intentionally permissive for small test fixtures.
    # Real corpora with richer idea.md content should hit higher marks.
    assert coverage >= 0.10, (
        f"Evidence coverage too low: {coverage:.2f} < 0.10. "
        f"Oracle tokens: {oracle_tokens}, evidence sample: {all_evidence[:5]}"
    )
    assert precision >= 0.08, (
        f"Grounded precision too low: {precision:.2f} < 0.08. "
        f"Oracle tokens: {oracle_tokens}, evidence sample: {all_evidence[:5]}"
    )


def test_graph_nodes_expose_canonical_semantic_group_fields(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {
            "entities": [{"id": "ent-revenue", "label": "Revenue", "kind": "metric"}],
            "relations": [{"id": "rel-evaluates", "kind": "evaluates"}],
        },
    )

    client = TestClient(app)
    graph_id = _run_case_and_wait_for_pipeline(client, "s-construction-v01")

    nodes_resp = client.get("/graph/nodes", params={"graph_id": graph_id})
    assert nodes_resp.status_code == 200
    nodes = nodes_resp.json()
    assert nodes

    meta = nodes[0].get("meta", {})
    assert "semantic_entity_ids" in meta
    assert "semantic_entity_id" in meta
    assert "semantic_entity_label" in meta


def test_commit_lane_gate_contract_rejects_missing_provenance_signal() -> None:
    gate = evaluate_commit_lane_gates(
        {
            "evidence_grounding_ratio": 0.2,
            "evidence_coverage_ratio": 0.2,
            "endpoint_integrity": 0.5,
            "edge_idempotency_integrity": 1.0,
        }
    )
    assert gate["passed"] is False
    assert "endpoint_integrity_floor" in gate["failures"]
