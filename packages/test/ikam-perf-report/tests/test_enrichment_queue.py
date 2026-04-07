from __future__ import annotations

from uuid import uuid4

from fastapi.testclient import TestClient

from ikam_perf_report.benchmarks import runner
from ikam_perf_report.benchmarks.store import EnrichmentItem, EnrichmentRun, STORE
from ikam_perf_report.main import app


def _run_case_and_await_async_pipeline(case_id: str, *, reset: bool = True) -> tuple[str, str]:
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={
            "case_ids": case_id,
            "include_evaluation": "false",
            "reset": str(reset).lower(),
        },
    )
    assert run_resp.status_code == 200
    run = run_resp.json()["runs"][0]
    run_id = run["run_id"]
    graph_id = run["graph_id"]

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
    return run_id, graph_id


def _stage_single_enrichment(*, run_id: str, graph_id: str) -> str:
    enrichment_id = STORE.next_enrichment_id(graph_id)
    STORE.add_enrichment_run(
        EnrichmentRun(
            enrichment_id=enrichment_id,
            run_id=run_id,
            graph_id=graph_id,
            sequence=1,
            lane_mode="explore-graph",
            status="staged",
            relation_count=1,
            unresolved_count=1,
        ),
        [
            EnrichmentItem(
                enrichment_id=enrichment_id,
                run_id=run_id,
                graph_id=graph_id,
                relation_id="rel-1",
                relation_kind="semantic_link",
                source="missing-a",
                target="missing-b",
                evidence=["demo"],
                status="staged",
                sequence=1,
                unresolved=True,
            )
        ],
    )
    return enrichment_id


def test_ingestion_creates_enrichment_run_and_items(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {
            "entities": [{"id": "ent-1", "label": "Acme", "kind": "intent"}],
            "relations": [{"id": "rel-1", "kind": "semantic_link", "source": "missing-a", "target": "missing-b", "evidence": ["demo"]}],
        },
    )
    run_id, graph_id = _run_case_and_await_async_pipeline("s-construction-v01", reset=True)
    _stage_single_enrichment(run_id=run_id, graph_id=graph_id)

    runs = STORE.list_enrichment_runs(graph_id)
    items = STORE.list_enrichment_items(graph_id)
    assert len(runs) == 1
    assert runs[0]["status"] == "staged"
    assert len(items) == 1
    assert items[0]["status"] == "staged"


def test_approve_then_commit_promotes_overlay_into_graph(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {
            "entities": [{"id": "ent-1", "label": "Acme", "kind": "intent"}],
            "relations": [{"id": "rel-2", "kind": "semantic_link", "source": "missing-a", "target": "missing-b", "evidence": ["proof"]}],
        },
    )
    run_id, graph_id = _run_case_and_await_async_pipeline("s-construction-v01", reset=True)
    enrichment_id = _stage_single_enrichment(run_id=run_id, graph_id=graph_id)

    approve = STORE.approve_enrichment(graph_id=graph_id, enrichment_id=enrichment_id)
    assert approve["queued"] == 1
    assert len(STORE.list_stage_queue(graph_id)) == 1

    commit = STORE.commit_stage_queue(graph_id)
    assert commit["committed"] == 1
    graph = STORE.get_graph(graph_id)
    assert graph is not None
    assert any((edge.get("meta") or {}).get("origin") == "enrichment" for edge in graph.edges)
    assert any(fragment.get("status") == "committed" for fragment in graph.relational_fragments)
