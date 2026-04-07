from __future__ import annotations

import asyncio
import pytest
import time
from threading import Event as ThreadEvent, Thread
from fastapi.testclient import TestClient
from unittest.mock import patch

from ikam_perf_report.api import benchmarks as benchmarks_api
from ikam_perf_report.benchmarks.debug_models import DebugRunState, DebugStepEvent
from ikam_perf_report.benchmarks.store import BenchmarkRunRecord, GraphSnapshot, STORE
from ikam.fragments import Fragment
from ikam_perf_report.main import app
from modelado.core.execution_scope import DefaultExecutionScope
from modelado.environment_scope import EnvironmentScope
from modelado.oraculo.persistent_graph_state import PersistentGraphState


def _get_verify_gate_step() -> str:
    dynamic_steps = DefaultExecutionScope().get_dynamic_execution_steps()
    for s in dynamic_steps:
        if "verify" in s or "gate" in s:
            return s
    return dynamic_steps[-2] if len(dynamic_steps) >= 2 else "verify_gate"

def _get_compose_step() -> str:
    dynamic_steps = DefaultExecutionScope().get_dynamic_execution_steps()
    for s in dynamic_steps:
        if "compose" in s:
            return s
    return dynamic_steps[-3] if len(dynamic_steps) >= 3 else "compose"






def event_with_step(events: list[dict], step_name: str) -> dict:
    
    return [event for event in events if event["step_name"] == step_name][0]


EXPECTED_DEBUG_STREAM_MISSING_KEYS = {
    "run_id",
    "pipeline_id",
    "pipeline_run_id",
    "status",
    "pipeline_steps",
    "events",
}

EXPECTED_DEBUG_STREAM_OK_KEYS = {
    "run_id",
    "pipeline_id",
    "pipeline_run_id",
    "status",
    "execution_mode",
    "execution_state",
    "control_availability",
    "pipeline_steps",
    "events",
}

EXPECTED_DEBUG_EVENT_KEYS = {
    "event_id",
    "run_id",
    "pipeline_id",
    "pipeline_run_id",
    "project_id",
    "operation_id",
    "ref",
    "step_name",
    "step_id",
    "status",
    "attempt_index",
    "retry_parent_step_id",
    "started_at",
    "ended_at",
    "duration_ms",
    "executor_id",
    "executor_kind",
    "metrics",
    "error",
}

EXPECTED_DEBUG_STATE_OK_KEYS = {
    "run_id",
    "status",
    "pipeline_id",
    "pipeline_run_id",
    "ref",
    "execution_state",
    "execution_mode",
    "current_step_name",
    "pipeline_steps",
    "current_attempt_index",
    "control_availability",
}

EXPECTED_DEBUG_STATE_MISSING_KEYS = {"run_id", "status"}

EXPECTED_STEP_DETAIL_BASE_KEYS = {
    "schema_version",
    "pipeline_id",
    "pipeline_run_id",
    "run_id",
    "step_id",
    "step_name",
    "attempt_index",
    "outcome",
    "why",
    "inputs",
    "outputs",
    "checks",
    "operation_ref",
    "operation_params",
    "produced_fragment_ids",
    "lineage",
}

EXPECTED_STEP_TRACE_KEYS = {
    "workflow_id",
    "request_id",
    "executor_id",
    "executor_kind",
    "transition_id",
    "marking_before_ref",
    "marking_after_ref",
    "enabled_transition_ids",
    "topic_sequence",
    "timeline",
    "raw_events",
    "trace_id",
    "trace_fragment_id",
}


@pytest.fixture(autouse=True)
def _reset_store() -> None:
    STORE.reset()
    yield
    STORE.reset()


def test_debug_stream_requires_pipeline_identity() -> None:
    client = TestClient(app)
    resp = client.get("/benchmarks/runs/run-missing/debug-stream")
    assert resp.status_code == 422


def test_debug_stream_missing_run_contract() -> None:
    client = TestClient(app)
    resp = client.get(
        "/benchmarks/runs/run-missing/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": "pipe-1"},
    )
    assert resp.status_code == 200
    body = resp.json()
    assert body["run_id"] == "run-missing"
    assert body["status"] == "missing"
    assert set(body.keys()) == EXPECTED_DEBUG_STREAM_MISSING_KEYS


def test_debug_stream_exposes_transition_ids() -> None:
    expected_ref = "refs/heads/run/dev-transition"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-transition",
            project_id="proj-transition",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-transition", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-transition",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-transition",
            project_id="proj-transition",
            operation_id="op-transition",
            env_type="dev",
            env_id="dev-transition",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-transition",
            run_id="run-transition",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-transition",
            project_id="proj-transition",
            operation_id="op-transition",
            env_type="dev",
            env_id="dev-transition",
            step_name="init.initialize",
            step_id="step-transition",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
        )
    )

    client = TestClient(app)
    resp = client.get("/benchmarks/runs/run-transition/debug-state")
    assert resp.status_code == 200
    body = resp.json()
    assert body["status"] == "ok"
    assert set(body.keys()) == EXPECTED_DEBUG_STATE_OK_KEYS
    assert body["ref"] == expected_ref
    assert body["current_step_name"].startswith("init.")

    stream = client.get(
        "/benchmarks/runs/run-transition/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": "pipe-transition"},
    )
    assert stream.status_code == 200
    stream_body = stream.json()
    assert set(stream_body.keys()) == EXPECTED_DEBUG_STREAM_OK_KEYS
    assert stream_body["events"][0]["step_name"].startswith("init.")
    assert set(stream_body["events"][0].keys()) == EXPECTED_DEBUG_EVENT_KEYS
    assert stream_body["events"][0]["ref"] == expected_ref
    assert "env_type" not in stream_body["events"][0]
    assert "env_id" not in stream_body["events"][0]


def test_debug_state_expands_minimal_dynamic_steps_to_canonical_pipeline(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(
        DefaultExecutionScope,
        "get_dynamic_execution_steps",
        lambda self: ["init.initialize", "parse_artifacts", "lift_fragments"],
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-canonical-steps",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-canonical-steps",
            project_id="proj-canonical-steps",
            operation_id="op-canonical-steps",
            env_type="dev",
            env_id="dev-canonical-steps",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )

    client = TestClient(app)
    response = client.get("/benchmarks/runs/run-canonical-steps/debug-state")

    assert response.status_code == 200
    body = response.json()
    assert body["pipeline_steps"] == [
        "init.initialize",
        "map.conceptual.lift.surface_fragments",
        "map.conceptual.lift.entities_and_relationships",
        "map.conceptual.lift.claims",
        "map.conceptual.lift.summarize",
        "map.conceptual.embed.discovery_index",
        "map.conceptual.normalize.discovery",
        "map.reconstructable.embed",
        "map.reconstructable.search.dependency_resolution",
        "map.reconstructable.normalize",
        "map.reconstructable.compose.reconstruction_programs",
        "map.conceptual.verify.discovery_gate",
        "map.conceptual.commit.semantic_only",
        "map.reconstructable.build_subgraph.reconstruction",
    ]


def test_debug_endpoints_use_selected_runnable_steps(monkeypatch: pytest.MonkeyPatch) -> None:
    selected_steps = [
        "init.initialize",
        "load.documents",
        "parse.chunk",
        "parse.entities_and_relationships",
        "parse.claims",
        "complete",
    ]

    monkeypatch.setattr(
        DefaultExecutionScope,
        "get_dynamic_execution_steps",
        lambda self: [
            "init.initialize",
            "map.conceptual.lift.surface_fragments",
            "map.conceptual.embed.discovery_index",
        ],
    )

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-selected-runnable",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-selected-runnable",
            project_id="proj-selected-runnable",
            operation_id="op-selected-runnable",
            env_type="dev",
            env_id="dev-selected-runnable",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.chunk",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-selected-runnable",
            run_id="run-selected-runnable",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-selected-runnable",
            project_id="proj-selected-runnable",
            operation_id="op-selected-runnable",
            env_type="dev",
            env_id="dev-selected-runnable",
            step_name="parse.chunk",
            step_id="step-selected-runnable",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-21T00:00:00Z",
            ended_at="2026-03-21T00:00:01Z",
            duration_ms=1,
        )
    )

    client = TestClient(app)

    state_resp = client.get("/benchmarks/runs/run-selected-runnable/debug-state")
    assert state_resp.status_code == 200
    assert state_resp.json()["pipeline_steps"] == selected_steps

    stream_resp = client.get(
        "/benchmarks/runs/run-selected-runnable/debug-stream",
        params={"pipeline_id": "ingestion-early-parse", "pipeline_run_id": "pipe-selected-runnable"},
    )
    assert stream_resp.status_code == 200
    body = stream_resp.json()
    assert body["pipeline_steps"] == selected_steps
    assert body["events"][0]["step_name"] == "parse.chunk"


def test_debug_scope_exposes_ingestion_step_execution_metadata() -> None:
    scope = benchmarks_api._PerfReportDebugExecutionScope(pipeline_id="ingestion-early-parse")

    load_metadata = scope.get_step_execution_metadata("parse_artifacts")
    metadata = scope.get_step_execution_metadata("parse.chunk")

    assert load_metadata["implementation_step_name"] == "parse_artifacts"
    assert load_metadata["capability"] == "python.load_documents"
    assert load_metadata["operator_id"] == "modelado/operators/load_documents"
    assert load_metadata["executor_id"] == "executor://python-primary"
    assert load_metadata["executor_kind"] == "python-executor"
    assert metadata["implementation_step_name"] == "map.conceptual.lift.surface_fragments"
    assert metadata["capability"] == "python.chunk_documents"
    assert metadata["operator_id"] == "modelado/operators/chunking"
    assert metadata["policy"] == {"cost_tier": "standard"}
    assert metadata["executor_id"] == "executor://python-primary"
    assert metadata["executor_kind"] == "python-executor"


def test_control_next_step_executes_selected_runnable_aliases() -> None:
    run_id = "run-selected-runnable-next-step"
    pipeline_run_id = "pipe-selected-runnable-next-step"

    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id="ingestion-early-parse",
            pipeline_run_id=pipeline_run_id,
            project_id="proj-selected-runnable-next-step",
            operation_id="op-selected-runnable-next-step",
            env_type="dev",
            env_id="dev-selected-runnable-next-step",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        run_id,
        {
            "source_bytes": b"# selected runnable\ntext",
            "mime_type": "text/markdown",
            "artifact_id": f"proj-selected-runnable-next-step:{run_id}",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
        },
    )

    client = TestClient(app)
    next_resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": "cmd-selected-runnable-next-step",
            "action": "next_step",
            "pipeline_id": "ingestion-early-parse",
            "pipeline_run_id": pipeline_run_id,
        },
    )
    assert next_resp.status_code == 200
    body = next_resp.json()
    assert body["state"]["current_step_name"] == "load.documents"
    assert body["state"]["execution_state"] == "paused"

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "ingestion-early-parse", "pipeline_run_id": pipeline_run_id},
    )
    assert stream_resp.status_code == 200
    stream_body = stream_resp.json()
    assert [event["step_name"] for event in stream_body["events"]] == ["load.documents"]
    assert stream_body["events"][0]["status"] == "succeeded"


def test_run_payloads_expose_canonical_ref() -> None:
    expected_ref = "refs/heads/run/dev-run-ref"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-ref-contract",
            project_id="proj-ref-contract",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-ref-contract", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-ref-contract",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-ref-contract",
            project_id="proj-ref-contract",
            operation_id="op-ref-contract",
            env_type="dev",
            env_id="dev-run-ref",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )

    client = TestClient(app)

    get_resp = client.get("/benchmarks/runs/run-ref-contract")
    assert get_resp.status_code == 200
    get_body = get_resp.json()
    assert get_body["ref"] == expected_ref

    list_resp = client.get("/benchmarks/runs")
    assert list_resp.status_code == 200
    listed = list_resp.json()
    run_payload = next(item for item in listed if item["run_id"] == "run-ref-contract")
    assert run_payload["ref"] == expected_ref


def test_inspection_endpoint_returns_hot_inspection_subgraph() -> None:
    run_id = "run-hot-inspection"
    step_id = "step-hot-inspection"
    document_set_ref = STORE.put_hot_subgraph(
        run_id=run_id,
        step_id=step_id,
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "artifact:hot-inspection",
            "subgraph_ref": f"hot://{run_id}/document_set/{step_id}",
            "document_refs": ["frag-doc-hot-1"],
            "documents": [
                {
                    "cas_id": "frag-doc-hot-1",
                    "mime_type": "text/markdown",
                    "value": "# Hot document",
                }
            ],
        },
    )
    STORE.add_run(
        BenchmarkRunRecord(
            run_id=run_id,
            project_id="proj-hot-inspection",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-hot-inspection", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-inspection",
            project_id="proj-hot-inspection",
            operation_id="op-hot-inspection",
            env_type="dev",
            env_id="dev-hot-inspection",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )

    client = TestClient(app)
    response = client.get(
        f"/benchmarks/runs/{run_id}/inspection",
        params={"ref": f"inspect://subgraph/{document_set_ref}", "max_depth": 1},
    )

    assert response.status_code == 200
    body = response.json()
    assert body["schema_version"] == "v1"
    assert body["root_node_id"] == f"subgraph:{document_set_ref}"
    assert body["navigation"] == {"focus": {"node_id": f"subgraph:{document_set_ref}"}}
    root = next(node for node in body["nodes"] if node["id"] == body["root_node_id"])
    assert root["kind"] == "subgraph"
    assert root["payload"]["kind"] == "document_set"
    assert root["refs"]["self"] == {
        "backend": "hot",
        "locator": {
            "subgraph_ref": document_set_ref,
            "head_fragment_id": root["provenance"]["head_fragment_id"],
        },
        "hint": None,
    }
    assert root["provenance"]["source_backend"] == "hot"
    member = next(node for node in body["nodes"] if node["kind"] == "fragment")
    assert member["id"] == "fragment:frag-doc-hot-1"
    assert member["ir_kind"] == "document"
    assert member["payload"] == {
        "cas_id": "frag-doc-hot-1",
        "mime_type": "text/markdown",
        "value": "# Hot document",
    }
    assert body["edges"] == [
        {
            "id": f"edge:contains:subgraph:{document_set_ref}:fragment:frag-doc-hot-1",
            "from": f"subgraph:{document_set_ref}",
            "to": "fragment:frag-doc-hot-1",
            "relation": "contains",
            "label": None,
            "summary": None,
            "payload": {},
            "refs": {},
            "provenance": {"source_backend": "hot"},
            "capabilities": {},
        }
    ]


def test_inspection_endpoint_returns_hot_fragment_inspection_for_document_member() -> None:
    run_id = "run-hot-fragment-inspection"
    step_id = "step-hot-fragment-inspection"
    document_set_ref = STORE.put_hot_subgraph(
        run_id=run_id,
        step_id=step_id,
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "artifact:hot-fragment-inspection",
            "subgraph_ref": f"hot://{run_id}/document_set/{step_id}",
            "document_refs": ["frag-doc-hot-2"],
            "documents": [
                {
                    "cas_id": "frag-doc-hot-2",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {
                        "document_id": "doc-hot-2",
                        "filename": "hot-fragment.md",
                        "text": "# Hot fragment document",
                    },
                }
            ],
        },
    )
    STORE.add_run(
        BenchmarkRunRecord(
            run_id=run_id,
            project_id="proj-hot-fragment-inspection",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-hot-fragment-inspection", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-fragment-inspection",
            project_id="proj-hot-fragment-inspection",
            operation_id="op-hot-fragment-inspection",
            env_type="dev",
            env_id="dev-hot-fragment-inspection",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )

    client = TestClient(app)
    response = client.get(
        f"/benchmarks/runs/{run_id}/inspection",
        params={"ref": "inspect://fragment/frag-doc-hot-2", "max_depth": 1},
    )

    assert response.status_code == 200
    body = response.json()
    assert body["root_node_id"] == "fragment:frag-doc-hot-2"
    root = next(node for node in body["nodes"] if node["id"] == body["root_node_id"])
    assert root["kind"] == "fragment"
    assert root["label"] == "hot-fragment.md"
    assert root["payload"] == {
        "cas_id": "frag-doc-hot-2",
        "mime_type": "application/vnd.ikam.loaded-document+json",
        "value": {
            "document_id": "doc-hot-2",
            "filename": "hot-fragment.md",
            "text": "# Hot fragment document",
        },
    }


def test_inspection_endpoint_returns_hot_fragment_inspection_for_chunk_member_with_source_document() -> None:
    run_id = "run-hot-chunk-fragment-inspection"
    step_id = "step-hot-chunk-fragment-inspection"
    STORE.put_hot_subgraph(
        run_id=run_id,
        step_id=step_id,
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "artifact:hot-chunk-fragment-inspection",
            "subgraph_ref": f"hot://{run_id}/document_set/{step_id}",
            "document_refs": ["frag-doc-hot-3"],
            "documents": [
                {
                    "cas_id": "frag-doc-hot-3",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {
                        "document_id": "doc-hot-3",
                        "artifact_id": "artifact:hot-chunk-fragment-inspection",
                        "filename": "hot-source.md",
                        "text": "# Hot source document",
                    },
                }
            ],
        },
    )
    STORE.put_hot_subgraph(
        run_id=run_id,
        step_id=step_id,
        contract_type="chunk_extraction_set",
        payload={
            "kind": "chunk_extraction_set",
            "source_subgraph_ref": f"hot://{run_id}/document_set/{step_id}",
            "subgraph_ref": f"hot://{run_id}/chunk_extraction_set/{step_id}",
            "extraction_refs": ["frag-chunk-hot-3"],
            "documents": [
                {
                    "cas_id": "frag-doc-hot-3",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {
                        "document_id": "doc-hot-3",
                        "artifact_id": "artifact:hot-chunk-fragment-inspection",
                        "filename": "hot-source.md",
                        "text": "# Hot source document",
                    },
                }
            ],
            "extractions": [
                {
                    "cas_id": "frag-chunk-hot-3",
                    "mime_type": "application/vnd.ikam.chunk-extraction+json",
                    "value": {
                        "chunk_id": "doc-hot-3:chunk:0",
                        "document_id": "doc-hot-3",
                        "source_document_fragment_id": "frag-doc-hot-3",
                        "artifact_id": "artifact:hot-chunk-fragment-inspection",
                        "filename": "hot-source.md",
                        "text": "Hot source chunk",
                        "span": {"start": 0, "end": 16},
                        "order": 0,
                    },
                }
            ],
            "edges": [
                {
                    "from": "fragment:frag-chunk-hot-3",
                    "to": "fragment:frag-doc-hot-3",
                    "edge_label": "knowledge:derives",
                }
            ],
        },
    )
    STORE.add_run(
        BenchmarkRunRecord(
            run_id=run_id,
            project_id="proj-hot-chunk-fragment-inspection",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-hot-chunk-fragment-inspection", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-chunk-fragment-inspection",
            project_id="proj-hot-chunk-fragment-inspection",
            operation_id="op-hot-chunk-fragment-inspection",
            env_type="dev",
            env_id="dev-hot-chunk-fragment-inspection",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.chunk",
            current_attempt_index=1,
        )
    )

    client = TestClient(app)
    response = client.get(
        f"/benchmarks/runs/{run_id}/inspection",
        params={"ref": "inspect://fragment/frag-chunk-hot-3", "max_depth": 1},
    )

    assert response.status_code == 200
    body = response.json()
    assert body["root_node_id"] == "fragment:frag-chunk-hot-3"
    node_ids = {node["id"] for node in body["nodes"]}
    assert "fragment:frag-doc-hot-3" in node_ids
    edges = {(edge["from"], edge["to"], edge["relation"]) for edge in body["edges"]}
    assert ("fragment:frag-chunk-hot-3", "fragment:frag-doc-hot-3", "derives") in edges


def test_inspection_endpoint_augments_environment_chunk_fragment_with_source_document_connection() -> None:
    run_id = "run-env-chunk-fragment-inspection"
    step_id = "step-env-chunk-fragment-inspection"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id=run_id,
            project_id="proj-env-chunk-fragment-inspection",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="proj-env-chunk-fragment-inspection",
                fragments=[
                    {
                        "id": "frag-doc-env-1",
                        "cas_id": "frag-doc-env-1",
                        "mime_type": "application/vnd.ikam.loaded-document+json",
                        "value": {
                            "document_id": "doc-env-1",
                            "artifact_id": "artifact:env-chunk-fragment-inspection",
                            "filename": "env-source.md",
                            "text": "source text",
                        },
                        "meta": {"env_type": "dev", "env_id": "dev-env-chunk-fragment-inspection"},
                    },
                    {
                        "id": "frag-chunk-env-1",
                        "cas_id": "frag-chunk-env-1",
                        "mime_type": "application/vnd.ikam.chunk-extraction+json",
                        "value": {
                            "chunk_id": "doc-env-1:chunk:0",
                            "document_id": "doc-env-1",
                            "artifact_id": "artifact:env-chunk-fragment-inspection",
                            "filename": "env-source.md",
                            "text": "source chunk",
                            "span": {"start": 0, "end": 12},
                            "order": 0,
                        },
                        "meta": {"env_type": "dev", "env_id": "dev-env-chunk-fragment-inspection"},
                    },
                ],
            ),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-env-chunk-fragment-inspection",
            project_id="proj-env-chunk-fragment-inspection",
            operation_id="op-env-chunk-fragment-inspection",
            env_type="dev",
            env_id="dev-env-chunk-fragment-inspection",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.chunk",
            current_attempt_index=1,
        )
    )
    STORE.put_hot_subgraph(
        run_id=run_id,
        step_id=step_id,
        contract_type="chunk_extraction_set",
        payload={
            "kind": "chunk_extraction_set",
            "source_subgraph_ref": f"hot://{run_id}/document_set/{step_id}",
            "subgraph_ref": f"hot://{run_id}/chunk_extraction_set/{step_id}",
            "extraction_refs": ["frag-chunk-env-1"],
            "documents": [
                {
                    "cas_id": "frag-doc-env-1",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {
                        "document_id": "doc-env-1",
                        "artifact_id": "artifact:env-chunk-fragment-inspection",
                        "filename": "env-source.md",
                        "text": "source text",
                    },
                }
            ],
            "extractions": [
                {
                    "cas_id": "frag-chunk-env-1",
                    "mime_type": "application/vnd.ikam.chunk-extraction+json",
                    "value": {
                        "chunk_id": "doc-env-1:chunk:0",
                        "document_id": "doc-env-1",
                        "source_document_fragment_id": "frag-doc-env-1",
                        "artifact_id": "artifact:env-chunk-fragment-inspection",
                        "filename": "env-source.md",
                        "text": "source chunk",
                        "span": {"start": 0, "end": 12},
                        "order": 0,
                    },
                }
            ],
            "edges": [
                {
                    "from": "fragment:frag-chunk-env-1",
                    "to": "fragment:frag-doc-env-1",
                    "edge_label": "knowledge:derives",
                }
            ],
        },
    )

    client = TestClient(app)
    response = client.get(
        f"/benchmarks/runs/{run_id}/inspection",
        params={"ref": "inspect://fragment/frag-chunk-env-1", "max_depth": 1},
    )

    assert response.status_code == 200
    body = response.json()
    node_ids = {node["id"] for node in body["nodes"]}
    assert "fragment:frag-doc-env-1" in node_ids
    edges = {(edge["from"], edge["to"], edge["relation"]) for edge in body["edges"]}
    assert ("fragment:frag-chunk-env-1", "fragment:frag-doc-env-1", "derives") in edges


def test_inspection_endpoint_keeps_chunk_container_context_for_chunk_and_document() -> None:
    run_id = "run-hot-container-context-inspection"
    step_id = "step-hot-container-context-inspection"
    STORE.put_hot_subgraph(
        run_id=run_id,
        step_id=step_id,
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "artifact:hot-container-context-inspection",
            "subgraph_ref": f"hot://{run_id}/document_set/{step_id}",
            "document_refs": ["frag-doc-hot-ctx-1"],
            "documents": [
                {
                    "cas_id": "frag-doc-hot-ctx-1",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {
                        "document_id": "doc-hot-ctx-1",
                        "artifact_id": "artifact:hot-container-context-inspection",
                        "filename": "hot-context.md",
                        "text": "# Hot context document",
                    },
                }
            ],
        },
    )
    STORE.put_hot_subgraph(
        run_id=run_id,
        step_id=step_id,
        contract_type="chunk_extraction_set",
        payload={
            "kind": "chunk_extraction_set",
            "source_subgraph_ref": f"hot://{run_id}/document_set/{step_id}",
            "subgraph_ref": f"hot://{run_id}/chunk_extraction_set/{step_id}",
            "extraction_refs": ["frag-chunk-hot-ctx-1"],
            "documents": [
                {
                    "cas_id": "frag-doc-hot-ctx-1",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {
                        "document_id": "doc-hot-ctx-1",
                        "artifact_id": "artifact:hot-container-context-inspection",
                        "filename": "hot-context.md",
                        "text": "# Hot context document",
                    },
                }
            ],
            "extractions": [
                {
                    "cas_id": "frag-chunk-hot-ctx-1",
                    "mime_type": "application/vnd.ikam.chunk-extraction+json",
                    "value": {
                        "chunk_id": "doc-hot-ctx-1:chunk:0",
                        "document_id": "doc-hot-ctx-1",
                        "source_document_fragment_id": "frag-doc-hot-ctx-1",
                        "artifact_id": "artifact:hot-container-context-inspection",
                        "filename": "hot-context.md",
                        "text": "Hot context chunk",
                        "span": {"start": 0, "end": 17},
                        "order": 0,
                    },
                }
            ],
            "document_chunk_sets": [
                {
                    "cas_id": "frag-doc-chunks-hot-ctx-1",
                    "mime_type": "application/vnd.ikam.document-chunk-set+json",
                    "value": {
                        "kind": "document_chunk_set",
                        "document_id": "doc-hot-ctx-1",
                        "source_document_fragment_id": "frag-doc-hot-ctx-1",
                        "artifact_id": "artifact:hot-container-context-inspection",
                        "filename": "hot-context.md",
                        "chunk_refs": ["frag-chunk-hot-ctx-1"],
                    },
                }
            ],
            "edges": [
                {
                    "from": "fragment:frag-chunk-hot-ctx-1",
                    "to": "fragment:frag-doc-hot-ctx-1",
                    "edge_label": "knowledge:derives",
                }
            ],
        },
    )
    STORE.add_run(
        BenchmarkRunRecord(
            run_id=run_id,
            project_id="proj-hot-container-context-inspection",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-hot-container-context-inspection", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-container-context-inspection",
            project_id="proj-hot-container-context-inspection",
            operation_id="op-hot-container-context-inspection",
            env_type="dev",
            env_id="dev-hot-container-context-inspection",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.chunk",
            current_attempt_index=1,
        )
    )

    client = TestClient(app)

    chunk_response = client.get(
        f"/benchmarks/runs/{run_id}/inspection",
        params={"ref": "inspect://fragment/frag-chunk-hot-ctx-1", "max_depth": 1},
    )
    assert chunk_response.status_code == 200
    chunk_body = chunk_response.json()
    chunk_node_ids = {node["id"] for node in chunk_body["nodes"]}

    assert "fragment:frag-doc-hot-ctx-1" in chunk_node_ids
    assert f"subgraph:hot://{run_id}/chunk_extraction_set/{step_id}" in chunk_node_ids

    document_response = client.get(
        f"/benchmarks/runs/{run_id}/inspection",
        params={"ref": "inspect://fragment/frag-doc-hot-ctx-1", "max_depth": 1},
    )
    assert document_response.status_code == 200
    document_body = document_response.json()
    document_node_ids = {node["id"] for node in document_body["nodes"]}

    assert f"subgraph:hot://{run_id}/document_set/{step_id}" in document_node_ids
    assert "fragment:frag-doc-chunks-hot-ctx-1" in document_node_ids


def test_inspection_endpoint_uses_hot_store_ref_when_container_payload_subgraph_ref_is_empty() -> None:
    run_id = "run-hot-empty-subgraph-ref-context"
    load_step_id = "step-hot-empty-subgraph-ref-load"
    parse_step_id = "step-hot-empty-subgraph-ref-parse"
    document_set_ref = STORE.put_hot_subgraph(
        run_id=run_id,
        step_id=load_step_id,
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "artifact:hot-empty-subgraph-ref-context",
            "subgraph_ref": "",
            "document_refs": ["frag-doc-hot-empty-subgraph-ref-1"],
            "documents": [
                {
                    "cas_id": "frag-doc-hot-empty-subgraph-ref-1",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {
                        "document_id": "doc-hot-empty-subgraph-ref-1",
                        "artifact_id": "artifact:hot-empty-subgraph-ref-context",
                        "filename": "hot-empty.md",
                        "text": "# Hot empty subgraph ref document",
                    },
                }
            ],
        },
    )
    chunk_extraction_set_ref = STORE.put_hot_subgraph(
        run_id=run_id,
        step_id=parse_step_id,
        contract_type="chunk_extraction_set",
        payload={
            "kind": "chunk_extraction_set",
            "source_subgraph_ref": document_set_ref,
            "subgraph_ref": "",
            "extraction_refs": ["frag-chunk-hot-empty-subgraph-ref-1"],
            "documents": [
                {
                    "cas_id": "frag-doc-hot-empty-subgraph-ref-1",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {
                        "document_id": "doc-hot-empty-subgraph-ref-1",
                        "artifact_id": "artifact:hot-empty-subgraph-ref-context",
                        "filename": "hot-empty.md",
                        "text": "# Hot empty subgraph ref document",
                    },
                }
            ],
            "extractions": [
                {
                    "cas_id": "frag-chunk-hot-empty-subgraph-ref-1",
                    "mime_type": "application/vnd.ikam.chunk-extraction+json",
                    "value": {
                        "chunk_id": "doc-hot-empty-subgraph-ref-1:chunk:0",
                        "document_id": "doc-hot-empty-subgraph-ref-1",
                        "source_document_fragment_id": "frag-doc-hot-empty-subgraph-ref-1",
                        "artifact_id": "artifact:hot-empty-subgraph-ref-context",
                        "filename": "hot-empty.md",
                        "text": "Hot empty context chunk",
                        "span": {"start": 0, "end": 23},
                        "order": 0,
                    },
                }
            ],
            "document_chunk_sets": [
                {
                    "cas_id": "frag-doc-chunks-hot-empty-subgraph-ref-1",
                    "mime_type": "application/vnd.ikam.document-chunk-set+json",
                    "value": {
                        "kind": "document_chunk_set",
                        "document_id": "doc-hot-empty-subgraph-ref-1",
                        "source_document_fragment_id": "frag-doc-hot-empty-subgraph-ref-1",
                        "artifact_id": "artifact:hot-empty-subgraph-ref-context",
                        "filename": "hot-empty.md",
                        "chunk_refs": ["frag-chunk-hot-empty-subgraph-ref-1"],
                    },
                }
            ],
        },
    )
    STORE.add_run(
        BenchmarkRunRecord(
            run_id=run_id,
            project_id="proj-hot-empty-subgraph-ref-context",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-hot-empty-subgraph-ref-context", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-empty-subgraph-ref-context",
            project_id="proj-hot-empty-subgraph-ref-context",
            operation_id="op-hot-empty-subgraph-ref-context",
            env_type="dev",
            env_id="dev-hot-empty-subgraph-ref-context",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.chunk",
            current_attempt_index=1,
        )
    )

    client = TestClient(app)
    document_response = client.get(
        f"/benchmarks/runs/{run_id}/inspection",
        params={"ref": "inspect://fragment/frag-doc-hot-empty-subgraph-ref-1", "max_depth": 1},
    )
    assert document_response.status_code == 200
    document_body = document_response.json()
    document_node_ids = {node["id"] for node in document_body["nodes"]}

    assert f"subgraph:{document_set_ref}" in document_node_ids
    assert f"subgraph:{chunk_extraction_set_ref}" in document_node_ids
    assert "fragment:frag-doc-chunks-hot-empty-subgraph-ref-1" in document_node_ids


def test_inspection_endpoint_returns_persistent_inspection_subgraph() -> None:
    run_id = "run-persistent-inspection"
    persistent_state = PersistentGraphState()
    persistent_state.add_fragment(
        Fragment(cas_id="frag-doc-persistent-1", mime_type="application/json", value={"title": "Persistent doc"})
    )
    persistent_state.register_inspection_subgraph(
        {
            "kind": "document_set",
            "subgraph_ref": "refs/heads/main/subgraphs/document-set-persistent",
            "member_refs": ["frag-doc-persistent-1"],
            "edges": [{"to": "frag-doc-persistent-1", "edge_label": "knowledge:contains"}],
        }
    )
    STORE.add_run(
        BenchmarkRunRecord(
            run_id=run_id,
            project_id="proj-persistent-inspection",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-persistent-inspection", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-persistent-inspection",
            project_id="proj-persistent-inspection",
            operation_id="op-persistent-inspection",
            env_type="committed",
            env_id="main",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="map.conceptual.verify.discovery_gate",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(run_id, {"persistent_graph_state": persistent_state})

    client = TestClient(app)
    response = client.get(
        f"/benchmarks/runs/{run_id}/inspection",
        params={"ref": "inspect://subgraph/refs/heads/main/subgraphs/document-set-persistent", "max_depth": 1},
    )

    assert response.status_code == 200
    body = response.json()
    assert body["schema_version"] == "v1"
    assert body["root_node_id"] == "subgraph:refs/heads/main/subgraphs/document-set-persistent"
    root = next(node for node in body["nodes"] if node["id"] == body["root_node_id"])
    assert root["provenance"] == {"source_backend": "persistent"}
    assert root["refs"]["self"] == {
        "backend": "persistent",
        "locator": {"subgraph_ref": "refs/heads/main/subgraphs/document-set-persistent"},
        "hint": None,
    }
    member = next(node for node in body["nodes"] if node["kind"] == "fragment")
    assert member["id"] == "fragment:frag-doc-persistent-1"
    assert member["ir_kind"] == "json"
    assert member["payload"] == {
        "cas_id": "frag-doc-persistent-1",
        "mime_type": "application/json",
        "value": {"title": "Persistent doc"},
    }
    assert body["edges"] == [
        {
            "id": "edge:contains:subgraph:refs/heads/main/subgraphs/document-set-persistent:fragment:frag-doc-persistent-1",
            "from": "subgraph:refs/heads/main/subgraphs/document-set-persistent",
            "to": "fragment:frag-doc-persistent-1",
            "relation": "contains",
            "label": None,
            "summary": None,
            "payload": {},
            "refs": {},
            "provenance": {"source_backend": "persistent"},
            "capabilities": {},
        }
    ]


def test_env_fragments_requires_scope_params() -> None:
    client = TestClient(app)
    resp = client.get(
        "/benchmarks/runs/run-missing/env/fragments",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": "pipe-1"},
    )
    assert resp.status_code == 422


def test_env_fragments_missing_run_contract() -> None:
    ref = "refs/heads/run/dev-1"
    client = TestClient(app)
    resp = client.get(
        "/benchmarks/runs/run-missing/env/fragments",
        params={
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-1",
            "ref": ref,
            "step_id": "step-1",
            "attempt_index": 1,
        },
    )
    assert resp.status_code == 200
    body = resp.json()
    assert body["status"] == "missing"
    assert body["run_id"] == "run-missing"


def test_env_fragments_returns_full_scoped_payloads() -> None:
    ref = "refs/heads/run/dev-1"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="proj-1",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="proj-1",
                fragments=[
                    {
                        "id": "frag-dev-1",
                        "mime_type": "application/ikam-claim-ir+json",
                        "value": {"claim": "a"},
                        "meta": {
                            "env_type": "dev",
                            "env_id": "dev-1",
                            "step_id": "step-1",
                            "attempt_index": 1,
                        },
                    },
                    {
                        "id": "frag-stg-1",
                        "mime_type": "application/ikam-claim-ir+json",
                        "value": {"claim": "b"},
                        "meta": {
                            "env_type": "staging",
                            "env_id": "stg-1",
                            "step_id": "step-1",
                            "attempt_index": 1,
                        },
                    },
                ],
            ),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-1",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-1",
            project_id="proj-1",
            operation_id="op-1",
            env_type="dev",
            env_id="dev-1",
            execution_mode="autonomous",
            execution_state="running",
            current_step_name="map.conceptual.lift.surface_fragments",
            current_attempt_index=1,
        )
    )

    client = TestClient(app)
    resp = client.get(
        "/benchmarks/runs/run-1/env/fragments",
        params={
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-1",
            "ref": ref,
            "step_id": "step-1",
            "attempt_index": 1,
        },
    )
    assert resp.status_code == 200
    body = resp.json()
    assert body["status"] == "ok"
    assert body["scope"]["ref"] == ref
    assert "env_type" not in body["scope"]
    assert "env_id" not in body["scope"]
    assert len(body["fragments"]) == 1
    assert body["fragments"][0]["id"] == "frag-dev-1"
    assert body["fragments"][0]["value"] == {"claim": "a"}


def test_verification_endpoint_returns_scoped_records() -> None:
    ref = "refs/heads/run/dev-2"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-2",
            project_id="proj-2",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="proj-2",
                fragments=[
                    {
                        "id": "verify-1",
                        "mime_type": "application/ikam-verification-result+json",
                        "value": {"passed": True, "metric": "renderer-equality"},
                        "meta": {
                            "env_type": "dev",
                            "env_id": "dev-2",
                            "step_id": "step-v",
                            "attempt_index": 2,
                            "record_type": "verification",
                        },
                    }
                ],
            ),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-2",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-2",
            project_id="proj-2",
            operation_id="op-2",
            env_type="dev",
            env_id="dev-2",
            execution_mode="autonomous",
            execution_state="running",
            current_step_name=_get_verify_gate_step(),
            current_attempt_index=2,
        )
    )

    client = TestClient(app)
    resp = client.get(
        "/benchmarks/runs/run-2/verification",
        params={
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-2",
            "ref": ref,
            "step_id": "step-v",
            "attempt_index": 2,
        },
    )
    assert resp.status_code == 200
    body = resp.json()
    assert body["status"] == "ok"
    assert len(body["verification_records"]) == 1
    assert body["verification_records"][0]["value"]["passed"] is True


def test_reconstruction_endpoint_returns_scoped_records() -> None:
    ref = "refs/heads/staging/stg-3"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-3",
            project_id="proj-3",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="proj-3",
                fragments=[
                    {
                        "id": "recon-1",
                        "mime_type": "application/ikam-reconstruction-program+json",
                        "value": {"ops": [{"op": "compose", "target": "sheet-1"}]},
                        "meta": {
                            "env_type": "staging",
                            "env_id": "stg-3",
                            "step_id": "step-c",
                            "attempt_index": 1,
                            "record_type": "reconstruction_program",
                        },
                    }
                ],
            ),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-3",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-3",
            project_id="proj-3",
            operation_id="op-3",
            env_type="staging",
            env_id="stg-3",
            execution_mode="autonomous",
            execution_state="running",
            current_step_name=_get_compose_step(),
            current_attempt_index=1,
        )
    )

    client = TestClient(app)
    resp = client.get(
        "/benchmarks/runs/run-3/reconstruction-program",
        params={
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-3",
            "ref": ref,
            "step_id": "step-c",
            "attempt_index": 1,
        },
    )
    assert resp.status_code == 200
    body = resp.json()
    assert body["status"] == "ok"
    assert len(body["reconstruction_programs"]) == 1
    assert body["reconstruction_programs"][0]["id"] == "recon-1"


def test_env_summary_endpoint_reports_scoped_counts() -> None:
    ref = "refs/heads/run/dev-4"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-4",
            project_id="proj-4",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="proj-4",
                fragments=[
                    {
                        "id": "claim-1",
                        "mime_type": "application/ikam-claim-ir+json",
                        "value": {"claim": "x"},
                        "meta": {"env_type": "dev", "env_id": "dev-4", "step_id": "s1", "attempt_index": 1},
                    },
                    {
                        "id": "verify-2",
                        "mime_type": "application/ikam-verification-result+json",
                        "value": {"passed": False},
                        "meta": {
                            "env_type": "dev",
                            "env_id": "dev-4",
                            "step_id": "s2",
                            "attempt_index": 1,
                            "record_type": "verification",
                        },
                    },
                ],
            ),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-4",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-4",
            project_id="proj-4",
            operation_id="op-4",
            env_type="dev",
            env_id="dev-4",
            execution_mode="autonomous",
            execution_state="running",
            current_step_name=_get_verify_gate_step(),
            current_attempt_index=1,
        )
    )

    client = TestClient(app)
    resp = client.get(
        "/benchmarks/runs/run-4/env-summary",
        params={
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-4",
            "ref": ref,
        },
    )
    assert resp.status_code == 200
    body = resp.json()
    assert body["status"] == "ok"
    assert body["summary"]["fragment_count"] == 2
    assert body["summary"]["verification_count"] == 1
    assert body["summary"]["ref"] == ref
    assert "env_type" not in body["summary"]
    assert "env_id" not in body["summary"]
    assert body["summary"]["executors_seen"] == []


def test_env_fragments_accepts_env_aliases_for_compatibility() -> None:
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-env-aliases",
            project_id="proj-env-aliases",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="proj-env-aliases",
                fragments=[
                    {
                        "id": "frag-env-aliases",
                        "mime_type": "application/ikam-claim-ir+json",
                        "value": {"claim": "alias"},
                        "meta": {
                            "env_type": "dev",
                            "env_id": "dev-alias",
                            "step_id": "step-alias",
                            "attempt_index": 1,
                        },
                    }
                ],
            ),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-env-aliases",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-env-aliases",
            project_id="proj-env-aliases",
            operation_id="op-env-aliases",
            env_type="dev",
            env_id="dev-alias",
            execution_mode="autonomous",
            execution_state="running",
            current_step_name="map.conceptual.lift.surface_fragments",
            current_attempt_index=1,
        )
    )

    client = TestClient(app)
    resp = client.get(
        "/benchmarks/runs/run-env-aliases/env/fragments",
        params={
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-env-aliases",
            "env_type": "dev",
            "env_id": "dev-alias",
            "step_id": "step-alias",
            "attempt_index": 1,
        },
    )

    assert resp.status_code == 200
    body = resp.json()
    assert body["status"] == "ok"
    assert body["scope"]["ref"] == "refs/heads/run/dev-alias"
    assert "env_type" not in body["scope"]
    assert "env_id" not in body["scope"]
    assert len(body["fragments"]) == 1


def test_debug_stream_serializes_executor_identity() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-executor-stream",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-executor-stream",
            project_id="proj-executor-stream",
            operation_id="op-executor-stream",
            env_type="dev",
            env_id="dev-executor-stream",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="map.conceptual.normalize.discovery",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-executor-stream",
            run_id="run-executor-stream",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-executor-stream",
            project_id="proj-executor-stream",
            operation_id="op-executor-stream",
            env_type="dev",
            env_id="dev-executor-stream",
            step_name="map.conceptual.normalize.discovery",
            step_id="step-executor-stream",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
            metrics={
                "trace": {
                    "executor_id": "executor://python-primary",
                    "executor_kind": "python-executor",
                }
            },
        )
    )

    client = TestClient(app)
    resp = client.get(
        "/benchmarks/runs/run-executor-stream/debug-stream",
        params={
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-executor-stream",
        },
    )

    assert resp.status_code == 200
    body = resp.json()
    assert body["events"][0]["executor_id"] == "executor://python-primary"
    assert body["events"][0]["executor_kind"] == "python-executor"


def test_debug_stream_prefers_compiled_executor_identity_for_ingestion_steps(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        benchmarks_api,
        "_compiled_operator_metadata",
        lambda pipeline_id: {
            "parse.entities_and_relationships": {
                "executor_id": "executor://metadata-ml",
                "executor_kind": "metadata-ml-executor",
            }
        }
        if pipeline_id == "ingestion-early-parse"
        else {},
    )

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-ingestion-executor-stream",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-ingestion-executor-stream",
            project_id="proj-ingestion-executor-stream",
            operation_id="op-ingestion-executor-stream",
            env_type="dev",
            env_id="dev-ingestion-executor-stream",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.entities_and_relationships",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-ingestion-executor-stream",
            run_id="run-ingestion-executor-stream",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-ingestion-executor-stream",
            project_id="proj-ingestion-executor-stream",
            operation_id="op-ingestion-executor-stream",
            env_type="dev",
            env_id="dev-ingestion-executor-stream",
            step_name="parse.entities_and_relationships",
            step_id="step-ingestion-executor-stream",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )

    client = TestClient(app)
    resp = client.get(
        "/benchmarks/runs/run-ingestion-executor-stream/debug-stream",
        params={
            "pipeline_id": "ingestion-early-parse",
            "pipeline_run_id": "pipe-ingestion-executor-stream",
        },
    )

    assert resp.status_code == 200
    body = resp.json()
    assert body["events"][0]["executor_id"] == "executor://metadata-ml"
    assert body["events"][0]["executor_kind"] == "metadata-ml-executor"


def test_store_can_update_debug_event_in_place_by_step_id() -> None:
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-running",
            run_id="run-store-update",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-store-update",
            project_id="proj-store-update",
            operation_id="op-store-update",
            env_type="dev",
            env_id="dev-store-update",
            step_name="map.conceptual.normalize.discovery",
            step_id="step-stable",
            status="running",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-22T00:00:00Z",
            ended_at=None,
            duration_ms=None,
            metrics={"logs": {"stdout_lines": ["line 1"], "stderr_lines": []}},
        )
    )

    STORE.update_debug_event(
        run_id="run-store-update",
        step_id="step-stable",
        status="succeeded",
        ended_at="2026-03-22T00:00:01Z",
        duration_ms=1000,
        metrics={"logs": {"stdout_lines": ["line 1", "line 2"], "stderr_lines": []}},
    )

    events = STORE.list_debug_events("run-store-update")
    assert len(events) == 1
    assert events[0].step_id == "step-stable"
    assert events[0].status == "succeeded"
    assert events[0].metrics["logs"]["stdout_lines"] == ["line 1", "line 2"]


def test_publish_running_event_logs_ignores_stale_step_id() -> None:
    benchmarks_api._publish_running_event_logs(
        run_id="run-missing-step",
        step_id="step-missing-step",
        executor_stdout_lines=[],
        executor_stderr_lines=[],
        system_stdout_lines=["executing parse.chunk operation"],
        system_stderr_lines=[],
        log_events=[
            {
                "seq": 1,
                "at": "2026-03-30T00:00:00Z",
                "source": "system",
                "stream": "stdout",
                "message": "executing parse.chunk operation",
            }
        ],
    )


def test_control_endpoint_updates_mode_and_state() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-5",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-5",
            project_id="proj-5",
            operation_id="op-5",
            env_type="dev",
            env_id="dev-5",
            execution_mode="autonomous",
            execution_state="running",
            current_step_name="map.conceptual.lift.surface_fragments",
            current_attempt_index=1,
        )
    )

    client = TestClient(app)
    mode_resp = client.post(
        "/benchmarks/runs/run-5/control",
        json={
            "command_id": "cmd-mode-1",
            "action": "set_mode",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-5",
            "mode": "manual",
        },
    )
    assert mode_resp.status_code == 200
    assert mode_resp.json()["state"]["execution_mode"] == "manual"

    pause_resp = client.post(
        "/benchmarks/runs/run-5/control",
        json={
            "command_id": "cmd-pause-1",
            "action": "pause",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-5",
        },
    )
    assert pause_resp.status_code == 200
    assert pause_resp.json()["state"]["execution_state"] == "paused"


def test_control_next_step_advances_top_level_step(monkeypatch: pytest.MonkeyPatch) -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-6",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-6",
            project_id="proj-6",
            operation_id="op-6",
            env_type="dev",
            env_id="dev-6",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="map.conceptual.normalize.discovery",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-6",
        {
            "source_bytes": b"# run-6\ntext",
            "mime_type": "text/markdown",
            "artifact_id": "proj-6:run-6",
            "step_outputs": {
                "decomposition": {
                    "root_fragments": [{"cas_id": "f1", "mime_type": "text/markdown"}],
                },
                "lifted": [{"id": "f1", "mime_type": "text/markdown"}],
                "ir_fragments": [],
            },
        },
    )

    monkeypatch.setenv("IKAM_ASYNC_NEXT_STEP", "0")
    client = TestClient(app)
    next_resp = client.post(
        "/benchmarks/runs/run-6/control",
        json={
            "command_id": "cmd-next-1",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-6",
        },
    )
    assert next_resp.status_code == 200
    state_json = next_resp.json()
    assert state_json["state"]["current_step_name"] == "map.reconstructable.embed"

    stream_resp = client.get(
        "/benchmarks/runs/run-6/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": "pipe-6"},
    )
    assert stream_resp.status_code == 200
    body = stream_resp.json()
    assert body["status"] == "ok"
    assert len(body["events"]) == 1
    assert body["events"][0]["metrics"]["executor"] == "ikam.forja.debug_execution"
    assert int(body["events"][0]["duration_ms"]) >= 1


def test_control_next_step_emits_structured_step_details(monkeypatch: pytest.MonkeyPatch) -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-9",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-9",
            project_id="proj-9",
            operation_id="op-9",
            env_type="dev",
            env_id="dev-9",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-9",
        {
            "source_bytes": b"# run-9\ntext",
            "mime_type": "text/markdown",
            "artifact_id": "proj-9:run-9",
            "step_outputs": {},
        },
    )

    monkeypatch.setenv("IKAM_ASYNC_NEXT_STEP", "0")
    client = TestClient(app)
    next_resp = client.post(
        "/benchmarks/runs/run-9/control",
        json={
            "command_id": "cmd-next-9",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-9",
        },
    )
    assert next_resp.status_code == 200

    stream_resp = client.get(
        "/benchmarks/runs/run-9/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": "pipe-9"},
    )
    body = stream_resp.json()
    latest_event = body["events"][-1]
    assert latest_event["step_name"] == "map.conceptual.lift.surface_fragments"
    details = latest_event["metrics"].get("details")
    assert isinstance(details, dict)
    assert isinstance(details.get("root_fragment_ids"), list)


def test_control_next_step_from_verify_creates_retry_boundary_and_increments_attempt(monkeypatch: pytest.MonkeyPatch) -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-7",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-7",
            project_id="proj-7",
            operation_id="op-7",
            env_type="dev",
            env_id="dev-7",
            execution_mode="manual",
            execution_state="paused",
            current_step_name=_get_verify_gate_step(),
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-verify-1",
            run_id="run-7",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-7",
            project_id="proj-7",
            operation_id="op-7",
            env_type="dev",
            env_id="dev-7",
            step_name=_get_verify_gate_step(),
            step_id="step-verify-1",
            status="failed",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-01-01T00:00:00Z",
            ended_at="2026-01-01T00:00:01Z",
            duration_ms=1000,
            metrics={},
            error={"message": "drift"},
        )
    )
    STORE.set_debug_runtime_context(
        "run-7",
        {
            "source_bytes": b"# run-7\ntext",
            "mime_type": "text/markdown",
            "artifact_id": "proj-7:run-7",
            "step_outputs": {
                "decomposition": {
                    "root_fragments": [{"cas_id": "f1", "mime_type": "text/markdown"}],
                },
                "lifted": [{"id": "f1", "mime_type": "text/markdown"}],
                "ir_fragments": [],
                "embeddings": [{"key": "f1", "digest": "abcd"}],
                "candidates": [],
                "normalized": ["text/markdown"],
                "proposal": {"commit_modes": ["normalized", "surface_only"]},
                "verification": {"passed": False},
            },
        },
    )

    monkeypatch.setenv("IKAM_ASYNC_NEXT_STEP", "0")
    client = TestClient(app)
    next_resp = client.post(
        "/benchmarks/runs/run-7/control",
        json={
            "command_id": "cmd-next-retry-1",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-7",
        },
    )
    assert next_resp.status_code == 200
    state = next_resp.json()["state"]
    assert state["current_attempt_index"] == 2
    assert state["current_step_name"] == "map.conceptual.lift.surface_fragments"

    stream_resp = client.get(
        "/benchmarks/runs/run-7/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": "pipe-7"},
    )
    assert stream_resp.status_code == 200
    body = stream_resp.json()
    retry_event = body["events"][-1]
    assert retry_event["step_name"] == "map.conceptual.lift.surface_fragments"
    assert retry_event["attempt_index"] == 2
    assert retry_event["retry_parent_step_id"] == "step-verify-1"


def test_all_10_steps_emit_details_in_metrics() -> None:
    """Contract: every canonical debug step emits a non-empty 'details' dict in metrics."""
    import os
    import psycopg

    db_url = "postgresql://user:pass@localhost:55432/ikam_perf_report"
    try:
        with psycopg.connect(db_url, connect_timeout=3) as conn:
            conn.execute("SELECT 1")
    except Exception:
        pytest.skip("pgvector Postgres not available on port 55432")

    old_env = os.environ.get("DATABASE_URL")
    os.environ["DATABASE_URL"] = db_url
    from modelado.db import reset_pool_for_pytest
    reset_pool_for_pytest()

    try:
        _test_all_10_steps_emit_details_in_metrics_impl()
    finally:
        if old_env is None:
            os.environ.pop("DATABASE_URL", None)
        else:
            os.environ["DATABASE_URL"] = old_env
        reset_pool_for_pytest()


def _test_all_10_steps_emit_details_in_metrics_impl() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-all-steps",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-all-steps",
            project_id="proj-all-steps",
            operation_id="op-all-steps",
            env_type="dev",
            env_id="dev-all-steps",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-all-steps",
        {
            "source_bytes": b"# all-steps test\nSome content for decomposition.",
            "mime_type": "text/markdown",
            "artifact_id": "proj-all-steps:run-all-steps",
            "step_outputs": {},
        },
    )

    client = TestClient(app)

    # Step through all 10 transitions (prepare_case is already current, so
    # next_step advances: prepare_case→decompose, decompose→embed_decomposed, ..., promote_commit→project_graph)
    for i in range(10):
        resp = client.post(
            "/benchmarks/runs/run-all-steps/control",
            json={
                "command_id": f"cmd-all-steps-{i}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": "pipe-all-steps",
            },
        )
        assert resp.status_code == 200, f"Step {i} failed: {resp.text}"

    # Fetch all events
    stream_resp = client.get(
        "/benchmarks/runs/run-all-steps/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": "pipe-all-steps"},
    )
    assert stream_resp.status_code == 200
    body = stream_resp.json()
    events = body["events"]

    # We should have 10 events (one per next_step transition: decompose through project_graph)
    assert len(events) >= 10, f"Expected at least 10 events, got {len(events)}"

    # Every event must have a details dict in metrics
    for event in events:
        step = event["step_name"]
        details = event["metrics"].get("details")
        assert isinstance(details, dict), f"Step '{step}' missing details dict in metrics"
        assert len(details) > 0, f"Step '{step}' has empty details dict"


def test_inject_verify_fail_causes_real_verify_handler_to_fail_with_drift_at() -> None:
    """inject_verify_fail stores drift_at in runtime context; the real verify
    handler checks for it, consumes it (one-shot), and returns a failure event
    with drift_at in the error dict.

    This is a DISCRIMINATING test: if inject_verify_fail creates a synthetic
    event instead of going through the real verify handler, this test FAILS
    because the verify event won't have the real verifier's metrics structure.
    """
    import os
    import psycopg

    db_url = "postgresql://user:pass@localhost:55432/ikam_perf_report"
    try:
        with psycopg.connect(db_url, connect_timeout=3) as conn:
            conn.execute("SELECT 1")
    except Exception:
        pytest.skip("pgvector Postgres not available on port 55432")

    old_db = os.environ.get("DATABASE_URL")
    old_test_mode = os.environ.get("IKAM_PERF_REPORT_TEST_MODE")
    old_debug_inj = os.environ.get("IKAM_ALLOW_DEBUG_INJECTION")
    os.environ["DATABASE_URL"] = db_url
    os.environ["IKAM_PERF_REPORT_TEST_MODE"] = "1"
    os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = "1"
    from modelado.db import reset_pool_for_pytest
    reset_pool_for_pytest()

    try:
        _test_inject_verify_fail_impl()
    finally:
        for key, old_val in [
            ("DATABASE_URL", old_db),
            ("IKAM_PERF_REPORT_TEST_MODE", old_test_mode),
            ("IKAM_ALLOW_DEBUG_INJECTION", old_debug_inj),
        ]:
            if old_val is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = old_val
        reset_pool_for_pytest()


def _test_inject_verify_fail_impl() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-inject",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-inject",
            project_id="proj-inject",
            operation_id="op-inject",
            env_type="dev",
            env_id="dev-inject",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-inject",
        {
            "source_bytes": b"# inject test\nSome content for decomposition.",
            "mime_type": "text/markdown",
            "artifact_id": "proj-inject:run-inject",
            "step_outputs": {},
        },
    )

    client = TestClient(app)

    # Step through prepare_case → decompose → embed_decomposed → lift → embed_lifted → candidate_search → normalize
    # (6 next_step calls to reach normalize, leaving compose_proposal as next)
    for i in range(9):
        resp = client.post(
            "/benchmarks/runs/run-inject/control",
            json={
                "command_id": f"cmd-inject-step-{i}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": "pipe-inject",
            },
        )
        assert resp.status_code == 200, f"Step {i} failed: {resp.text}"

    # Current step should be normalize (6th next_step from prepare_case)
    state = resp.json()["state"]
    assert state["current_step_name"] == "map.reconstructable.normalize"

    # Now inject a verify failure with drift_at="map.conceptual.normalize.discovery"
    inject_resp = client.post(
        "/benchmarks/runs/run-inject/control",
        json={
            "command_id": "cmd-do-inject",
            "action": "inject_verify_fail",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-inject",
            "drift_at": "map.conceptual.normalize.discovery",
        },
    )
    assert inject_resp.status_code == 200
    inject_body = inject_resp.json()
    assert inject_body["status"] == "ok"
    # State should NOT have jumped to verify — injection only stores the flag
    assert inject_body["state"]["current_step_name"] == "map.reconstructable.normalize"

    # Step to compose_proposal (7th next_step)
    cp_resp = client.post(
        "/benchmarks/runs/run-inject/control",
        json={
            "command_id": "cmd-inject-cp",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-inject",
        },
    )
    assert cp_resp.status_code == 200
    assert cp_resp.json()["state"]["current_step_name"] == _get_compose_step()

    # Step to verify (8th next_step)
    verify_resp = client.post(
        "/benchmarks/runs/run-inject/control",
        json={
            "command_id": "cmd-inject-verify",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-inject",
        },
    )
    assert verify_resp.status_code == 200
    verify_state = verify_resp.json()["state"]
    assert verify_state["current_step_name"] == _get_verify_gate_step()

    # Fetch events to check the verify event
    stream_resp = client.get(
        "/benchmarks/runs/run-inject/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": "pipe-inject"},
    )
    events = stream_resp.json()["events"]
    verify_event = [e for e in events if e["step_name"] == _get_verify_gate_step()][-1]

    # DISCRIMINATING: verify must have FAILED because of injection
    assert verify_event["status"] == "failed"
    # Error must contain drift_at from the injection
    assert verify_event["error"]["drift_at"] == "map.conceptual.normalize.discovery"
    # Metrics must show it was an injected failure (from the real verify handler)
    assert verify_event["metrics"]["injection_used"] is True
    assert verify_event["metrics"]["executor"] == "ikam.forja.debug_execution"


def test_retry_loop_reenters_at_drift_point_on_verify_fail() -> None:
    """After verify FAIL with drift_at, next_step re-enters at the diagnosed
    drift step (not always decompose). attempt_index increments,
    retry_parent_step_id is set.

    DISCRIMINATING: current code always re-enters at 'map'.
    This test asserts re-entry at 'lift' (the drift_at value).
    """
    import os
    import psycopg

    db_url = "postgresql://user:pass@localhost:55432/ikam_perf_report"
    try:
        with psycopg.connect(db_url, connect_timeout=3) as conn:
            conn.execute("SELECT 1")
    except Exception:
        pytest.skip("pgvector Postgres not available on port 55432")

    old_db = os.environ.get("DATABASE_URL")
    old_test_mode = os.environ.get("IKAM_PERF_REPORT_TEST_MODE")
    old_debug_inj = os.environ.get("IKAM_ALLOW_DEBUG_INJECTION")
    os.environ["DATABASE_URL"] = db_url
    os.environ["IKAM_PERF_REPORT_TEST_MODE"] = "1"
    os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = "1"
    from modelado.db import reset_pool_for_pytest
    reset_pool_for_pytest()

    try:
        _test_retry_loop_reenters_at_drift_point_impl()
    finally:
        for key, old_val in [
            ("DATABASE_URL", old_db),
            ("IKAM_PERF_REPORT_TEST_MODE", old_test_mode),
            ("IKAM_ALLOW_DEBUG_INJECTION", old_debug_inj),
        ]:
            if old_val is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = old_val
        reset_pool_for_pytest()


def _test_retry_loop_reenters_at_drift_point_impl() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-retry",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-retry",
            project_id="proj-retry",
            operation_id="op-retry",
            env_type="dev",
            env_id="dev-retry",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-retry",
        {
            "source_bytes": b"# retry test\nSome content for retry loop.",
            "mime_type": "text/markdown",
            "artifact_id": "proj-retry:run-retry",
            "step_outputs": {},
        },
    )

    client = TestClient(app)

    # Step through prepare_case → decompose → embed_decomposed → lift → embed_lifted → candidate_search → normalize
    # (6 next_step calls from prepare_case)
    for i in range(9):
        resp = client.post(
            "/benchmarks/runs/run-retry/control",
            json={
                "command_id": f"cmd-retry-step-{i}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": "pipe-retry",
            },
        )
        assert resp.status_code == 200, f"Step {i} failed: {resp.text}"
    assert resp.json()["state"]["current_step_name"] == "map.reconstructable.normalize"

    # Inject verify failure with drift_at="map.conceptual.normalize.discovery"
    inject_resp = client.post(
        "/benchmarks/runs/run-retry/control",
        json={
            "command_id": "cmd-retry-inject",
            "action": "inject_verify_fail",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-retry",
            "drift_at": "map.conceptual.normalize.discovery",
        },
    )
    assert inject_resp.status_code == 200

    # Step to compose_proposal (7th next_step)
    cp_resp = client.post(
        "/benchmarks/runs/run-retry/control",
        json={
            "command_id": "cmd-retry-cp",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-retry",
        },
    )
    assert cp_resp.status_code == 200
    assert cp_resp.json()["state"]["current_step_name"] == _get_compose_step()

    # Step to verify (8th next_step) — this triggers the injected failure
    verify_resp = client.post(
        "/benchmarks/runs/run-retry/control",
        json={
            "command_id": "cmd-retry-verify",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-retry",
        },
    )
    assert verify_resp.status_code == 200
    verify_state = verify_resp.json()["state"]
    assert verify_state["current_step_name"] == _get_verify_gate_step()

    # Confirm verify actually failed
    stream_resp = client.get(
        "/benchmarks/runs/run-retry/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": "pipe-retry"},
    )
    events = stream_resp.json()["events"]
    verify_event = [e for e in events if e["step_name"] == _get_verify_gate_step()][-1]
    assert verify_event["status"] == "failed"
    assert verify_event["error"]["drift_at"] == "map.conceptual.normalize.discovery"

    # CRITICAL: Next next_step call should re-enter at drift point "map.conceptual.normalize.discovery",
    # NOT at "map" as the current code does.
    retry_resp = client.post(
        "/benchmarks/runs/run-retry/control",
        json={
            "command_id": "cmd-retry-reenter",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-retry",
        },
    )
    assert retry_resp.status_code == 200
    retry_body = retry_resp.json()

    # Re-entry step must be "map.conceptual.normalize.discovery" (the drift_at), not "map"
    assert retry_body["state"]["current_step_name"] == "map.conceptual.normalize.discovery"

    # Fetch events to inspect the retry event
    stream2 = client.get(
        "/benchmarks/runs/run-retry/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": "pipe-retry"},
    )
    all_events = stream2.json()["events"]
    retry_event = all_events[-1]  # latest event is the retry re-entry
    assert retry_event["step_name"] == "map.conceptual.normalize.discovery"
    assert retry_event["attempt_index"] == 2  # incremented from 1
    assert retry_event["retry_parent_step_id"] is not None


def test_retry_budget_exhausted_terminates_run() -> None:
    """When retry_budget reaches 0 and verify fails again, the run terminates
    with budget_exhausted status instead of retrying.

    Uses retry_budget_remaining=1 so one verify-fail exhausts it.
    """
    import os
    import psycopg

    db_url = "postgresql://user:pass@localhost:55432/ikam_perf_report"
    try:
        with psycopg.connect(db_url, connect_timeout=3) as conn:
            conn.execute("SELECT 1")
    except Exception:
        pytest.skip("pgvector Postgres not available on port 55432")

    old_db = os.environ.get("DATABASE_URL")
    old_test_mode = os.environ.get("IKAM_PERF_REPORT_TEST_MODE")
    old_debug_inj = os.environ.get("IKAM_ALLOW_DEBUG_INJECTION")
    os.environ["DATABASE_URL"] = db_url
    os.environ["IKAM_PERF_REPORT_TEST_MODE"] = "1"
    os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = "1"
    from modelado.db import reset_pool_for_pytest
    reset_pool_for_pytest()

    try:
        _test_retry_budget_exhausted_impl()
    finally:
        for key, old_val in [
            ("DATABASE_URL", old_db),
            ("IKAM_PERF_REPORT_TEST_MODE", old_test_mode),
            ("IKAM_ALLOW_DEBUG_INJECTION", old_debug_inj),
        ]:
            if old_val is None:
                os.environ.pop(key, None)
            else:
                os.environ[key] = old_val
        reset_pool_for_pytest()


def _test_retry_budget_exhausted_impl() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-budget",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-budget",
            project_id="proj-budget",
            operation_id="op-budget",
            env_type="dev",
            env_id="dev-budget",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-budget",
        {
            "source_bytes": b"# budget test\nSome content for budget.",
            "mime_type": "text/markdown",
            "artifact_id": "proj-budget:run-budget",
            "step_outputs": {},
        },
    )

    client = TestClient(app)

    # Step through prepare_case → normalize (6 steps)
    for i in range(9):
        resp = client.post(
            "/benchmarks/runs/run-budget/control",
            json={
                "command_id": f"cmd-budget-step-{i}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": "pipe-budget",
            },
        )
        assert resp.status_code == 200, f"Step {i} failed: {resp.text}"
    assert resp.json()["state"]["current_step_name"] == "map.reconstructable.normalize"

    # Inject verify failure
    inject_resp = client.post(
        "/benchmarks/runs/run-budget/control",
        json={
            "command_id": "cmd-budget-inject-1",
            "action": "inject_verify_fail",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-budget",
            "drift_at": "map.conceptual.lift.surface_fragments",
        },
    )
    assert inject_resp.status_code == 200

    # Step to compose_proposal then verify (2 more steps) — verify fails
    for step_label in (_get_compose_step(), _get_verify_gate_step()):
        resp = client.post(
            "/benchmarks/runs/run-budget/control",
            json={
                "command_id": f"cmd-budget-{step_label}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": "pipe-budget",
            },
        )
        assert resp.status_code == 200

    # Verify that verify failed
    state_after_fail = resp.json()["state"]
    assert state_after_fail["current_step_name"] == _get_verify_gate_step()

    # First retry re-enters at decompose (budget goes from 3 → 2)
    retry1_resp = client.post(
        "/benchmarks/runs/run-budget/control",
        json={
            "command_id": "cmd-budget-retry1",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-budget",
        },
    )
    assert retry1_resp.status_code == 200
    # Should have re-entered at decompose
    assert retry1_resp.json()["state"]["current_step_name"] == "map.conceptual.lift.surface_fragments"

    # Step through decompose → normalize again (5 steps: decompose already done, need embed_decomposed→lift→embed_lifted→candidate_search→normalize)
    for i in range(8):
        resp = client.post(
            "/benchmarks/runs/run-budget/control",
            json={
                "command_id": f"cmd-budget-retry1-step-{i}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": "pipe-budget",
            },
        )
        assert resp.status_code == 200
    assert resp.json()["state"]["current_step_name"] == "map.reconstructable.normalize"

    # Inject another verify failure
    inject2_resp = client.post(
        "/benchmarks/runs/run-budget/control",
        json={
            "command_id": "cmd-budget-inject-2",
            "action": "inject_verify_fail",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-budget",
            "drift_at": "map.conceptual.lift.surface_fragments",
        },
    )
    assert inject2_resp.status_code == 200

    # Step to compose_proposal then verify — verify fails again
    for step_label in ("compose_proposal_2", "verify_2"):
        resp = client.post(
            "/benchmarks/runs/run-budget/control",
            json={
                "command_id": f"cmd-budget-{step_label}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": "pipe-budget",
            },
        )
        assert resp.status_code == 200

    # Second retry re-enters at decompose (budget goes from 2 → 1)
    retry2_resp = client.post(
        "/benchmarks/runs/run-budget/control",
        json={
            "command_id": "cmd-budget-retry2",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-budget",
        },
    )
    assert retry2_resp.status_code == 200
    assert retry2_resp.json()["state"]["current_step_name"] == "map.conceptual.lift.surface_fragments"

    # Step through again: decompose → normalize (5 steps)
    for i in range(8):
        resp = client.post(
            "/benchmarks/runs/run-budget/control",
            json={
                "command_id": f"cmd-budget-retry2-step-{i}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": "pipe-budget",
            },
        )
        assert resp.status_code == 200
    assert resp.json()["state"]["current_step_name"] == "map.reconstructable.normalize"

    # Inject THIRD verify failure
    inject3_resp = client.post(
        "/benchmarks/runs/run-budget/control",
        json={
            "command_id": "cmd-budget-inject-3",
            "action": "inject_verify_fail",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-budget",
            "drift_at": "map.conceptual.lift.surface_fragments",
        },
    )
    assert inject3_resp.status_code == 200

    # Step to compose_proposal then verify — verify fails THIRD time
    for step_label in ("compose_proposal_3", "verify_3"):
        resp = client.post(
            "/benchmarks/runs/run-budget/control",
            json={
                "command_id": f"cmd-budget-{step_label}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": "pipe-budget",
            },
        )
        assert resp.status_code == 200

    # Third retry attempt — budget is now 0, should terminate
    exhausted_resp = client.post(
        "/benchmarks/runs/run-budget/control",
        json={
            "command_id": "cmd-budget-exhausted",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-budget",
        },
    )
    assert exhausted_resp.status_code == 200
    exhausted_body = exhausted_resp.json()

    # Run should be terminated with budget_exhausted status
    assert exhausted_body["state"]["execution_state"] == "budget_exhausted"


def test_control_next_step_from_successful_verify_advances_to_promote_commit(monkeypatch: pytest.MonkeyPatch) -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-8",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-8",
            project_id="proj-8",
            operation_id="op-8",
            env_type="dev",
            env_id="dev-8",
            execution_mode="manual",
            execution_state="paused",
            current_step_name=_get_verify_gate_step(),
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-verify-8",
            run_id="run-8",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-8",
            project_id="proj-8",
            operation_id="op-8",
            env_type="dev",
            env_id="dev-8",
            step_name=_get_verify_gate_step(),
            step_id="step-verify-8",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-01-01T00:00:00Z",
            ended_at="2026-01-01T00:00:01Z",
            duration_ms=1000,
            metrics={"executor": "ikam.forja.debug_execution", "passed": True},
            error=None,
        )
    )
    STORE.set_debug_runtime_context(
        "run-8",
        {
            "source_bytes": b"# run-8\ntext",
            "mime_type": "text/markdown",
            "artifact_id": "proj-8:run-8",
            "step_outputs": {
                "decomposition": {
                    "root_fragments": [{"cas_id": "f1", "mime_type": "text/markdown"}],
                },
                "verification": {"passed": True},
            },
        },
    )

    monkeypatch.setenv("IKAM_ASYNC_NEXT_STEP", "0")
    client = TestClient(app)
    next_resp = client.post(
        "/benchmarks/runs/run-8/control",
        json={
            "command_id": "cmd-next-promote-1",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-8",
        },
    )
    assert next_resp.status_code == 200
    assert next_resp.json()["state"]["current_step_name"] == "map.conceptual.commit.semantic_only"

    stream_resp = client.get(
        "/benchmarks/runs/run-8/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": "pipe-8"},
    )
    body = stream_resp.json()
    latest = body["events"][-1]
    assert latest["step_name"] == "map.conceptual.commit.semantic_only"
    assert latest["attempt_index"] == 1
    assert latest["retry_parent_step_id"] is None


def test_debug_step_detail_uses_canonical_schema_for_decompose() -> None:
    """Debug step detail returns the canonical hard-cutover schema.

    Legacy top-level ad-hoc fields (like "fragments") are not allowed.
    """
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-drill",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-drill",
            project_id="proj-drill",
            operation_id="op-drill",
            env_type="dev",
            env_id="dev-drill",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-drill",
        {
            "source_bytes": b"# drill test\nParagraph one.\n\n## Section\nParagraph two.",
            "mime_type": "text/markdown",
            "artifact_id": "proj-drill:run-drill",
            "step_outputs": {},
        },
    )

    client = TestClient(app)

    # next_step from prepare_case → executes decompose
    resp = client.post(
        "/benchmarks/runs/run-drill/control",
        json={
            "command_id": "cmd-drill-1",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-drill",
        },
    )
    assert resp.status_code == 200
    assert resp.json()["state"]["current_step_name"] == "map.conceptual.lift.surface_fragments"

    # Get the step_id from the event stream
    stream_resp = client.get(
        "/benchmarks/runs/run-drill/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": "pipe-drill"},
    )
    events = stream_resp.json()["events"]
    decompose_event = event_with_step(events, "map.conceptual.lift.surface_fragments")
    step_id = decompose_event["step_id"]

    # Drill through: GET the step detail
    detail_resp = client.get(f"/benchmarks/runs/run-drill/debug-step/{step_id}/detail")
    assert detail_resp.status_code == 200

    data = detail_resp.json()
    assert data["schema_version"] == "v1"
    assert data["step_name"] == "map.conceptual.lift.surface_fragments"
    assert data["pipeline_id"] == "compression-rerender/v1"
    assert data["pipeline_run_id"] == "pipe-drill"
    assert data["run_id"] == "run-drill"
    assert data["step_id"] == step_id
    assert data["attempt_index"] == 1

    for required in ("outcome", "why", "inputs", "outputs", "checks", "lineage"):
        assert required in data

    # Hard cutover: old ad-hoc top-level fields are no longer valid.
    assert "fragments" not in data

    lineage = data["lineage"]
    assert "roots" in lineage and isinstance(lineage["roots"], list)
    assert "nodes" in lineage and isinstance(lineage["nodes"], list)
    assert "edges" in lineage and isinstance(lineage["edges"], list)


def test_debug_step_detail_exposes_next_transition_id() -> None:
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-next-transition",
            project_id="proj-next-transition",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-next-transition", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-next-transition",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-next-transition",
            project_id="proj-next-transition",
            operation_id="op-next-transition",
            env_type="dev",
            env_id="dev-next-transition",
            execution_mode="manual",
            execution_state="paused",
            current_step_name=_get_verify_gate_step(),
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-next-transition",
            run_id="run-next-transition",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-next-transition",
            project_id="proj-next-transition",
            operation_id="op-next-transition",
            env_type="dev",
            env_id="dev-next-transition",
            step_name=_get_verify_gate_step(),
            step_id="step-next-transition",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
            metrics={"next_transition_id": "map.conceptual.commit.semantic_only"},
        )
    )

    client = TestClient(app)
    response = client.get("/benchmarks/runs/run-next-transition/debug-step/step-next-transition/detail")
    assert response.status_code == 200
    body = response.json()
    assert body["next_transition_id"] == "map.conceptual.commit.semantic_only"


def test_debug_step_detail_exposes_operation_and_produced_fragment_ids() -> None:
    expected_ref = "refs/heads/run/dev-op-detail"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-op-detail",
            project_id="proj-op-detail",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-op-detail", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-op-detail",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-op-detail",
            project_id="proj-op-detail",
            operation_id="op-op-detail",
            env_type="dev",
            env_id="dev-op-detail",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="map.conceptual.normalize.discovery",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-op-detail",
            run_id="run-op-detail",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-op-detail",
            project_id="proj-op-detail",
            operation_id="op-op-detail",
            env_type="dev",
            env_id="dev-op-detail",
            step_name="map.conceptual.normalize.discovery",
            step_id="step-op-detail",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
            metrics={
                "operation_ref": "modelado/operators/lift",
                "operation_params": {"chunking_policy_id": "default"},
            },
        )
    )
    STORE.set_debug_runtime_context(
        "run-op-detail",
        {
            "step_outputs": {
                "lifted": [
                    {"id": "ir-1", "mime_type": "application/ikam-proposition+json"},
                    {"id": "ir-2", "mime_type": "application/ikam-proposition+json"},
                ]
            }
        },
    )

    client = TestClient(app)
    response = client.get("/benchmarks/runs/run-op-detail/debug-step/step-op-detail/detail")
    assert response.status_code == 200
    body = response.json()
    assert body["operation_ref"] == "modelado/operators/lift"
    assert body["operation_params"] == {"chunking_policy_id": "default"}
    assert body["outcome"]["ref"] == expected_ref
    assert "produced_fragment_ids" in body
    assert isinstance(body["produced_fragment_ids"], list)


def test_debug_step_detail_exposes_step_boundaries_contract() -> None:
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-step-boundaries",
            project_id="proj-step-boundaries",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-step-boundaries", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-step-boundaries",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-boundaries",
            project_id="proj-step-boundaries",
            operation_id="op-step-boundaries",
            env_type="dev",
            env_id="dev-step-boundaries",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="map.conceptual.normalize.discovery",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-step-boundaries",
            run_id="run-step-boundaries",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-boundaries",
            project_id="proj-step-boundaries",
            operation_id="op-step-boundaries",
            env_type="dev",
            env_id="dev-step-boundaries",
            step_name="map.conceptual.normalize.discovery",
            step_id="step-boundaries",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    STORE.set_debug_runtime_context("run-step-boundaries", {"step_outputs": {}})

    client = TestClient(app)
    response = client.get("/benchmarks/runs/run-step-boundaries/debug-step/step-boundaries/detail")

    assert response.status_code == 200
    body = response.json()
    assert "step_boundaries" in body
    assert set(body["step_boundaries"].keys()) == {
        "input_boundary",
        "transition",
        "output_boundary",
        "ikam_environment_before",
        "ikam_environment_after",
        "handoff_to_next",
    }


def test_debug_step_detail_exposes_ikam_environment_before_after() -> None:
    expected_ref = "refs/heads/run/dev-step-env"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-step-env",
            project_id="proj-step-env",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-step-env", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-step-env",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-env",
            project_id="proj-step-env",
            operation_id="op-step-env",
            env_type="dev",
            env_id="dev-step-env",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="map.conceptual.normalize.discovery",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-step-env",
            run_id="run-step-env",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-env",
            project_id="proj-step-env",
            operation_id="op-step-env",
            env_type="dev",
            env_id="dev-step-env",
            step_name="map.conceptual.normalize.discovery",
            step_id="step-step-env",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
            metrics={
                "trace": {
                    "marking_before_ref": "marking://before-step-env",
                    "marking_after_ref": "marking://after-step-env",
                    "trace_fragment_id": "trace-fragment-step-env",
                }
            },
        )
    )
    STORE.set_debug_runtime_context("run-step-env", {"step_outputs": {}})

    client = TestClient(app)
    response = client.get("/benchmarks/runs/run-step-env/debug-step/step-step-env/detail")

    assert response.status_code == 200
    body = response.json()
    assert body["outcome"]["ref"] == expected_ref
    assert body["step_boundaries"]["ikam_environment_before"]["active_ref"] == expected_ref
    assert body["step_boundaries"]["ikam_environment_after"]["active_ref"] == expected_ref
    assert body["step_boundaries"]["ikam_environment_before"]["marking_ref"] == "marking://before-step-env"
    assert body["step_boundaries"]["ikam_environment_after"]["marking_ref"] == "marking://after-step-env"
    assert body["step_boundaries"]["ikam_environment_before"]["trace_fragment_id"] == "trace-fragment-step-env"
    assert body["step_boundaries"]["ikam_environment_after"]["trace_fragment_id"] == "trace-fragment-step-env"


def test_debug_step_detail_exposes_step_trace_visibility_contract() -> None:
    expected_ref = "refs/heads/run/dev-step-trace"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-step-trace",
            project_id="proj-step-trace",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-step-trace", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-step-trace",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-trace",
            project_id="proj-step-trace",
            operation_id="op-step-trace",
            env_type="dev",
            env_id="dev-step-trace",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="map.conceptual.normalize.discovery",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-step-trace",
            run_id="run-step-trace",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-trace",
            project_id="proj-step-trace",
            operation_id="op-step-trace",
            env_type="dev",
            env_id="dev-step-trace",
            step_name="map.conceptual.normalize.discovery",
            step_id="step-step-trace",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
            metrics={
                "next_transition_id": "map.reconstructable.embed",
                "trace": {
                    "workflow_id": "wf-step-trace",
                    "request_id": "req-step-trace",
                    "executor_id": "executor://python-primary",
                    "executor_kind": "python-executor",
                    "transition_id": "transition:dispatch-parse",
                    "marking_before_ref": "marking://before-step-trace",
                    "marking_after_ref": "marking://after-step-trace",
                    "enabled_transition_ids": ["transition:review", "transition:commit"],
                    "topic_sequence": [
                        {"topic": "workflow.events", "event_type": "execution.queued", "status": "queued"},
                        {"topic": "execution.progress", "event_type": "execution.running", "status": "running"},
                        {"topic": "execution.results", "event_type": "execution.completed", "status": "succeeded"},
                    ],
                    "timeline": [
                        {
                            "topic": "workflow.events",
                            "event_type": "execution.queued",
                            "status": "queued",
                            "occurred_at": "2026-03-01T00:00:00Z",
                            "payload": {"message": "queued"},
                        },
                        {
                            "topic": "execution.results",
                            "event_type": "execution.completed",
                            "status": "succeeded",
                            "occurred_at": "2026-03-01T00:00:01Z",
                            "payload": {"documents": 1},
                        },
                    ],
                    "raw_events": [
                        {
                            "topic": "execution.results",
                            "payload": {"documents": 1},
                        }
                    ],
                    "trace_id": "trace-step-trace",
                    "trace_fragment_id": "trace-fragment-step-trace",
                },
                "logs": {
                    "stdout_lines": ["python stdout line"],
                    "stderr_lines": ["python stderr line"],
                },
                "executor_logs": {
                    "stdout_lines": ["python stdout line"],
                    "stderr_lines": ["python stderr line"],
                },
                "system_logs": {
                    "stdout_lines": ["[parse.chunk] step started at 2026-03-01T00:00:00Z"],
                    "stderr_lines": [],
                },
            },
        )
    )

    client = TestClient(app)
    response = client.get("/benchmarks/runs/run-step-trace/debug-step/step-step-trace/detail")

    assert response.status_code == 200
    body = response.json()
    assert EXPECTED_STEP_TRACE_KEYS.issubset(body["trace"].keys())
    assert body["outcome"]["ref"] == expected_ref
    assert "env_type" not in body["outcome"]
    assert "env_id" not in body["outcome"]
    assert body["trace"]["workflow_id"] == "wf-step-trace"
    assert body["trace"]["request_id"] == "req-step-trace"
    assert body["trace"]["executor_id"] == "executor://python-primary"
    assert body["trace"]["executor_kind"] == "python-executor"
    assert body["trace"]["transition_id"] == "transition:dispatch-parse"
    assert body["trace"]["marking_before_ref"] == "marking://before-step-trace"
    assert body["trace"]["marking_after_ref"] == "marking://after-step-trace"
    assert body["trace"]["enabled_transition_ids"] == ["transition:review", "transition:commit"]
    assert body["trace"]["trace_id"] == "trace-step-trace"
    assert body["trace"]["trace_fragment_id"] == "trace-fragment-step-trace"
    assert "committed_trace_fragment_id" not in body["trace"]
    assert body["trace"]["topic_sequence"] == [
        {"topic": "workflow.events", "event_type": "execution.queued", "status": "queued"},
        {"topic": "execution.progress", "event_type": "execution.running", "status": "running"},
        {"topic": "execution.results", "event_type": "execution.completed", "status": "succeeded"},
    ]
    assert body["trace"]["timeline"] == [
        {
            "topic": "workflow.events",
            "event_type": "execution.queued",
            "status": "queued",
            "occurred_at": "2026-03-01T00:00:00Z",
            "payload": {"message": "queued"},
        },
        {
            "topic": "execution.results",
            "event_type": "execution.completed",
            "status": "succeeded",
            "occurred_at": "2026-03-01T00:00:01Z",
            "payload": {"documents": 1},
        },
    ]
    assert body["trace"]["raw_events"] == [
        {
            "topic": "execution.results",
            "payload": {"documents": 1},
        }
    ]


def test_debug_step_detail_exposes_step_boundaries_contract() -> None:
    expected_ref = "refs/heads/run/dev-step-boundaries"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-step-boundaries",
            project_id="proj-step-boundaries",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-step-boundaries", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-step-boundaries",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-boundaries",
            project_id="proj-step-boundaries",
            operation_id="op-step-boundaries",
            env_type="dev",
            env_id="dev-step-boundaries",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="map.conceptual.normalize.discovery",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-step-boundaries",
            run_id="run-step-boundaries",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-boundaries",
            project_id="proj-step-boundaries",
            operation_id="op-step-boundaries",
            env_type="dev",
            env_id="dev-step-boundaries",
            step_name="map.conceptual.normalize.discovery",
            step_id="step-step-boundaries",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
            metrics={
                "trace": {
                    "workflow_id": "wf-step-boundaries",
                    "request_id": "req-step-boundaries",
                    "executor_id": "executor://python-primary",
                    "executor_kind": "python-executor",
                    "transition_id": "transition:normalize",
                    "marking_before_ref": "marking://before-step-boundaries",
                    "marking_after_ref": "marking://after-step-boundaries",
                    "enabled_transition_ids": ["transition:commit"],
                    "trace_fragment_id": "trace-fragment-step-boundaries",
                },
                "operation_ref": "modelado/operators/normalize",
                "operation_params": {"mode": "strict"},
            },
        )
    )
    STORE.set_debug_runtime_context(
        "run-step-boundaries",
        {
            "artifact_id": "artifact:step-boundaries",
            "mime_type": "text/markdown",
            "step_outputs": {
                "fragment_ids": ["frag-step-boundaries"],
                "program_ids": ["prog-step-boundaries"],
            },
        },
    )

    client = TestClient(app)
    response = client.get("/benchmarks/runs/run-step-boundaries/debug-step/step-step-boundaries/detail")

    assert response.status_code == 200
    body = response.json()
    assert body["outcome"]["ref"] == expected_ref
    assert "step_boundaries" in body
    assert {
        "input_boundary",
        "transition",
        "output_boundary",
        "ikam_environment_before",
        "ikam_environment_after",
        "handoff_to_next",
    }.issubset(body["step_boundaries"].keys())


def test_debug_step_detail_exposes_ikam_environment_before_after_blocks() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-step-env-boundaries",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-env-boundaries",
            project_id="proj-step-env-boundaries",
            operation_id="op-step-env-boundaries",
            env_type="dev",
            env_id="dev-step-env-boundaries",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="map.conceptual.normalize.discovery",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-step-env-boundaries",
            run_id="run-step-env-boundaries",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-env-boundaries",
            project_id="proj-step-env-boundaries",
            operation_id="op-step-env-boundaries",
            env_type="dev",
            env_id="dev-step-env-boundaries",
            step_name="map.conceptual.normalize.discovery",
            step_id="step-step-env-boundaries",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
            metrics={
                "trace": {
                    "marking_before_ref": "marking://before-env-boundaries",
                    "marking_after_ref": "marking://after-env-boundaries",
                    "trace_fragment_id": "trace-fragment-env-boundaries",
                }
            },
        )
    )
    STORE.set_debug_runtime_context(
        "run-step-env-boundaries",
        {
            "artifact_id": "artifact:env-boundaries",
            "mime_type": "text/plain",
            "step_outputs": {"fragment_ids": ["frag-env-boundaries"]},
        },
    )

    client = TestClient(app)
    response = client.get(
        "/benchmarks/runs/run-step-env-boundaries/debug-step/step-step-env-boundaries/detail"
    )

    assert response.status_code == 200
    body = response.json()
    before = body["step_boundaries"]["ikam_environment_before"]
    after = body["step_boundaries"]["ikam_environment_after"]
    assert before["active_ref"] == "refs/heads/run/dev-step-env-boundaries"
    assert after["active_ref"] == "refs/heads/run/dev-step-env-boundaries"
    assert before["marking_ref"] == "marking://before-env-boundaries"
    assert after["marking_ref"] == "marking://after-env-boundaries"
    assert before["trace_fragment_id"] == "trace-fragment-env-boundaries"
    assert after["trace_fragment_id"] == "trace-fragment-env-boundaries"


def test_debug_step_detail_prefers_actual_next_step_inputs_for_handoff() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-step-handoff",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-handoff",
            project_id="proj-step-handoff",
            operation_id="op-step-handoff",
            env_type="dev",
            env_id="dev-step-handoff",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="map.reconstructable.embed",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-step-handoff-current",
            run_id="run-step-handoff",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-handoff",
            project_id="proj-step-handoff",
            operation_id="op-step-handoff",
            env_type="dev",
            env_id="dev-step-handoff",
            step_name="map.conceptual.normalize.discovery",
            step_id="step-step-handoff-current",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-step-handoff-next",
            run_id="run-step-handoff",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-handoff",
            project_id="proj-step-handoff",
            operation_id="op-step-handoff",
            env_type="dev",
            env_id="dev-step-handoff",
            step_name="map.reconstructable.embed",
            step_id="step-step-handoff-next",
            status="pending",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:02Z",
            ended_at=None,
            duration_ms=None,
            metrics={},
        )
    )
    STORE.set_debug_runtime_context(
        "run-step-handoff",
        {
            "artifact_id": "artifact:handoff",
            "mime_type": "text/plain",
            "step_output_snapshots": {
                "step-step-handoff-current": {
                    "fragment_ids": ["frag-current"],
                    "program_ids": ["prog-current"],
                },
                "step-step-handoff-next": {
                    "inputs": {
                        "artifact_ids": ["artifact:handoff"],
                        "fragment_ids": ["frag-forwarded"],
                        "program_ids": ["prog-forwarded"],
                    }
                },
            },
        },
    )

    client = TestClient(app)
    response = client.get("/benchmarks/runs/run-step-handoff/debug-step/step-step-handoff-current/detail")

    assert response.status_code == 200
    handoff = response.json()["step_boundaries"]["handoff_to_next"]
    assert handoff["next_step_name"] == "map.reconstructable.embed"
    assert handoff["source"] == "next_step_inputs"
    assert handoff["forwarded_fragment_ids"] == ["frag-forwarded"]
    assert handoff["forwarded_program_ids"] == ["prog-forwarded"]


def test_debug_step_detail_falls_back_to_declared_outputs_for_handoff() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-step-handoff-fallback",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-handoff-fallback",
            project_id="proj-step-handoff-fallback",
            operation_id="op-step-handoff-fallback",
            env_type="dev",
            env_id="dev-step-handoff-fallback",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="map.reconstructable.embed",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-step-handoff-fallback-current",
            run_id="run-step-handoff-fallback",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-handoff-fallback",
            project_id="proj-step-handoff-fallback",
            operation_id="op-step-handoff-fallback",
            env_type="dev",
            env_id="dev-step-handoff-fallback",
            step_name="map.conceptual.normalize.discovery",
            step_id="step-step-handoff-fallback-current",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    STORE.set_debug_runtime_context(
        "run-step-handoff-fallback",
        {
            "artifact_id": "artifact:handoff-fallback",
            "mime_type": "text/plain",
            "step_output_snapshots": {
                "step-step-handoff-fallback-current": {
                    "fragment_ids": ["frag-fallback"],
                    "program_ids": ["prog-fallback"],
                },
            },
        },
    )

    client = TestClient(app)
    response = client.get(
        "/benchmarks/runs/run-step-handoff-fallback/debug-step/step-step-handoff-fallback-current/detail"
    )

    assert response.status_code == 200
    handoff = response.json()["step_boundaries"]["handoff_to_next"]
    assert handoff["source"] == "declared_outputs_fallback"
    assert handoff["forwarded_fragment_ids"] == ["frag-fallback"]
    assert handoff["forwarded_program_ids"] == ["prog-fallback"]


def test_debug_step_detail_exposes_transition_validation_for_load_documents_outputs() -> None:
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-load-documents-validation",
            project_id="proj-load-documents-validation",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="proj-load-documents-validation",
                fragments=[
                    {
                        "id": "frag-loaded-document-1",
                        "cas_id": "frag-loaded-document-1",
                        "mime_type": "application/vnd.ikam.loaded-document+json",
                        "value": {
                            "document_id": "doc-1",
                            "text": "Invoice 123 is ready",
                            "metadata": {"source": "brief.md"},
                            "artifact_id": "artifact:load-documents-validation",
                            "filename": "brief.md",
                            "mime_type": "text/markdown",
                        },
                        "meta": {
                            "env_type": "dev",
                            "env_id": "dev-load-documents-validation",
                            "step_id": "step-load-documents-validation",
                            "attempt_index": 1,
                            "artifact_id": "artifact:load-documents-validation",
                        },
                    }
                ],
            ),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-load-documents-validation",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-load-documents-validation",
            project_id="proj-load-documents-validation",
            operation_id="op-load-documents-validation",
            env_type="dev",
            env_id="dev-load-documents-validation",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-load-documents-validation",
            run_id="run-load-documents-validation",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-load-documents-validation",
            project_id="proj-load-documents-validation",
            operation_id="op-load-documents-validation",
            env_type="dev",
            env_id="dev-load-documents-validation",
            step_name="load.documents",
            step_id="step-load-documents-validation",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    STORE.set_debug_runtime_context(
        "run-load-documents-validation",
        {
            "artifact_id": "artifact:load-documents-validation",
            "mime_type": "text/markdown",
            "step_outputs": {
                "document_fragment_refs": ["frag-loaded-document-1"],
                "documents": [
                    {
                        "id": "doc-1",
                        "text": "Invoice 123 is ready",
                        "artifact_id": "artifact:load-documents-validation",
                        "filename": "brief.md",
                        "mime_type": "text/markdown",
                        "metadata": {"source": "brief.md"},
                    }
                ],
                "document_loads": [
                    {
                        "artifact_id": "artifact:load-documents-validation",
                        "filename": "brief.md",
                        "mime_type": "text/markdown",
                        "status": "success",
                        "document_count": 1,
                    }
                ],
            },
        },
    )

    client = TestClient(app)
    response = client.get(
        "/benchmarks/runs/run-load-documents-validation/debug-step/step-load-documents-validation/detail"
    )

    assert response.status_code == 200
    body = response.json()
    assert "transition_validation" in body
    assert [spec["name"] for spec in body["transition_validation"]["specs"]] == [
        "input-url",
        "output-document-set",
    ]
    assert body["transition_validation"]["resolved_inputs"]["url"][0]["value"]["location"].endswith("/tests/fixtures/cases/s-local-retail-v01")
    assert body["transition_validation"]["resolved_outputs"]["document_set"][0]["value"]["kind"] == "document_set"
    assert body["transition_validation"]["resolved_outputs"]["document_set"][0]["value"]["artifact_head_ref"] == "artifact:load-documents-validation"
    assert body["transition_validation"]["resolved_outputs"]["document_set"][0]["value"]["subgraph_ref"] == "hot://run-load-documents-validation/document_set/step-load-documents-validation"
    assert body["transition_validation"]["resolved_outputs"]["document_set"][0]["inspection"] == {
        "value_kind": "document_set",
        "summary": "document_set 1 ref",
        "refs": ["frag-loaded-document-1"],
        "content": {
            "kind": "document_set",
            "artifact_head_ref": "artifact:load-documents-validation",
            "subgraph_ref": "hot://run-load-documents-validation/document_set/step-load-documents-validation",
            "document_refs": ["frag-loaded-document-1"],
        },
        "resolved_refs": [
            {
                "fragment_id": "frag-loaded-document-1",
                "cas_id": "frag-loaded-document-1",
                "mime_type": "application/vnd.ikam.loaded-document+json",
                "name": "brief.md",
                "inspection_ref": "inspect://fragment/frag-loaded-document-1",
                "value": {
                    "document_id": "doc-1",
                    "text": "Invoice 123 is ready",
                    "metadata": {"source": "brief.md"},
                    "artifact_id": "artifact:load-documents-validation",
                    "filename": "brief.md",
                    "mime_type": "text/markdown",
                },
            }
        ],
    }
    results = {item["name"]: item for item in body["transition_validation"]["results"]}
    assert results["input-url"]["status"] == "passed"
    assert results["output-document-set"]["status"] == "passed"


def test_debug_step_detail_refreshes_stale_persisted_load_documents_validation_with_named_refs() -> None:
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-load-documents-stale-validation",
            project_id="proj-load-documents-stale-validation",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="proj-load-documents-stale-validation",
                fragments=[
                    {
                        "id": "frag-loaded-document-stale-1",
                        "cas_id": "frag-loaded-document-stale-1",
                        "mime_type": "application/vnd.ikam.loaded-document+json",
                        "value": {
                            "document_id": "doc-stale-1",
                            "text": "Invoice 456 is ready",
                            "artifact_id": "artifact:load-documents-stale-validation",
                            "filename": "invoice-456.md",
                            "mime_type": "text/markdown",
                        },
                        "meta": {
                            "env_type": "dev",
                            "env_id": "dev-load-documents-stale-validation",
                            "step_id": "step-load-documents-stale-validation",
                            "attempt_index": 1,
                            "artifact_id": "artifact:load-documents-stale-validation",
                        },
                    }
                ],
            ),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-load-documents-stale-validation",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-load-documents-stale-validation",
            project_id="proj-load-documents-stale-validation",
            operation_id="op-load-documents-stale-validation",
            env_type="dev",
            env_id="dev-load-documents-stale-validation",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-load-documents-stale-validation",
            run_id="run-load-documents-stale-validation",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-load-documents-stale-validation",
            project_id="proj-load-documents-stale-validation",
            operation_id="op-load-documents-stale-validation",
            env_type="dev",
            env_id="dev-load-documents-stale-validation",
            step_name="load.documents",
            step_id="step-load-documents-stale-validation",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    STORE.set_debug_runtime_context(
        "run-load-documents-stale-validation",
        {
            "artifact_id": "artifact:load-documents-stale-validation",
            "mime_type": "text/markdown",
            "step_outputs": {
                "document_fragment_refs": ["frag-loaded-document-stale-1"],
                "documents": [
                    {
                        "id": "doc-stale-1",
                        "text": "Invoice 456 is ready",
                        "artifact_id": "artifact:load-documents-stale-validation",
                        "filename": "invoice-456.md",
                        "mime_type": "text/markdown",
                    }
                ],
                "document_loads": [
                    {
                        "artifact_id": "artifact:load-documents-stale-validation",
                        "filename": "invoice-456.md",
                        "mime_type": "text/markdown",
                        "status": "success",
                        "document_count": 1,
                    }
                ],
            },
            "step_transition_validations": {
                "step-load-documents-stale-validation": {
                    "specs": [
                        {"name": "input-url", "direction": "input", "kind": "type", "config": {"schema": {"title": "url"}}},
                        {"name": "output-document-set", "direction": "output", "kind": "type", "config": {"schema": {"title": "document_set"}}},
                    ],
                    "resolved_inputs": {
                        "url": [
                            {
                                "value": {
                                    "kind": "url",
                                    "location": "/repo/tests/fixtures/cases/s-local-retail-v01",
                                },
                                "inspection": {
                                    "value_kind": "url",
                                    "summary": "url /repo/tests/fixtures/cases/s-local-retail-v01",
                                    "refs": [],
                                    "content": {
                                        "kind": "url",
                                        "location": "/repo/tests/fixtures/cases/s-local-retail-v01",
                                    },
                                    "resolved_refs": [],
                                },
                            }
                        ]
                    },
                    "resolved_outputs": {
                        "document_set": [
                            {
                                "value": {
                                    "kind": "document_set",
                                    "artifact_head_ref": "artifact:load-documents-stale-validation",
                                    "subgraph_ref": "hot://run-load-documents-stale-validation/document_set/step-load-documents-stale-validation",
                                    "document_refs": ["frag-loaded-document-stale-1"],
                                },
                                "inspection": {
                                    "value_kind": "document_set",
                                    "summary": "document_set 1 ref",
                                    "refs": ["frag-loaded-document-stale-1"],
                                    "content": {
                                        "kind": "document_set",
                                        "artifact_head_ref": "artifact:load-documents-stale-validation",
                                        "subgraph_ref": "hot://run-load-documents-stale-validation/document_set/step-load-documents-stale-validation",
                                        "document_refs": ["frag-loaded-document-stale-1"],
                                    },
                                    "resolved_refs": [],
                                },
                            }
                        ]
                    },
                    "results": [
                        {"name": "input-url", "direction": "input", "kind": "type", "status": "passed", "matched_fragment_ids": ["input.url"], "evidence": {"matched_count": 1}},
                        {"name": "output-document-set", "direction": "output", "kind": "type", "status": "passed", "matched_fragment_ids": ["output.document_set"], "evidence": {"matched_count": 1}},
                    ],
                }
            },
        },
    )

    client = TestClient(app)
    response = client.get(
        "/benchmarks/runs/run-load-documents-stale-validation/debug-step/step-load-documents-stale-validation/detail"
    )

    assert response.status_code == 200
    body = response.json()
    resolved_refs = body["transition_validation"]["resolved_outputs"]["document_set"][0]["inspection"]["resolved_refs"]
    assert resolved_refs == [
        {
            "fragment_id": "frag-loaded-document-stale-1",
            "cas_id": "frag-loaded-document-stale-1",
            "mime_type": "application/vnd.ikam.loaded-document+json",
            "name": "invoice-456.md",
            "inspection_ref": "inspect://fragment/frag-loaded-document-stale-1",
            "value": {
                "document_id": "doc-stale-1",
                "text": "Invoice 456 is ready",
                "artifact_id": "artifact:load-documents-stale-validation",
                "filename": "invoice-456.md",
                "mime_type": "text/markdown",
            },
        }
    ]


def test_debug_step_detail_exposes_transition_validation_for_parse_chunk_inputs() -> None:
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-parse-chunk-validation",
            project_id="proj-parse-chunk-validation",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="proj-parse-chunk-validation",
                fragments=[
                    {
                        "id": "frag-loaded-document-parse",
                        "cas_id": "frag-loaded-document-parse",
                        "mime_type": "application/vnd.ikam.loaded-document+json",
                        "value": {
                            "document_id": "doc-parse-1",
                            "text": "Chunk this paragraph into segments",
                            "metadata": {"source": "brief.md"},
                            "artifact_id": "artifact:parse-chunk-validation",
                            "filename": "brief.md",
                            "mime_type": "text/markdown",
                        },
                        "meta": {
                            "env_type": "dev",
                            "env_id": "dev-parse-chunk-validation",
                            "step_id": "step-parse-chunk-validation",
                            "attempt_index": 1,
                            "artifact_id": "artifact:parse-chunk-validation",
                        },
                    },
                    {
                        "id": "frag-surface-segment-1",
                        "cas_id": "frag-surface-segment-1",
                        "mime_type": "application/vnd.ikam.map-segment+json",
                        "value": {
                            "text": "Chunk this paragraph into segments",
                            "segment_id": "segment-1",
                        },
                        "meta": {
                            "env_type": "dev",
                            "env_id": "dev-parse-chunk-validation",
                            "step_id": "step-parse-chunk-validation",
                            "attempt_index": 1,
                            "artifact_id": "artifact:parse-chunk-validation",
                        },
                    },
                ],
            ),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-parse-chunk-validation",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-parse-chunk-validation",
            project_id="proj-parse-chunk-validation",
            operation_id="op-parse-chunk-validation",
            env_type="dev",
            env_id="dev-parse-chunk-validation",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.chunk",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-parse-chunk-validation",
            run_id="run-parse-chunk-validation",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-parse-chunk-validation",
            project_id="proj-parse-chunk-validation",
            operation_id="op-parse-chunk-validation",
            env_type="dev",
            env_id="dev-parse-chunk-validation",
            step_name="parse.chunk",
            step_id="step-parse-chunk-validation",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    STORE.set_debug_runtime_context(
        "run-parse-chunk-validation",
        {
            "artifact_id": "artifact:parse-chunk-validation",
            "mime_type": "text/markdown",
            "step_output_snapshots": {
                "step-parse-chunk-validation": {
                    "inputs": {
                        "artifact_ids": ["artifact:parse-chunk-validation"],
                        "document_fragment_refs": ["frag-loaded-document-parse"],
                    },
                    "fragment_ids": ["frag-surface-segment-1"],
                }
            },
        },
    )

    client = TestClient(app)
    response = client.get(
        "/benchmarks/runs/run-parse-chunk-validation/debug-step/step-parse-chunk-validation/detail"
    )

    assert response.status_code == 200
    body = response.json()
    assert "transition_validation" in body
    assert [spec["name"] for spec in body["transition_validation"]["specs"]] == [
        "input-document-set",
        "output-chunk-extraction-set",
    ]
    assert body["transition_validation"]["resolved_inputs"]["document_set"][0]["value"]["kind"] == "document_set"
    assert body["transition_validation"]["resolved_outputs"]["chunk_extraction_set"][0]["value"]["kind"] == "chunk_extraction_set"
    assert body["transition_validation"]["resolved_inputs"]["document_set"][0]["value"]["subgraph_ref"] == "hot://run-parse-chunk-validation/document_set/step-parse-chunk-validation"
    assert body["transition_validation"]["resolved_outputs"]["chunk_extraction_set"][0]["value"]["source_subgraph_ref"] == "hot://run-parse-chunk-validation/document_set/step-parse-chunk-validation"
    assert body["transition_validation"]["resolved_outputs"]["chunk_extraction_set"][0]["value"]["subgraph_ref"] == "hot://run-parse-chunk-validation/chunk_extraction_set/step-parse-chunk-validation"
    inspection = body["transition_validation"]["resolved_outputs"]["chunk_extraction_set"][0]["inspection"]
    assert inspection["value_kind"] == "chunk_extraction_set"
    assert inspection["summary"] == "chunk_extraction_set 1 ref"
    assert inspection["refs"] == ["frag-surface-segment-1"]
    assert inspection["content"] == {
        "kind": "chunk_extraction_set",
        "source_subgraph_ref": "hot://run-parse-chunk-validation/document_set/step-parse-chunk-validation",
        "subgraph_ref": "hot://run-parse-chunk-validation/chunk_extraction_set/step-parse-chunk-validation",
        "extraction_refs": ["frag-surface-segment-1"],
    }
    assert inspection["resolved_refs"] == [
        {
            "fragment_id": "frag-surface-segment-1",
            "cas_id": "frag-surface-segment-1",
            "mime_type": "application/vnd.ikam.map-segment+json",
            "name": inspection["resolved_refs"][0]["name"],
            "inspection_ref": "inspect://fragment/frag-surface-segment-1",
            "value": {
                "text": "Chunk this paragraph into segments",
                "segment_id": "segment-1",
            },
        }
    ]
    assert body["input_validation"]["resolved_inputs"]["document_set"][0]["value"]["kind"] == "document_set"
    assert body["output_validation"]["resolved_outputs"]["chunk_extraction_set"][0]["value"]["kind"] == "chunk_extraction_set"
    results = {item["name"]: item for item in body["transition_validation"]["results"]}
    assert results["input-document-set"]["status"] == "passed"
    assert results["output-chunk-extraction-set"]["status"] == "passed"


def test_debug_step_detail_exposes_transition_validation_for_parse_entities_inputs() -> None:
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-parse-entities-validation",
            project_id="proj-parse-entities-validation",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="proj-parse-entities-validation",
                fragments=[
                    {
                        "id": "frag-surface-entity-1",
                        "cas_id": "frag-surface-entity-1",
                        "mime_type": "application/vnd.ikam.map-segment+json",
                        "value": {
                            "text": "Acme Corp acquired Beta LLC.",
                            "segment_id": "segment-entity-1",
                        },
                        "meta": {
                            "env_type": "dev",
                            "env_id": "dev-parse-entities-validation",
                            "step_id": "step-parse-entities-validation",
                            "attempt_index": 1,
                            "artifact_id": "artifact:parse-entities-validation",
                        },
                    }
                ],
            ),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-parse-entities-validation",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-parse-entities-validation",
            project_id="proj-parse-entities-validation",
            operation_id="op-parse-entities-validation",
            env_type="dev",
            env_id="dev-parse-entities-validation",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.entities_and_relationships",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-parse-entities-validation",
            run_id="run-parse-entities-validation",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-parse-entities-validation",
            project_id="proj-parse-entities-validation",
            operation_id="op-parse-entities-validation",
            env_type="dev",
            env_id="dev-parse-entities-validation",
            step_name="parse.entities_and_relationships",
            step_id="step-parse-entities-validation",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    STORE.set_debug_runtime_context(
        "run-parse-entities-validation",
        {
            "artifact_id": "artifact:parse-entities-validation",
            "mime_type": "text/markdown",
            "step_output_snapshots": {
                "step-parse-entities-validation": {
                    "inputs": {
                        "chunk_extraction_set_ref": "hot://run-parse-entities-validation/chunk_extraction_set/step-parse-chunk-validation",
                        "fragment_ids": ["frag-surface-entity-1"],
                    },
                    "fragment_ids": ["frag-surface-entity-1"],
                }
            },
        },
    )

    client = TestClient(app)
    response = client.get(
        "/benchmarks/runs/run-parse-entities-validation/debug-step/step-parse-entities-validation/detail"
    )

    assert response.status_code == 200
    body = response.json()
    assert "transition_validation" in body
    assert [spec["name"] for spec in body["transition_validation"]["specs"]] == [
        "input-chunk-extraction-set",
        "output-entity-relationship-set",
    ]
    assert body["transition_validation"]["resolved_inputs"]["chunk_extraction_set"][0]["value"]["kind"] == "chunk_extraction_set"
    assert body["transition_validation"]["resolved_inputs"]["chunk_extraction_set"][0]["value"]["subgraph_ref"] == "hot://run-parse-entities-validation/chunk_extraction_set/step-parse-chunk-validation"
    assert body["transition_validation"]["resolved_outputs"]["entity_relationship_set"][0]["value"]["kind"] == "entity_relationship_set"
    assert body["transition_validation"]["resolved_outputs"]["entity_relationship_set"][0]["value"]["subgraph_ref"] == "hot://run-parse-entities-validation/entity_relationship_set/step-parse-entities-validation"
    results = {item["name"]: item for item in body["transition_validation"]["results"]}
    assert results["input-chunk-extraction-set"]["status"] == "passed"
    assert results["output-entity-relationship-set"]["status"] == "passed"


def test_debug_step_detail_exposes_transition_validation_for_parse_claims_inputs() -> None:
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-parse-claims-validation",
            project_id="proj-parse-claims-validation",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="proj-parse-claims-validation",
                fragments=[
                    {
                        "id": "frag-surface-claim-1",
                        "cas_id": "frag-surface-claim-1",
                        "mime_type": "application/vnd.ikam.map-segment+json",
                        "value": {
                            "text": "Acme Corp acquired Beta LLC.",
                            "segment_id": "segment-claim-1",
                        },
                        "meta": {
                            "env_type": "dev",
                            "env_id": "dev-parse-claims-validation",
                            "step_id": "step-parse-claims-validation",
                            "attempt_index": 1,
                            "artifact_id": "artifact:parse-claims-validation",
                        },
                    }
                ],
            ),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-parse-claims-validation",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-parse-claims-validation",
            project_id="proj-parse-claims-validation",
            operation_id="op-parse-claims-validation",
            env_type="dev",
            env_id="dev-parse-claims-validation",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.claims",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-parse-claims-validation",
            run_id="run-parse-claims-validation",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-parse-claims-validation",
            project_id="proj-parse-claims-validation",
            operation_id="op-parse-claims-validation",
            env_type="dev",
            env_id="dev-parse-claims-validation",
            step_name="parse.claims",
            step_id="step-parse-claims-validation",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    STORE.set_debug_runtime_context(
        "run-parse-claims-validation",
        {
            "artifact_id": "artifact:parse-claims-validation",
            "mime_type": "text/markdown",
            "step_output_snapshots": {
                "step-parse-claims-validation": {
                    "inputs": {
                        "entity_relationship_set_ref": "hot://run-parse-claims-validation/entity_relationship_set/step-parse-entities-validation",
                        "fragment_ids": ["frag-surface-claim-1"],
                    },
                    "fragment_ids": ["frag-surface-claim-1"],
                }
            },
        },
    )

    client = TestClient(app)
    response = client.get(
        "/benchmarks/runs/run-parse-claims-validation/debug-step/step-parse-claims-validation/detail"
    )

    assert response.status_code == 200
    body = response.json()
    assert "transition_validation" in body
    assert [spec["name"] for spec in body["transition_validation"]["specs"]] == [
        "input-entity-relationship-set",
        "output-claim-set",
    ]
    assert body["transition_validation"]["resolved_inputs"]["entity_relationship_set"][0]["value"]["kind"] == "entity_relationship_set"
    assert body["transition_validation"]["resolved_inputs"]["entity_relationship_set"][0]["value"]["subgraph_ref"] == "hot://run-parse-claims-validation/entity_relationship_set/step-parse-entities-validation"
    assert body["transition_validation"]["resolved_outputs"]["claim_set"][0]["value"]["kind"] == "claim_set"
    assert body["transition_validation"]["resolved_outputs"]["claim_set"][0]["value"]["subgraph_ref"] == "hot://run-parse-claims-validation/claim_set/step-parse-claims-validation"
    results = {item["name"]: item for item in body["transition_validation"]["results"]}
    assert results["input-entity-relationship-set"]["status"] == "passed"
    assert results["output-claim-set"]["status"] == "passed"


def test_hot_subgraph_refs_resolve_through_store() -> None:
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-hot-store-resolution",
            project_id="proj-hot-store-resolution",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-hot-store-resolution", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-hot-store-resolution",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-store-resolution",
            project_id="proj-hot-store-resolution",
            operation_id="op-hot-store-resolution",
            env_type="dev",
            env_id="dev-hot-store-resolution",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    document_set_ref = STORE.put_hot_subgraph(
        run_id="run-hot-store-resolution",
        step_id="step-hot-store-resolution",
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "artifact:hot-store-resolution",
            "document_refs": ["frag-a", "frag-b"],
        },
    )

    resolved = STORE.get_hot_subgraph(document_set_ref)

    assert resolved is not None
    assert resolved["kind"] == "document_set"
    assert resolved["artifact_head_ref"] == "artifact:hot-store-resolution"


def test_next_step_hydrates_parse_chunk_inputs_from_runtime_hot_document_set(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-hot-runtime-handoff",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-handoff",
            project_id="proj-hot-runtime-handoff",
            operation_id="op-hot-runtime-handoff",
            env_type="dev",
            env_id="dev-hot-runtime-handoff",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-hot-runtime-handoff-load",
            run_id="run-hot-runtime-handoff",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-handoff",
            project_id="proj-hot-runtime-handoff",
            operation_id="op-hot-runtime-handoff",
            env_type="dev",
            env_id="dev-hot-runtime-handoff",
            step_name="load.documents",
            step_id="step-hot-runtime-handoff-load",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    document_set_ref = STORE.put_hot_subgraph(
        run_id="run-hot-runtime-handoff",
        step_id="step-hot-runtime-handoff-load",
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "artifact:hot-runtime-handoff",
            "subgraph_ref": "hot://run-hot-runtime-handoff/document_set/step-hot-runtime-handoff-load",
            "document_refs": ["frag-hot-doc-1", "frag-hot-doc-2"],
        },
    )
    STORE.set_debug_runtime_context(
        "run-hot-runtime-handoff",
        {
            "source_bytes": b"# hot handoff\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:hot-runtime-handoff",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "document_set_ref": document_set_ref,
        },
    )

    captured: dict[str, object] = {}

    async def _fake_execute_step(step_name: str, state, scope=None):
        captured["step_name"] = step_name
        captured["document_fragment_refs"] = list(state.outputs.get("document_fragment_refs") or [])
        captured["inputs"] = dict(state.outputs.get("inputs") or {})
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-hot-runtime-handoff/control",
            json={
                "command_id": "cmd-hot-runtime-handoff-parse",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-hot-runtime-handoff",
            },
        )

        assert response.status_code == 200
        assert captured["step_name"] == "map.conceptual.lift.surface_fragments"
        assert captured["document_fragment_refs"] == ["frag-hot-doc-1", "frag-hot-doc-2"]
        assert captured["inputs"] == {
            "document_set_ref": "hot://run-hot-runtime-handoff/document_set/step-hot-runtime-handoff-load",
            "document_fragment_refs": ["frag-hot-doc-1", "frag-hot-doc-2"],
        }
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_parse_chunk_fails_input_validation_before_execute(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-parse-chunk-input-gate",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-parse-chunk-input-gate",
            project_id="proj-parse-chunk-input-gate",
            operation_id="op-parse-chunk-input-gate",
            env_type="dev",
            env_id="dev-parse-chunk-input-gate",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-parse-chunk-input-gate",
        {
            "source_bytes": b"# input gate\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:parse-chunk-input-gate",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
        },
    )

    called = {"execute": False}

    async def _fake_execute_step(step_name: str, state, scope=None):
        called["execute"] = True
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-parse-chunk-input-gate/control",
            json={
                "command_id": "cmd-parse-chunk-input-gate",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-parse-chunk-input-gate",
            },
        )

        assert response.status_code == 200
        assert called["execute"] is False
        stream = client.get(
            "/benchmarks/runs/run-parse-chunk-input-gate/debug-stream",
            params={"pipeline_id": "ingestion-early-parse", "pipeline_run_id": "pipe-parse-chunk-input-gate"},
        )
        assert stream.status_code == 200
        event = next(item for item in stream.json()["events"] if item["step_name"] == "parse.chunk")
        assert event["status"] == "failed"
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_parse_chunk_fails_output_validation_and_does_not_publish_chunk_handoff(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-parse-chunk-output-gate",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-parse-chunk-output-gate",
            project_id="proj-parse-chunk-output-gate",
            operation_id="op-parse-chunk-output-gate",
            env_type="dev",
            env_id="dev-parse-chunk-output-gate",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    document_set_ref = STORE.put_hot_subgraph(
        run_id="run-parse-chunk-output-gate",
        step_id="step-parse-chunk-output-gate-load",
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "artifact:parse-chunk-output-gate",
            "subgraph_ref": "hot://run-parse-chunk-output-gate/document_set/step-parse-chunk-output-gate-load",
            "document_refs": ["frag-output-gate-doc-1"],
        },
    )
    STORE.set_debug_runtime_context(
        "run-parse-chunk-output-gate",
        {
            "source_bytes": b"# output gate\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:parse-chunk-output-gate",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "document_set_ref": document_set_ref,
        },
    )

    async def _fake_execute_step(step_name: str, state, scope=None):
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-parse-chunk-output-gate/control",
            json={
                "command_id": "cmd-parse-chunk-output-gate",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-parse-chunk-output-gate",
            },
        )

        assert response.status_code == 200
        stream = client.get(
            "/benchmarks/runs/run-parse-chunk-output-gate/debug-stream",
            params={"pipeline_id": "ingestion-early-parse", "pipeline_run_id": "pipe-parse-chunk-output-gate"},
        )
        assert stream.status_code == 200
        event = next(item for item in stream.json()["events"] if item["step_name"] == "parse.chunk")
        assert event["status"] == "failed"
        runtime_context = STORE.get_debug_runtime_context("run-parse-chunk-output-gate")
        assert runtime_context is not None
        assert "chunk_extraction_set_ref" not in runtime_context
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_debug_step_detail_prefers_persisted_parse_chunk_transition_validation(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-parse-chunk-persisted-validation",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-parse-chunk-persisted-validation",
            project_id="proj-parse-chunk-persisted-validation",
            operation_id="op-parse-chunk-persisted-validation",
            env_type="dev",
            env_id="dev-parse-chunk-persisted-validation",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    document_set_ref = STORE.put_hot_subgraph(
        run_id="run-parse-chunk-persisted-validation",
        step_id="step-parse-chunk-persisted-validation-load",
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "artifact:parse-chunk-persisted-validation",
            "subgraph_ref": "hot://run-parse-chunk-persisted-validation/document_set/step-parse-chunk-persisted-validation-load",
            "document_refs": ["frag-persisted-doc-1"],
        },
    )
    STORE.set_debug_runtime_context(
        "run-parse-chunk-persisted-validation",
        {
            "source_bytes": b"# persisted validation\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:parse-chunk-persisted-validation",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "document_set_ref": document_set_ref,
        },
    )

    async def _fake_execute_step(step_name: str, state, scope=None):
        state.outputs["fragment_ids"] = ["frag-persisted-chunk-1"]
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        control_response = client.post(
            "/benchmarks/runs/run-parse-chunk-persisted-validation/control",
            json={
                "command_id": "cmd-parse-chunk-persisted-validation",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-parse-chunk-persisted-validation",
            },
        )

        assert control_response.status_code == 200
        stream_response = client.get(
            "/benchmarks/runs/run-parse-chunk-persisted-validation/debug-stream",
            params={
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-parse-chunk-persisted-validation",
            },
        )
        assert stream_response.status_code == 200
        events = stream_response.json()["events"]
        parse_chunk_event = next(item for item in events if item["step_name"] == "parse.chunk")

        runtime_context = STORE.get_debug_runtime_context("run-parse-chunk-persisted-validation")
        assert runtime_context is not None
        persisted = runtime_context["step_transition_validations"][parse_chunk_event["step_id"]]
        assert [spec["name"] for spec in persisted["specs"]] == ["input-document-set", "output-chunk-extraction-set"]
        persisted_input = runtime_context["step_input_validations"][parse_chunk_event["step_id"]]
        persisted_output = runtime_context["step_output_validations"][parse_chunk_event["step_id"]]
        assert [spec["name"] for spec in persisted_input["specs"]] == ["input-document-set"]
        assert [spec["name"] for spec in persisted_output["specs"]] == ["output-chunk-extraction-set"]

        detail_response = client.get(
            f"/benchmarks/runs/run-parse-chunk-persisted-validation/debug-step/{parse_chunk_event['step_id']}/detail"
        )
        assert detail_response.status_code == 200
        body = detail_response.json()
        assert [spec["name"] for spec in body["input_validation"]["specs"]] == ["input-document-set"]
        assert [spec["name"] for spec in body["output_validation"]["specs"]] == ["output-chunk-extraction-set"]
        assert [spec["name"] for spec in body["transition_validation"]["specs"]] == ["input-document-set", "output-chunk-extraction-set"]
        assert body["transition_validation"]["resolved_inputs"]["document_set"][0]["value"]["kind"] == "document_set"
        assert body["transition_validation"]["resolved_outputs"]["chunk_extraction_set"][0]["value"]["kind"] == "chunk_extraction_set"
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_next_step_persists_runtime_hot_chunk_extraction_set_after_parse_chunk(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-hot-runtime-chunk-set",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-chunk-set",
            project_id="proj-hot-runtime-chunk-set",
            operation_id="op-hot-runtime-chunk-set",
            env_type="dev",
            env_id="dev-hot-runtime-chunk-set",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-hot-runtime-chunk-set-load",
            run_id="run-hot-runtime-chunk-set",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-chunk-set",
            project_id="proj-hot-runtime-chunk-set",
            operation_id="op-hot-runtime-chunk-set",
            env_type="dev",
            env_id="dev-hot-runtime-chunk-set",
            step_name="load.documents",
            step_id="step-hot-runtime-chunk-set-load",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    document_set_ref = STORE.put_hot_subgraph(
        run_id="run-hot-runtime-chunk-set",
        step_id="step-hot-runtime-chunk-set-load",
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "artifact:hot-runtime-chunk-set",
            "subgraph_ref": "hot://run-hot-runtime-chunk-set/document_set/step-hot-runtime-chunk-set-load",
            "document_refs": ["frag-hot-doc-1"],
        },
    )
    STORE.set_debug_runtime_context(
        "run-hot-runtime-chunk-set",
        {
            "source_bytes": b"# hot chunk set\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:hot-runtime-chunk-set",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "document_set_ref": document_set_ref,
        },
    )

    async def _fake_execute_step(step_name: str, state, scope=None):
        assert step_name == "map.conceptual.lift.surface_fragments"
        state.outputs["fragment_ids"] = ["frag-chunk-1", "frag-chunk-2"]
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-hot-runtime-chunk-set/control",
            json={
                "command_id": "cmd-hot-runtime-chunk-set-parse",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-hot-runtime-chunk-set",
            },
        )

        assert response.status_code == 200
        runtime_context = STORE.get_debug_runtime_context("run-hot-runtime-chunk-set")
        assert runtime_context is not None
        chunk_extraction_set_ref = runtime_context["chunk_extraction_set_ref"]
        assert chunk_extraction_set_ref.startswith("hot://run-hot-runtime-chunk-set/chunk_extraction_set/step-")
        resolved = STORE.get_hot_subgraph(chunk_extraction_set_ref)
        assert resolved == {
            "kind": "chunk_extraction_set",
            "source_subgraph_ref": "hot://run-hot-runtime-chunk-set/document_set/step-hot-runtime-chunk-set-load",
            "subgraph_ref": "",
            "extraction_refs": ["frag-chunk-1", "frag-chunk-2"],
        }
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_next_step_persists_runtime_hot_chunk_extraction_set_from_decomposition_structural(monkeypatch: pytest.MonkeyPatch) -> None:
    import os
    from types import SimpleNamespace

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-hot-runtime-chunk-structural",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-chunk-structural",
            project_id="proj-hot-runtime-chunk-structural",
            operation_id="op-hot-runtime-chunk-structural",
            env_type="dev",
            env_id="dev-hot-runtime-chunk-structural",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-hot-runtime-chunk-structural-load",
            run_id="run-hot-runtime-chunk-structural",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-chunk-structural",
            project_id="proj-hot-runtime-chunk-structural",
            operation_id="op-hot-runtime-chunk-structural",
            env_type="dev",
            env_id="dev-hot-runtime-chunk-structural",
            step_name="load.documents",
            step_id="step-hot-runtime-chunk-structural-load",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    document_set_ref = STORE.put_hot_subgraph(
        run_id="run-hot-runtime-chunk-structural",
        step_id="step-hot-runtime-chunk-structural-load",
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "artifact:hot-runtime-chunk-structural",
            "subgraph_ref": "hot://run-hot-runtime-chunk-structural/document_set/step-hot-runtime-chunk-structural-load",
            "document_refs": ["frag-hot-doc-1"],
        },
    )
    STORE.set_debug_runtime_context(
        "run-hot-runtime-chunk-structural",
        {
            "source_bytes": b"# hot chunk structural\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:hot-runtime-chunk-structural",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "document_set_ref": document_set_ref,
        },
    )

    async def _fake_execute_step(step_name: str, state, scope=None):
        state.outputs["decomposition"] = SimpleNamespace(
            structural=[
                SimpleNamespace(cas_id="frag-structural-1", id="frag-structural-1"),
                SimpleNamespace(cas_id="frag-structural-2", id="frag-structural-2"),
            ]
        )
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-hot-runtime-chunk-structural/control",
            json={
                "command_id": "cmd-hot-runtime-chunk-structural",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-hot-runtime-chunk-structural",
            },
        )

        assert response.status_code == 200
        runtime_context = STORE.get_debug_runtime_context("run-hot-runtime-chunk-structural")
        assert runtime_context is not None
        chunk_extraction_set_ref = runtime_context["chunk_extraction_set_ref"]
        resolved = STORE.get_hot_subgraph(chunk_extraction_set_ref)
        assert resolved == {
            "kind": "chunk_extraction_set",
            "source_subgraph_ref": "hot://run-hot-runtime-chunk-structural/document_set/step-hot-runtime-chunk-structural-load",
            "subgraph_ref": "",
            "extraction_refs": ["frag-structural-1", "frag-structural-2"],
        }
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_next_step_persists_runtime_hot_chunk_extraction_set_with_source_document_derivation(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-hot-runtime-chunk-derived-doc",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-chunk-derived-doc",
            project_id="proj-hot-runtime-chunk-derived-doc",
            operation_id="op-hot-runtime-chunk-derived-doc",
            env_type="dev",
            env_id="dev-hot-runtime-chunk-derived-doc",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-hot-runtime-chunk-derived-doc-load",
            run_id="run-hot-runtime-chunk-derived-doc",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-chunk-derived-doc",
            project_id="proj-hot-runtime-chunk-derived-doc",
            operation_id="op-hot-runtime-chunk-derived-doc",
            env_type="dev",
            env_id="dev-hot-runtime-chunk-derived-doc",
            step_name="load.documents",
            step_id="step-hot-runtime-chunk-derived-doc-load",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    document_set_ref = STORE.put_hot_subgraph(
        run_id="run-hot-runtime-chunk-derived-doc",
        step_id="step-hot-runtime-chunk-derived-doc-load",
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "artifact:hot-runtime-chunk-derived-doc",
            "subgraph_ref": "hot://run-hot-runtime-chunk-derived-doc/document_set/step-hot-runtime-chunk-derived-doc-load",
            "document_refs": ["frag-hot-doc-derived-1"],
            "documents": [
                {
                    "cas_id": "frag-hot-doc-derived-1",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {
                        "document_id": "doc-derived-1",
                        "artifact_id": "artifact:hot-runtime-chunk-derived-doc",
                        "filename": "derived.md",
                        "text": "alpha beta",
                    },
                }
            ],
        },
    )
    STORE.set_debug_runtime_context(
        "run-hot-runtime-chunk-derived-doc",
        {
            "source_bytes": b"# derived doc\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:hot-runtime-chunk-derived-doc",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {
                "documents": [
                    {
                        "id": "doc-derived-1",
                        "text": "alpha beta",
                        "artifact_id": "artifact:hot-runtime-chunk-derived-doc",
                        "filename": "derived.md",
                        "mime_type": "text/markdown",
                    }
                ],
                "document_fragment_refs": ["frag-hot-doc-derived-1"],
            },
            "document_set_ref": document_set_ref,
        },
    )

    async def _fake_execute_step(step_name: str, state, scope=None):
        state.outputs["chunks"] = [
            {
                "fragment_id": "frag-chunk-derived-1",
                "chunk_id": "doc-derived-1:chunk:0",
                "document_id": "doc-derived-1",
                "artifact_id": "artifact:hot-runtime-chunk-derived-doc",
                "filename": "derived.md",
                "text": "alpha beta",
                "span": {"start": 0, "end": 10},
                "order": 0,
            }
        ]
        state.outputs["fragment_ids"] = ["frag-chunk-derived-1"]
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-hot-runtime-chunk-derived-doc/control",
            json={
                "command_id": "cmd-hot-runtime-chunk-derived-doc-parse",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-hot-runtime-chunk-derived-doc",
            },
        )

        assert response.status_code == 200
        runtime_context = STORE.get_debug_runtime_context("run-hot-runtime-chunk-derived-doc")
        assert runtime_context is not None
        chunk_extraction_set_ref = runtime_context["chunk_extraction_set_ref"]
        resolved = STORE.get_hot_subgraph(chunk_extraction_set_ref)
        assert resolved["edges"] == [
            {
                "from": "fragment:frag-chunk-derived-1",
                "to": "fragment:frag-hot-doc-derived-1",
                "edge_label": "knowledge:derives",
            }
        ]
        assert resolved["extractions"][0]["value"]["source_document_fragment_id"] == "frag-hot-doc-derived-1"
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_parse_chunk_execution_preserves_document_chunk_sets_in_state_outputs(monkeypatch: pytest.MonkeyPatch) -> None:
    import asyncio

    from ikam.forja import debug_execution

    state = debug_execution.StepExecutionState(
        source_bytes=b"# grouped doc\n",
        mime_type="text/markdown",
        artifact_id="artifact:hot-runtime-chunk-document-groups",
        assets=[
            {
                "artifact_id": "artifact:hot-runtime-chunk-document-groups",
                "filename": "grouped.md",
                "mime_type": "text/markdown",
                "payload": b"alpha beta",
            }
        ],
        outputs={
            "inputs": {"document_set_ref": "hot://run-hot-runtime-chunk-document-groups/document_set/step-load"},
            "document_fragment_refs": ["frag-hot-doc-group-1"],
        },
    )

    monkeypatch.setattr(
        debug_execution,
        "_build_documents_for_chunking",
        lambda **kwargs: [
            {
                "id": "doc-group-1",
                "text": "alpha beta",
                "artifact_id": "artifact:hot-runtime-chunk-document-groups",
                "filename": "grouped.md",
                "mime_type": "text/markdown",
                "source_document_fragment_id": "frag-hot-doc-group-1",
            }
        ],
    )
    monkeypatch.setattr(debug_execution, "create_ai_client_from_env", lambda: None, raising=False)

    class _FakeChunkOperator:
        def apply(self, _graph, params, _env):
            assert params.parameters["documents"][0]["source_document_fragment_id"] == "frag-hot-doc-group-1"
            return {
                "chunks": [
                    {
                        "fragment_id": "frag-chunk-group-1",
                        "chunk_id": "doc-group-1:chunk:0",
                        "document_id": "doc-group-1",
                        "artifact_id": "artifact:hot-runtime-chunk-document-groups",
                        "filename": "grouped.md",
                        "text": "alpha beta",
                        "span": {"start": 0, "end": 10},
                        "order": 0,
                        "source_document_fragment_id": "frag-hot-doc-group-1",
                    }
                ],
                "fragment_ids": ["frag-chunk-group-1"],
                "fragment_artifact_map": {"frag-chunk-group-1": "artifact:hot-runtime-chunk-document-groups"},
                "document_stats": [
                    {
                        "document_id": "doc-group-1",
                        "source_document_fragment_id": "frag-hot-doc-group-1",
                        "artifact_id": "artifact:hot-runtime-chunk-document-groups",
                        "filename": "grouped.md",
                        "chunk_count": 1,
                        "char_count": 10,
                    }
                ],
                "document_chunk_sets": [
                    {
                        "cas_id": "frag-doc-chunk-group-1",
                        "mime_type": "application/vnd.ikam.document-chunk-set+json",
                        "value": {
                            "kind": "document_chunk_set",
                            "document_id": "doc-group-1",
                            "source_document_fragment_id": "frag-hot-doc-group-1",
                            "artifact_id": "artifact:hot-runtime-chunk-document-groups",
                            "filename": "grouped.md",
                            "chunk_refs": ["frag-chunk-group-1"],
                            "fragment_id": "frag-doc-chunk-group-1",
                        },
                    }
                ],
                "chunk_extraction_set": {
                    "kind": "chunk_extraction_set",
                    "source_subgraph_ref": "hot://run-hot-runtime-chunk-document-groups/document_set/step-load",
                    "subgraph_ref": "",
                    "extraction_refs": ["frag-chunk-group-1"],
                    "document_chunk_sets": [
                        {
                            "fragment_id": "frag-doc-chunk-group-1",
                            "kind": "document_chunk_set",
                            "document_id": "doc-group-1",
                            "source_document_fragment_id": "frag-hot-doc-group-1",
                            "artifact_id": "artifact:hot-runtime-chunk-document-groups",
                            "filename": "grouped.md",
                            "chunk_refs": ["frag-chunk-group-1"],
                        }
                    ],
                },
                "summary": {"document_count": 1, "chunk_count": 1, "artifact_count": 1},
            }

    monkeypatch.setattr("modelado.operators.chunking.ChunkOperator", _FakeChunkOperator)

    asyncio.run(debug_execution._execute_ingestion_chunking("parse.chunk", state, None, {}))

    assert state.outputs["document_chunk_sets"] == [
        {
            "cas_id": "frag-doc-chunk-group-1",
            "mime_type": "application/vnd.ikam.document-chunk-set+json",
            "value": {
                "kind": "document_chunk_set",
                "document_id": "doc-group-1",
                "source_document_fragment_id": "frag-hot-doc-group-1",
                "artifact_id": "artifact:hot-runtime-chunk-document-groups",
                "filename": "grouped.md",
                "chunk_refs": ["frag-chunk-group-1"],
                "fragment_id": "frag-doc-chunk-group-1",
            },
        }
    ]
    assert state.outputs["chunk_extraction_set"]["document_chunk_sets"] == [
        {
            "fragment_id": "frag-doc-chunk-group-1",
            "kind": "document_chunk_set",
            "document_id": "doc-group-1",
            "source_document_fragment_id": "frag-hot-doc-group-1",
            "artifact_id": "artifact:hot-runtime-chunk-document-groups",
            "filename": "grouped.md",
            "chunk_refs": ["frag-chunk-group-1"],
        }
    ]


def test_next_step_persists_runtime_hot_document_set_with_documents_for_chunk_inspection(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-hot-runtime-document-set-documents",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-document-set-documents",
            project_id="proj-hot-runtime-document-set-documents",
            operation_id="op-hot-runtime-document-set-documents",
            env_type="dev",
            env_id="dev-hot-runtime-document-set-documents",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-hot-runtime-document-set-documents-init",
            run_id="run-hot-runtime-document-set-documents",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-document-set-documents",
            project_id="proj-hot-runtime-document-set-documents",
            operation_id="op-hot-runtime-document-set-documents",
            env_type="dev",
            env_id="dev-hot-runtime-document-set-documents",
            step_name="init.initialize",
            step_id="step-hot-runtime-document-set-documents-init",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    STORE.set_debug_runtime_context(
        "run-hot-runtime-document-set-documents",
        {
            "source_bytes": b"# source doc\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:hot-runtime-document-set-documents",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
        },
    )

    async def _fake_execute_step(step_name: str, state, scope=None):
        state.outputs["documents"] = [
            {
                "id": "doc-persisted-1",
                "text": "alpha beta",
                "artifact_id": "artifact:hot-runtime-document-set-documents",
                "filename": "persisted.md",
                "mime_type": "text/markdown",
                "source_document_fragment_id": "frag-hot-doc-persisted-1",
            }
        ]
        state.outputs["document_fragment_refs"] = ["frag-hot-doc-persisted-1"]
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-hot-runtime-document-set-documents/control",
            json={
                "command_id": "cmd-hot-runtime-document-set-documents-load",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-hot-runtime-document-set-documents",
            },
        )

        assert response.status_code == 200
        runtime_context = STORE.get_debug_runtime_context("run-hot-runtime-document-set-documents")
        assert runtime_context is not None
        document_set_ref = runtime_context["document_set_ref"]
        resolved = STORE.get_hot_subgraph(document_set_ref)
        assert resolved["document_refs"] == ["frag-hot-doc-persisted-1"]
        assert resolved["documents"] == [
            {
                "cas_id": "frag-hot-doc-persisted-1",
                "mime_type": "application/vnd.ikam.loaded-document+json",
                "value": {
                    "id": "doc-persisted-1",
                    "text": "alpha beta",
                    "artifact_id": "artifact:hot-runtime-document-set-documents",
                    "filename": "persisted.md",
                    "mime_type": "text/markdown",
                    "source_document_fragment_id": "frag-hot-doc-persisted-1",
                },
            }
        ]
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_next_step_hydrates_entities_inputs_from_runtime_hot_chunk_extraction_set(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-hot-runtime-entities-handoff",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-entities-handoff",
            project_id="proj-hot-runtime-entities-handoff",
            operation_id="op-hot-runtime-entities-handoff",
            env_type="dev",
            env_id="dev-hot-runtime-entities-handoff",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.chunk",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-hot-runtime-entities-handoff-parse",
            run_id="run-hot-runtime-entities-handoff",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-entities-handoff",
            project_id="proj-hot-runtime-entities-handoff",
            operation_id="op-hot-runtime-entities-handoff",
            env_type="dev",
            env_id="dev-hot-runtime-entities-handoff",
            step_name="parse.chunk",
            step_id="step-hot-runtime-entities-handoff-parse",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    chunk_extraction_set_ref = STORE.put_hot_subgraph(
        run_id="run-hot-runtime-entities-handoff",
        step_id="step-hot-runtime-entities-handoff-parse",
        contract_type="chunk_extraction_set",
        payload={
            "kind": "chunk_extraction_set",
            "source_subgraph_ref": "hot://run-hot-runtime-entities-handoff/document_set/step-load",
            "subgraph_ref": "hot://run-hot-runtime-entities-handoff/chunk_extraction_set/step-hot-runtime-entities-handoff-parse",
            "extraction_refs": ["frag-chunk-a", "frag-chunk-b"],
        },
    )
    STORE.set_debug_runtime_context(
        "run-hot-runtime-entities-handoff",
        {
            "source_bytes": b"# entities handoff\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:hot-runtime-entities-handoff",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "chunk_extraction_set_ref": chunk_extraction_set_ref,
        },
    )

    captured: dict[str, object] = {}

    async def _fake_execute_step(step_name: str, state, scope=None):
        captured["step_name"] = step_name
        captured["fragment_ids"] = list(state.outputs.get("fragment_ids") or [])
        captured["inputs"] = dict(state.outputs.get("inputs") or {})
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-hot-runtime-entities-handoff/control",
            json={
                "command_id": "cmd-hot-runtime-entities-handoff-next",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-hot-runtime-entities-handoff",
            },
        )

        assert response.status_code == 200
        assert captured["step_name"] == "map.conceptual.lift.entities_and_relationships"
        assert captured["fragment_ids"] == ["frag-chunk-a", "frag-chunk-b"]
        assert captured["inputs"] == {
            "chunk_extraction_set_ref": "hot://run-hot-runtime-entities-handoff/chunk_extraction_set/step-hot-runtime-entities-handoff-parse",
            "fragment_ids": ["frag-chunk-a", "frag-chunk-b"],
        }
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_next_step_hydrates_claims_inputs_from_runtime_hot_entity_relationship_set(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-hot-runtime-claims-handoff",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-claims-handoff",
            project_id="proj-hot-runtime-claims-handoff",
            operation_id="op-hot-runtime-claims-handoff",
            env_type="dev",
            env_id="dev-hot-runtime-claims-handoff",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.entities_and_relationships",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-hot-runtime-claims-handoff-entities",
            run_id="run-hot-runtime-claims-handoff",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-claims-handoff",
            project_id="proj-hot-runtime-claims-handoff",
            operation_id="op-hot-runtime-claims-handoff",
            env_type="dev",
            env_id="dev-hot-runtime-claims-handoff",
            step_name="parse.entities_and_relationships",
            step_id="step-hot-runtime-claims-handoff-entities",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    entity_relationship_set_ref = STORE.put_hot_subgraph(
        run_id="run-hot-runtime-claims-handoff",
        step_id="step-hot-runtime-claims-handoff-entities",
        contract_type="entity_relationship_set",
        payload={
            "kind": "entity_relationship_set",
            "source_subgraph_ref": "hot://run-hot-runtime-claims-handoff/chunk_extraction_set/step-hot-runtime-claims-handoff-parse",
            "subgraph_ref": "hot://run-hot-runtime-claims-handoff/entity_relationship_set/step-hot-runtime-claims-handoff-entities",
            "entity_relationship_refs": ["frag-claim-a", "frag-claim-b"],
        },
    )
    STORE.set_debug_runtime_context(
        "run-hot-runtime-claims-handoff",
        {
            "source_bytes": b"# claims handoff\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:hot-runtime-claims-handoff",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "entity_relationship_set_ref": entity_relationship_set_ref,
        },
    )

    captured: dict[str, object] = {}

    async def _fake_execute_step(step_name: str, state, scope=None):
        captured["step_name"] = step_name
        captured["fragment_ids"] = list(state.outputs.get("fragment_ids") or [])
        captured["inputs"] = dict(state.outputs.get("inputs") or {})
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-hot-runtime-claims-handoff/control",
            json={
                "command_id": "cmd-hot-runtime-claims-handoff-next",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-hot-runtime-claims-handoff",
            },
        )

        assert response.status_code == 200
        assert captured["step_name"] == "map.conceptual.lift.claims"
        assert captured["fragment_ids"] == ["frag-claim-a", "frag-claim-b"]
        assert captured["inputs"] == {
            "entity_relationship_set_ref": "hot://run-hot-runtime-claims-handoff/entity_relationship_set/step-hot-runtime-claims-handoff-entities",
            "fragment_ids": ["frag-claim-a", "frag-claim-b"],
        }
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_parse_entities_fails_output_validation_and_does_not_publish_entity_relationship_handoff(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-parse-entities-output-gate",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-parse-entities-output-gate",
            project_id="proj-parse-entities-output-gate",
            operation_id="op-parse-entities-output-gate",
            env_type="dev",
            env_id="dev-parse-entities-output-gate",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.chunk",
            current_attempt_index=1,
        )
    )
    chunk_extraction_set_ref = STORE.put_hot_subgraph(
        run_id="run-parse-entities-output-gate",
        step_id="step-parse-entities-output-gate-parse",
        contract_type="chunk_extraction_set",
        payload={
            "kind": "chunk_extraction_set",
            "source_subgraph_ref": "hot://run-parse-entities-output-gate/document_set/step-load",
            "subgraph_ref": "hot://run-parse-entities-output-gate/chunk_extraction_set/step-parse",
            "extraction_refs": ["frag-entities-gate-1"],
        },
    )
    STORE.set_debug_runtime_context(
        "run-parse-entities-output-gate",
        {
            "source_bytes": b"# entities output gate\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:parse-entities-output-gate",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "chunk_extraction_set_ref": chunk_extraction_set_ref,
        },
    )

    async def _fake_execute_step(step_name: str, state, scope=None):
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-parse-entities-output-gate/control",
            json={
                "command_id": "cmd-parse-entities-output-gate",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-parse-entities-output-gate",
            },
        )

        assert response.status_code == 200
        stream = client.get(
            "/benchmarks/runs/run-parse-entities-output-gate/debug-stream",
            params={"pipeline_id": "ingestion-early-parse", "pipeline_run_id": "pipe-parse-entities-output-gate"},
        )
        assert stream.status_code == 200
        event = next(item for item in stream.json()["events"] if item["step_name"] == "parse.entities_and_relationships")
        assert event["status"] == "failed"
        runtime_context = STORE.get_debug_runtime_context("run-parse-entities-output-gate")
        assert runtime_context is not None
        assert "entity_relationship_set_ref" not in runtime_context
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_next_step_persists_runtime_hot_entity_relationship_set_after_parse_entities() -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-hot-runtime-entity-relationship-set",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-entity-relationship-set",
            project_id="proj-hot-runtime-entity-relationship-set",
            operation_id="op-hot-runtime-entity-relationship-set",
            env_type="dev",
            env_id="dev-hot-runtime-entity-relationship-set",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.chunk",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-hot-runtime-entity-relationship-set-parse",
            run_id="run-hot-runtime-entity-relationship-set",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-entity-relationship-set",
            project_id="proj-hot-runtime-entity-relationship-set",
            operation_id="op-hot-runtime-entity-relationship-set",
            env_type="dev",
            env_id="dev-hot-runtime-entity-relationship-set",
            step_name="parse.chunk",
            step_id="step-hot-runtime-entity-relationship-set-parse",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    chunk_extraction_set_ref = STORE.put_hot_subgraph(
        run_id="run-hot-runtime-entity-relationship-set",
        step_id="step-hot-runtime-entity-relationship-set-parse",
        contract_type="chunk_extraction_set",
        payload={
            "kind": "chunk_extraction_set",
            "source_subgraph_ref": "hot://run-hot-runtime-entity-relationship-set/document_set/step-load",
            "subgraph_ref": "hot://run-hot-runtime-entity-relationship-set/chunk_extraction_set/step-hot-runtime-entity-relationship-set-parse",
            "extraction_refs": ["frag-hot-chunk-1"],
            "chunk_extractions": [
                {
                    "cas_id": "frag-hot-chunk-1",
                    "mime_type": "application/vnd.ikam.chunk+json",
                    "value": {
                        "chunk_id": "doc-1:chunk:0",
                        "document_id": "doc-1",
                        "artifact_id": "artifact://hot-runtime-entity-relationship-set/doc-1",
                        "filename": "doc-1.md",
                        "text": "Alice founded Acme in Bogota.",
                        "span": {"start": 0, "end": 30},
                        "order": 0,
                    },
                }
            ],
        },
    )
    STORE.set_debug_runtime_context(
        "run-hot-runtime-entity-relationship-set",
        {
            "source_bytes": b"# entity relationship set\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:hot-runtime-entity-relationship-set",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "chunk_extraction_set_ref": chunk_extraction_set_ref,
        },
    )

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-hot-runtime-entity-relationship-set/control",
            json={
                "command_id": "cmd-hot-runtime-entity-relationship-set-next",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-hot-runtime-entity-relationship-set",
            },
        )

        assert response.status_code == 200
        stream = client.get(
            "/benchmarks/runs/run-hot-runtime-entity-relationship-set/debug-stream",
            params={
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-hot-runtime-entity-relationship-set",
            },
        )
        assert stream.status_code == 200
        event = next(item for item in stream.json()["events"] if item["step_name"] == "parse.entities_and_relationships")
        assert event["status"] == "succeeded"

        runtime_context = STORE.get_debug_runtime_context("run-hot-runtime-entity-relationship-set")
        assert runtime_context is not None
        entity_relationship_set_ref = runtime_context["entity_relationship_set_ref"]
        assert entity_relationship_set_ref.startswith("hot://run-hot-runtime-entity-relationship-set/entity_relationship_set/step-")
        resolved = STORE.get_hot_subgraph(entity_relationship_set_ref)
        assert resolved == {
            "kind": "entity_relationship_set",
            "source_subgraph_ref": "hot://run-hot-runtime-entity-relationship-set/chunk_extraction_set/step-hot-runtime-entity-relationship-set-parse",
            "subgraph_ref": "",
            "entity_relationship_refs": resolved["entity_relationship_refs"],
        }
        assert len(resolved["entity_relationship_refs"]) == 1

        detail = client.get(
            f"/benchmarks/runs/run-hot-runtime-entity-relationship-set/debug-step/{event['step_id']}/detail"
        )
        assert detail.status_code == 200
        body = detail.json()
        assert body["outputs"]["entity_relationship_set"]["kind"] == "entity_relationship_set"
        assert body["outputs"]["entity_relationship_set"]["entity_relationship_refs"] == resolved["entity_relationship_refs"]
        assert body["outputs"]["summary"] == {
            "chunk_count": 1,
            "entity_relationship_fragment_count": 1,
            "entity_count": 3,
            "relationship_count": 3,
        }
        assert len(body["outputs"]["entity_relationships"]) == 1
        assert body["outputs"]["entity_relationships"][0]["chunk_fragment_id"] == "frag-hot-chunk-1"
        assert body["transition_validation"]["resolved_outputs"]["entity_relationship_set"][0]["value"]["kind"] == "entity_relationship_set"
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_parse_claims_fails_output_validation_and_does_not_publish_claim_handoff(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-parse-claims-output-gate",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-parse-claims-output-gate",
            project_id="proj-parse-claims-output-gate",
            operation_id="op-parse-claims-output-gate",
            env_type="dev",
            env_id="dev-parse-claims-output-gate",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.entities_and_relationships",
            current_attempt_index=1,
        )
    )
    entity_relationship_set_ref = STORE.put_hot_subgraph(
        run_id="run-parse-claims-output-gate",
        step_id="step-parse-claims-output-gate-entities",
        contract_type="entity_relationship_set",
        payload={
            "kind": "entity_relationship_set",
            "source_subgraph_ref": "hot://run-parse-claims-output-gate/chunk_extraction_set/step-parse",
            "subgraph_ref": "hot://run-parse-claims-output-gate/entity_relationship_set/step-entities",
            "entity_relationship_refs": ["frag-claims-gate-1"],
        },
    )
    STORE.set_debug_runtime_context(
        "run-parse-claims-output-gate",
        {
            "source_bytes": b"# claims output gate\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:parse-claims-output-gate",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "entity_relationship_set_ref": entity_relationship_set_ref,
        },
    )

    async def _fake_execute_step(step_name: str, state, scope=None):
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-parse-claims-output-gate/control",
            json={
                "command_id": "cmd-parse-claims-output-gate",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-parse-claims-output-gate",
            },
        )

        assert response.status_code == 200
        stream = client.get(
            "/benchmarks/runs/run-parse-claims-output-gate/debug-stream",
            params={"pipeline_id": "ingestion-early-parse", "pipeline_run_id": "pipe-parse-claims-output-gate"},
        )
        assert stream.status_code == 200
        event = next(item for item in stream.json()["events"] if item["step_name"] == "parse.claims")
        assert event["status"] == "failed"
        runtime_context = STORE.get_debug_runtime_context("run-parse-claims-output-gate")
        assert runtime_context is not None
        assert "claim_set_ref" not in runtime_context
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_next_step_persists_runtime_hot_claim_set_after_parse_claims(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-hot-runtime-claim-set",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-claim-set",
            project_id="proj-hot-runtime-claim-set",
            operation_id="op-hot-runtime-claim-set",
            env_type="dev",
            env_id="dev-hot-runtime-claim-set",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.entities_and_relationships",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-hot-runtime-claim-set-entities",
            run_id="run-hot-runtime-claim-set",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-hot-runtime-claim-set",
            project_id="proj-hot-runtime-claim-set",
            operation_id="op-hot-runtime-claim-set",
            env_type="dev",
            env_id="dev-hot-runtime-claim-set",
            step_name="parse.entities_and_relationships",
            step_id="step-hot-runtime-claim-set-entities",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-26T00:00:00Z",
            ended_at="2026-03-26T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    entity_relationship_set_ref = STORE.put_hot_subgraph(
        run_id="run-hot-runtime-claim-set",
        step_id="step-hot-runtime-claim-set-entities",
        contract_type="entity_relationship_set",
        payload={
            "kind": "entity_relationship_set",
            "source_subgraph_ref": "hot://run-hot-runtime-claim-set/chunk_extraction_set/step-parse-chunk",
            "subgraph_ref": "hot://run-hot-runtime-claim-set/entity_relationship_set/step-hot-runtime-claim-set-entities",
            "entity_relationship_refs": ["frag-claim-ir-1", "frag-claim-ir-2"],
        },
    )
    STORE.set_debug_runtime_context(
        "run-hot-runtime-claim-set",
        {
            "source_bytes": b"# hot claim set\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:hot-runtime-claim-set",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "entity_relationship_set_ref": entity_relationship_set_ref,
        },
    )

    async def _fake_execute_step(step_name: str, state, scope=None):
        assert step_name == "map.conceptual.lift.claims"
        state.outputs["fragment_ids"] = ["frag-claim-out-1", "frag-claim-out-2"]
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            "/benchmarks/runs/run-hot-runtime-claim-set/control",
            json={
                "command_id": "cmd-hot-runtime-claim-set-next",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-hot-runtime-claim-set",
            },
        )

        assert response.status_code == 200
        runtime_context = STORE.get_debug_runtime_context("run-hot-runtime-claim-set")
        assert runtime_context is not None
        claim_set_ref = runtime_context["claim_set_ref"]
        assert claim_set_ref.startswith("hot://run-hot-runtime-claim-set/claim_set/step-")
        resolved = STORE.get_hot_subgraph(claim_set_ref)
        assert resolved == {
            "kind": "claim_set",
            "source_subgraph_ref": "hot://run-hot-runtime-claim-set/entity_relationship_set/step-hot-runtime-claim-set-entities",
            "subgraph_ref": "",
            "claim_refs": ["frag-claim-out-1", "frag-claim-out-2"],
        }
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_next_step_parse_claims_uses_operator_backed_claim_outputs() -> None:
    import os

    run_id = "run-operator-backed-claim-set"
    pipeline_run_id = "pipe-operator-backed-claim-set"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id=run_id,
            project_id="proj-operator-backed-claim-set",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-operator-backed-claim-set", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id="ingestion-early-parse",
            pipeline_run_id=pipeline_run_id,
            project_id="proj-operator-backed-claim-set",
            operation_id="op-operator-backed-claim-set",
            env_type="dev",
            env_id="dev-operator-backed-claim-set",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.entities_and_relationships",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-operator-backed-claim-set-entities",
            run_id=run_id,
            pipeline_id="ingestion-early-parse",
            pipeline_run_id=pipeline_run_id,
            project_id="proj-operator-backed-claim-set",
            operation_id="op-operator-backed-claim-set",
            env_type="dev",
            env_id="dev-operator-backed-claim-set",
            step_name="parse.entities_and_relationships",
            step_id="step-operator-backed-claim-set-entities",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-27T00:00:00Z",
            ended_at="2026-03-27T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    entity_relationship_set_ref = STORE.put_hot_subgraph(
        run_id=run_id,
        step_id="step-operator-backed-claim-set-entities",
        contract_type="entity_relationship_set",
        payload={
            "kind": "entity_relationship_set",
            "source_subgraph_ref": f"hot://{run_id}/chunk_extraction_set/step-parse-chunk",
            "subgraph_ref": f"hot://{run_id}/entity_relationship_set/step-operator-backed-claim-set-entities",
            "entity_relationship_refs": ["frag-ir-operator-1"],
            "entity_relationships": [
                {
                    "cas_id": "frag-ir-operator-1",
                    "mime_type": "application/vnd.ikam.entity-relationship+json",
                    "value": {
                        "fragment_id": "frag-ir-row-operator-1",
                        "chunk_fragment_id": "frag-chunk-operator-1",
                        "chunk_id": "doc-1:chunk:0",
                        "document_id": "doc-1",
                        "artifact_id": "artifact://operator-backed-claim-set/doc-1",
                        "filename": "doc-1.md",
                        "span": {"start": 0, "end": 30},
                        "order": 0,
                        "text": "Alice founded Acme in Bogota.",
                        "entities": [
                            {"name": "Alice", "type": "candidate_entity"},
                            {"name": "Acme", "type": "candidate_entity"},
                            {"name": "Bogota", "type": "candidate_entity"},
                        ],
                        "relationships": [
                            {"source": "Alice", "target": "Acme", "relationship": "co_occurs_in_chunk"},
                            {"source": "Acme", "target": "Bogota", "relationship": "co_occurs_in_chunk"},
                        ],
                    },
                }
            ],
        },
    )
    STORE.set_debug_runtime_context(
        run_id,
        {
            "source_bytes": b"# operator backed claim set\n",
            "mime_type": "text/markdown",
            "artifact_id": "artifact:operator-backed-claim-set",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "entity_relationship_set_ref": entity_relationship_set_ref,
        },
    )

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        response = client.post(
            f"/benchmarks/runs/{run_id}/control",
            json={
                "command_id": "cmd-operator-backed-claim-set-next",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": pipeline_run_id,
            },
        )

        assert response.status_code == 200
        runtime_context = STORE.get_debug_runtime_context(run_id)
        assert runtime_context is not None
        claim_set_ref = runtime_context["claim_set_ref"]
        resolved = STORE.get_hot_subgraph(claim_set_ref)
        assert resolved["kind"] == "claim_set"
        assert resolved["source_subgraph_ref"] == f"hot://{run_id}/entity_relationship_set/step-operator-backed-claim-set-entities"
        assert len(resolved["claim_refs"]) == 2

        stream = client.get(
            f"/benchmarks/runs/{run_id}/debug-stream",
            params={"pipeline_id": "ingestion-early-parse", "pipeline_run_id": pipeline_run_id},
        )
        assert stream.status_code == 200
        event = next(item for item in stream.json()["events"] if item["step_name"] == "parse.claims")
        assert event["status"] == "succeeded"

        detail = client.get(f"/benchmarks/runs/{run_id}/debug-step/{event['step_id']}/detail")
        assert detail.status_code == 200
        body = detail.json()
        assert body["outputs"]["claim_set"]["kind"] == "claim_set"
        assert body["outputs"]["claim_set"]["claim_refs"] == resolved["claim_refs"]
        assert body["outputs"]["summary"] == {
            "entity_relationship_count": 1,
            "claim_fragment_count": 2,
            "claim_count": 2,
        }
        assert [item["claim"] for item in body["outputs"]["claims"]] == [
            "Alice co_occurs_in_chunk Acme",
            "Acme co_occurs_in_chunk Bogota",
        ]

        env_fragments = client.get(
            f"/benchmarks/runs/{run_id}/env/fragments",
            params={
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": pipeline_run_id,
                "ref": "refs/heads/run/dev-operator-backed-claim-set",
                "step_id": event["step_id"],
                "attempt_index": 1,
            },
        )
        assert env_fragments.status_code == 200
        fragment_ids = [item["id"] for item in env_fragments.json()["fragments"]]
        assert fragment_ids == resolved["claim_refs"]
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_debug_step_detail_prefers_ref_native_promotion_payload() -> None:
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-commit-detail",
            project_id="proj-commit-detail",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-commit-detail", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-commit-detail",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-commit-detail",
            project_id="proj-commit-detail",
            operation_id="op-commit-detail",
            env_type="dev",
            env_id="dev-commit-detail",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="map.conceptual.commit.semantic_only",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-commit-detail",
            run_id="run-commit-detail",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-commit-detail",
            project_id="proj-commit-detail",
            operation_id="op-commit-detail",
            env_type="dev",
            env_id="dev-commit-detail",
            step_name="map.conceptual.commit.semantic_only",
            step_id="step-commit-detail",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:01Z",
            duration_ms=1,
            metrics={},
        )
    )
    STORE.set_debug_runtime_context(
        "run-commit-detail",
        {
            "step_outputs": {
                "commit": {
                    "mode": "normalized",
                    "target_ref": "refs/heads/main",
                    "promoted_fragment_ids": ["frag-a", "frag-b"],
                    "promoted_program_ids": ["prog-a"],
                }
            }
        },
    )

    client = TestClient(app)
    response = client.get("/benchmarks/runs/run-commit-detail/debug-step/step-commit-detail/detail")

    assert response.status_code == 200
    body = response.json()
    assert body["outputs"]["commit"] == {
        "mode": "normalized",
        "target_ref": "refs/heads/main",
        "promoted_fragment_ids": ["frag-a", "frag-b"],
        "promoted_program_ids": ["prog-a"],
    }
    assert "committed" not in body["why"]["summary"].lower()
    assert "target ref" in body["why"]["summary"].lower()


def test_step_detail_prefers_runtime_trace_logs_for_executor_output() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-step-trace-hydrated",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-trace-hydrated",
            project_id="proj-step-trace-hydrated",
            operation_id="op-step-trace-hydrated",
            env_type="dev",
            env_id="dev-step-trace-hydrated",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.chunk",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-step-trace-hydrated",
            run_id="run-step-trace-hydrated",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-step-trace-hydrated",
            project_id="proj-step-trace-hydrated",
            operation_id="op-step-trace-hydrated",
            env_type="dev",
            env_id="dev-step-trace-hydrated",
            step_name="parse.chunk",
            step_id="step-step-trace-hydrated",
            status="running",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at=None,
            duration_ms=None,
            metrics={
                "system_logs": {
                    "stdout_lines": [
                        "executing parse.chunk operation",
                        "[parse.chunk] still running",
                    ],
                    "stderr_lines": [],
                },
                "logs": {
                    "stdout_lines": [
                        "executing parse.chunk operation",
                        "[parse.chunk] still running",
                    ],
                    "stderr_lines": [],
                },
                "trace": {
                    "timeline": [
                        {
                            "topic": "execution.progress",
                            "event_type": "execution.running",
                            "status": "running",
                            "occurred_at": "2026-03-01T00:00:00Z",
                            "payload": {
                                "stdout_lines": ["real runtime line 1"],
                                "stderr_lines": [],
                            },
                        }
                    ],
                    "raw_events": [
                        {
                            "topic": "execution.progress",
                            "payload": {
                                "stdout_lines": ["real runtime line 1"],
                                "stderr_lines": [],
                            },
                        }
                    ],
                },
            },
            error=None,
        )
    )

    client = TestClient(app)
    response = client.get("/benchmarks/runs/run-step-trace-hydrated/debug-step/step-step-trace-hydrated/detail")

    assert response.status_code == 200
    body = response.json()
    assert body["executor_logs"] == {
        "stdout_lines": ["real runtime line 1"],
        "stderr_lines": [],
    }
    assert body["system_logs"] == {
        "stdout_lines": [
            "executing parse.chunk operation",
            "[parse.chunk] still running",
        ],
        "stderr_lines": [],
    }


def test_step_detail_derives_all_log_views_from_log_events() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-log-events-authoritative",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-log-events-authoritative",
            project_id="proj-log-events-authoritative",
            operation_id="op-log-events-authoritative",
            env_type="dev",
            env_id="dev-log-events-authoritative",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.chunk",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-log-events-authoritative",
            run_id="run-log-events-authoritative",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-log-events-authoritative",
            project_id="proj-log-events-authoritative",
            operation_id="op-log-events-authoritative",
            env_type="dev",
            env_id="dev-log-events-authoritative",
            step_name="parse.chunk",
            step_id="step-log-events-authoritative",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:02Z",
            duration_ms=2000,
            metrics={
                "executor_logs": {
                    "stdout_lines": ["stale executor stdout"],
                    "stderr_lines": ["stale executor stderr"],
                },
                "system_logs": {
                    "stdout_lines": ["stale system stdout"],
                    "stderr_lines": ["stale system stderr"],
                },
                "logs": {
                    "stdout_lines": ["stale merged stdout"],
                    "stderr_lines": ["stale merged stderr"],
                },
                "log_events": [
                    {
                        "seq": 1,
                        "at": "2026-03-01T00:00:00Z",
                        "source": "system",
                        "stream": "stdout",
                        "message": "executing parse.chunk operation",
                    },
                    {
                        "seq": 2,
                        "at": "2026-03-01T00:00:01Z",
                        "source": "executor",
                        "stream": "stdout",
                        "message": "real executor stdout",
                    },
                    {
                        "seq": 3,
                        "at": "2026-03-01T00:00:02Z",
                        "source": "executor",
                        "stream": "stderr",
                        "message": "real executor stderr",
                    },
                ],
            },
            error=None,
        )
    )

    client = TestClient(app)
    response = client.get(
        "/benchmarks/runs/run-log-events-authoritative/debug-step/step-log-events-authoritative/detail"
    )

    assert response.status_code == 200
    body = response.json()
    assert body["log_events"] == [
        {
            "seq": 1,
            "at": "2026-03-01T00:00:00Z",
            "source": "system",
            "stream": "stdout",
            "message": "executing parse.chunk operation",
        },
        {
            "seq": 2,
            "at": "2026-03-01T00:00:01Z",
            "source": "executor",
            "stream": "stdout",
            "message": "real executor stdout",
        },
        {
            "seq": 3,
            "at": "2026-03-01T00:00:02Z",
            "source": "executor",
            "stream": "stderr",
            "message": "real executor stderr",
        },
    ]
    assert body["executor_logs"] == {
        "stdout_lines": ["real executor stdout"],
        "stderr_lines": ["real executor stderr"],
    }
    assert body["system_logs"] == {
        "stdout_lines": ["executing parse.chunk operation"],
        "stderr_lines": [],
    }
    assert body["logs"] == {
        "stdout_lines": ["executing parse.chunk operation", "real executor stdout"],
        "stderr_lines": ["real executor stderr"],
    }


def test_step_detail_preserves_duplicate_executor_stdout_from_log_events() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-log-events-duplicates",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-log-events-duplicates",
            project_id="proj-log-events-duplicates",
            operation_id="op-log-events-duplicates",
            env_type="dev",
            env_id="dev-log-events-duplicates",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="parse.chunk",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-log-events-duplicates",
            run_id="run-log-events-duplicates",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-log-events-duplicates",
            project_id="proj-log-events-duplicates",
            operation_id="op-log-events-duplicates",
            env_type="dev",
            env_id="dev-log-events-duplicates",
            step_name="parse.chunk",
            step_id="step-log-events-duplicates",
            status="succeeded",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-01T00:00:00Z",
            ended_at="2026-03-01T00:00:02Z",
            duration_ms=2000,
            metrics={
                "log_events": [
                    {
                        "seq": 1,
                        "at": "2026-03-01T00:00:00Z",
                        "source": "system",
                        "stream": "stdout",
                        "message": "executing parse.chunk operation",
                    },
                    {
                        "seq": 2,
                        "at": "2026-03-01T00:00:01Z",
                        "source": "executor",
                        "stream": "stdout",
                        "message": "duplicate executor stdout",
                    },
                    {
                        "seq": 3,
                        "at": "2026-03-01T00:00:01Z",
                        "source": "executor",
                        "stream": "stdout",
                        "message": "duplicate executor stdout",
                    },
                ],
                "trace": {
                    "timeline": [
                        {
                            "topic": "execution.progress",
                            "event_type": "execution.running",
                            "status": "running",
                            "occurred_at": "2026-03-01T00:00:01Z",
                            "payload": {
                                "stdout_lines": ["duplicate executor stdout"],
                                "stderr_lines": [],
                            },
                        }
                    ],
                    "raw_events": [
                        {
                            "topic": "execution.progress",
                            "payload": {
                                "stdout_lines": ["duplicate executor stdout"],
                                "stderr_lines": [],
                            },
                        }
                    ],
                },
            },
            error=None,
        )
    )

    client = TestClient(app)
    response = client.get(
        "/benchmarks/runs/run-log-events-duplicates/debug-step/step-log-events-duplicates/detail"
    )

    assert response.status_code == 200
    body = response.json()
    expected_executor_stdout = [
        item["message"]
        for item in body["log_events"]
        if item["source"] == "executor" and item["stream"] == "stdout"
    ]
    assert expected_executor_stdout == [
        "duplicate executor stdout",
        "duplicate executor stdout",
    ]
    assert body["executor_logs"]["stdout_lines"] == expected_executor_stdout


def test_control_next_step_captures_stdout_and_stderr_logs(monkeypatch: pytest.MonkeyPatch) -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-log-capture",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-log-capture",
            project_id="proj-log-capture",
            operation_id="op-log-capture",
            env_type="dev",
            env_id="dev-log-capture",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-log-capture",
        {
            "source_bytes": b"# log capture\ntext",
            "mime_type": "text/markdown",
            "artifact_id": "proj-log-capture:run-log-capture",
            "step_outputs": {},
        },
    )

    monkeypatch.setenv("IKAM_ASYNC_NEXT_STEP", "0")
    client = TestClient(app)
    next_resp = client.post(
        "/benchmarks/runs/run-log-capture/control",
        json={
            "command_id": "cmd-log-capture-1",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-log-capture",
        },
    )
    assert next_resp.status_code == 200

    stream_resp = client.get(
        "/benchmarks/runs/run-log-capture/debug-stream",
        params={
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-log-capture",
        },
    )
    assert stream_resp.status_code == 200
    event = stream_resp.json()["events"][0]

    detail_resp = client.get(f"/benchmarks/runs/run-log-capture/debug-step/{event['step_id']}/detail")
    assert detail_resp.status_code == 200
    body = detail_resp.json()
    assert body["logs"]["stdout_lines"] != []
    assert body["logs"]["stderr_lines"] == []


def test_step_detail_uses_event_output_snapshot_instead_of_latest_runtime_outputs(monkeypatch: pytest.MonkeyPatch) -> None:
    import os

    from ikam_perf_report.api import benchmarks as benchmarks_api

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-step-output-snapshots",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-step-output-snapshots",
            project_id="proj-step-output-snapshots",
            operation_id="op-step-output-snapshots",
            env_type="dev",
            env_id="dev-step-output-snapshots",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=0,
        )
    )
    STORE.set_debug_runtime_context(
        "run-step-output-snapshots",
        {
            "source_bytes": b"# Title\n\nParagraph one.",
            "mime_type": "text/markdown",
            "artifact_id": "proj-step-output-snapshots:brief.md",
            "asset_manifest": [
                {
                    "artifact_id": "proj-step-output-snapshots:brief.md",
                    "filename": "brief.md",
                    "mime_type": "text/markdown",
                    "payload": b"# Title\n\nParagraph one.",
                }
            ],
            "asset_payloads": [
                {
                    "artifact_id": "proj-step-output-snapshots:brief.md",
                    "filename": "brief.md",
                    "mime_type": "text/markdown",
                    "payload": b"# Title\n\nParagraph one.",
                }
            ],
            "step_outputs": {},
        },
    )

    async def _fake_execute_step(step_name: str, state, scope=None):
        if step_name == "parse_artifacts":
            state.outputs["documents"] = [
                {
                    "id": "doc-1",
                    "text": "Paragraph one.",
                    "artifact_id": "proj-step-output-snapshots:brief.md",
                    "filename": "brief.md",
                    "mime_type": "text/markdown",
                    "reader_key": "simple_directory_reader",
                    "reader_method": "SimpleDirectoryReader.load_data",
                    "metadata": {"file_name": "brief.md"},
                }
            ]
            state.outputs["document_loads"] = [
                {
                    "artifact_id": "proj-step-output-snapshots:brief.md",
                    "filename": "brief.md",
                    "mime_type": "text/markdown",
                    "reader_key": "simple_directory_reader",
                    "reader_library": "llama_index.core",
                    "reader_method": "SimpleDirectoryReader.load_data",
                    "status": "success",
                    "document_count": 1,
                }
            ]
            state.outputs["operation_telemetry"] = {"operation_name": "load.documents"}
            return {"executor": "test", "status": "ok"}
        state.outputs["documents"] = []
        state.outputs["document_loads"] = [
            {
                "artifact_id": "proj-step-output-snapshots:future.docx",
                "filename": "future.docx",
                "mime_type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "reader_key": "simple_directory_reader",
                "reader_library": "llama_index.core",
                "reader_method": "SimpleDirectoryReader.load_data",
                "status": "error",
                "document_count": 0,
                "error_message": "future step output",
            }
        ]
        state.outputs["operation_telemetry"] = {"operation_name": "parse.chunk"}
        return {"executor": "test", "status": "ok"}

    monkeypatch.setattr(benchmarks_api, "execute_step", _fake_execute_step)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        first_resp = client.post(
            "/benchmarks/runs/run-step-output-snapshots/control",
            json={
                "command_id": "cmd-step-output-snapshots-1",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-step-output-snapshots",
            },
        )
        assert first_resp.status_code == 200
        second_resp = client.post(
            "/benchmarks/runs/run-step-output-snapshots/control",
            json={
                "command_id": "cmd-step-output-snapshots-2",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-step-output-snapshots",
            },
        )
        assert second_resp.status_code == 200

        stream_resp = client.get(
            "/benchmarks/runs/run-step-output-snapshots/debug-stream",
            params={
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-step-output-snapshots",
            },
        )
        assert stream_resp.status_code == 200
        events = stream_resp.json()["events"]
        load_event = next(item for item in events if item["step_name"] == "load.documents")

        detail_resp = client.get(
            f"/benchmarks/runs/run-step-output-snapshots/debug-step/{load_event['step_id']}/detail"
        )
        assert detail_resp.status_code == 200
        body = detail_resp.json()
        assert body["outputs"]["operation_telemetry"]["operation_name"] == "load.documents"
        assert body["outputs"]["document_loads"] == [
            {
                "artifact_id": "proj-step-output-snapshots:brief.md",
                "filename": "brief.md",
                "mime_type": "text/markdown",
                "reader_key": "simple_directory_reader",
                "reader_library": "llama_index.core",
                "reader_method": "SimpleDirectoryReader.load_data",
                "status": "success",
                "document_count": 1,
            }
        ]
        assert body["outputs"]["documents"][0]["artifact_id"] == "proj-step-output-snapshots:brief.md"
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_parse_chunk_runtime_logs_expose_generic_chunk_step_contract() -> None:
    document_set_ref = STORE.put_hot_subgraph(
        run_id="run-real-parse-runtime-logs",
        step_id="step-real-parse-runtime-logs-load",
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "proj-real-parse-runtime-logs:run-real-parse-runtime-logs",
            "subgraph_ref": "hot://run-real-parse-runtime-logs/document_set/step-real-parse-runtime-logs-load",
            "document_refs": ["frag-real-parse-runtime-logs-doc-1"],
        },
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-real-parse-runtime-logs",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-real-parse-runtime-logs",
            project_id="proj-real-parse-runtime-logs",
            operation_id="op-real-parse-runtime-logs",
            env_type="dev",
            env_id="dev-real-parse-runtime-logs",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-real-parse-runtime-logs",
        {
            "source_bytes": b"# parse runtime logs\n\nParagraph one.\n\nParagraph two.",
            "mime_type": "text/markdown",
            "artifact_id": "proj-real-parse-runtime-logs:run-real-parse-runtime-logs",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "document_set_ref": document_set_ref,
        },
    )

    import os
    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        next_resp = client.post(
            "/benchmarks/runs/run-real-parse-runtime-logs/control",
            json={
                "command_id": "cmd-real-parse-runtime-logs-1",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-real-parse-runtime-logs",
            },
        )
        assert next_resp.status_code == 200

        stream_resp = client.get(
            "/benchmarks/runs/run-real-parse-runtime-logs/debug-stream",
            params={
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-real-parse-runtime-logs",
            },
        )
        assert stream_resp.status_code == 200
        event = stream_resp.json()["events"][0]
        assert event["step_name"] == "parse.chunk"

        detail_resp = client.get(
            f"/benchmarks/runs/run-real-parse-runtime-logs/debug-step/{event['step_id']}/detail"
        )
        assert detail_resp.status_code == 200
        body = detail_resp.json()
        assert body["executor_logs"]["stdout_lines"] != []
        assert any(
            "parse.chunk: mapping_mode=" in line for line in body["executor_logs"]["stdout_lines"]
        )
        assert any(
            "parse.chunk: chunk_count=" in line
            for line in body["executor_logs"]["stdout_lines"]
        )
        assert any(
            "parse.chunk: asset filename=" in line and "source=primary" in line
            for line in body["executor_logs"]["stdout_lines"]
        )
        assert any(
            "parse.chunk: asset_chunks filename=" in line
            and "segments=" in line
            and "max_chars=" in line
            and "total_chars=" in line
            for line in body["executor_logs"]["stdout_lines"]
        )
        assert any(
            "parse.chunk: summary assets=" in line
            and "documents=" in line
            and "chunks=" in line
            for line in body["executor_logs"]["stdout_lines"]
        )
        assert not any("parse.chunk: mcp phase=" in line for line in body["executor_logs"]["stdout_lines"])
        assert not any("branch=semantic_map" in line for line in body["executor_logs"]["stdout_lines"])
        assert not any("agent_review" in line for line in body["executor_logs"]["stdout_lines"])
        assert "map_subgraph" not in body["outputs"]
        assert "map_agent_review" not in body["outputs"]
        assert "map_elicitation" not in body["outputs"]
        assert body["system_logs"]["stdout_lines"][0] == "executing parse.chunk operation"
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_parse_chunk_runtime_uses_python_native_chunking_without_mcp_generation(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import os
    from ikam.forja import debug_execution

    document_set_ref = STORE.put_hot_subgraph(
        run_id="run-agentic-parse-runtime-logs",
        step_id="step-agentic-parse-runtime-logs-load",
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "proj-agentic-parse-runtime-logs:run-agentic-parse-runtime-logs",
            "subgraph_ref": "hot://run-agentic-parse-runtime-logs/document_set/step-agentic-parse-runtime-logs-load",
            "document_refs": ["frag-agentic-parse-runtime-logs-doc-1"],
        },
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-agentic-parse-runtime-logs",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-agentic-parse-runtime-logs",
            project_id="proj-agentic-parse-runtime-logs",
            operation_id="op-agentic-parse-runtime-logs",
            env_type="dev",
            env_id="dev-agentic-parse-runtime-logs",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-agentic-parse-runtime-logs",
        {
            "source_bytes": b"# parse runtime logs\n\nParagraph one.\n\nParagraph two.",
            "mime_type": "text/markdown",
            "artifact_id": "proj-agentic-parse-runtime-logs:run-agentic-parse-runtime-logs",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {"mapping_mode": "full_preservation"},
            "document_set_ref": document_set_ref,
        },
    )

    called = {"mcp": 0}

    def _explode_if_called(*_args: object, **_kwargs: object) -> dict[str, object]:
        called["mcp"] += 1
        raise AssertionError("MCP map generation should not run for ingestion-early-parse parse.chunk")

    monkeypatch.setattr(debug_execution, "_invoke_mcp_map_generation", _explode_if_called, raising=False)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        next_resp = client.post(
            "/benchmarks/runs/run-agentic-parse-runtime-logs/control",
            json={
                "command_id": "cmd-agentic-parse-runtime-logs-1",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-agentic-parse-runtime-logs",
            },
        )
        assert next_resp.status_code == 200

        stream_resp = client.get(
            "/benchmarks/runs/run-agentic-parse-runtime-logs/debug-stream",
            params={
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-agentic-parse-runtime-logs",
            },
        )
        assert stream_resp.status_code == 200
        event = stream_resp.json()["events"][0]
        assert event["step_name"] == "parse.chunk"

        detail_resp = client.get(
            f"/benchmarks/runs/run-agentic-parse-runtime-logs/debug-step/{event['step_id']}/detail"
        )
        assert detail_resp.status_code == 200
        body = detail_resp.json()
        assert called["mcp"] == 0
        assert body["outputs"]["operation_telemetry"]["executor_id"] == "executor://python-primary"
        assert body["outputs"]["operation_telemetry"]["operation_name"] == "parse.chunk"
        assert body["outputs"]["operation_telemetry"]["branch"] == "python_native_chunking"
        assert body["outputs"]["operation_telemetry"]["chunk_execution_local"] is True
        assert body["outputs"]["operation_telemetry"]["document_input_mode"] == "document_set"
        assert any(
            "parse.chunk: branch=python_native_chunking" in line
            and "planner_external=false" in line
            and "chunk_execution_local=true" in line
            for line in body["executor_logs"]["stdout_lines"]
        )
        assert any(
            "parse.chunk: framework branch=python_native_chunking" in line
            and "framework=modelado" in line
            and "operation_library=modelado.operators" in line
            and "operation_method=ChunkOperator.apply" in line
            for line in body["executor_logs"]["stdout_lines"]
        )
        assert "map_subgraph" not in body["outputs"]
        assert "map_agent_review" not in body["outputs"]
        assert "map_elicitation" not in body["outputs"]
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_load_documents_runtime_logs_expose_reader_method_and_loaded_documents(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import os
    from ikam.forja import debug_execution

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-load-documents-runtime-logs",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-load-documents-runtime-logs",
            project_id="proj-load-documents-runtime-logs",
            operation_id="op-load-documents-runtime-logs",
            env_type="dev",
            env_id="dev-load-documents-runtime-logs",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-load-documents-runtime-logs",
        {
            "source_bytes": b"# Title\n\nParagraph one.\n\nParagraph two.",
            "mime_type": "text/markdown",
            "artifact_id": "proj-load-documents-runtime-logs:brief.md",
            "asset_manifest": [],
            "asset_payloads": [
                {
                    "artifact_id": "proj-load-documents-runtime-logs:brief.md",
                    "filename": "brief.md",
                    "mime_type": "text/markdown",
                    "payload": b"# Title\n\nParagraph one.\n\nParagraph two.",
                },
                {
                    "artifact_id": "proj-load-documents-runtime-logs:data.json",
                    "filename": "data.json",
                    "mime_type": "application/json",
                    "payload": b'{"revenue": 42}',
                },
                {
                    "artifact_id": "proj-load-documents-runtime-logs:table.xlsx",
                    "filename": "table.xlsx",
                    "mime_type": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                    "payload": b'PK\x03\x04',
                }
            ],
            "step_outputs": {},
        },
    )

    def _fake_load_asset(*, asset: dict[str, object], asset_index: int) -> dict[str, object]:
        filename = str(asset["filename"])
        if filename == "brief.md":
            return {
                "reader_key": "simple_directory_reader",
                "reader_library": "llama_index.core",
                "reader_method": "SimpleDirectoryReader.load_data",
                "status": "success",
                "documents": [
                    {
                        "id": "doc-1",
                        "text": "Title\n\nParagraph one.",
                        "metadata": {"file_name": "brief.md", "reader": "markdown"},
                    },
                    {
                        "id": "doc-2",
                        "text": "Paragraph two.",
                        "metadata": {"file_name": "brief.md", "section": "body"},
                    },
                ],
            }
        if filename == "data.json":
            return {
                "reader_key": "json_reader",
                "reader_library": "llama_index.readers.json",
                "reader_method": "JSONReader.load_data",
                "status": "success",
                "documents": [
                    {
                        "id": "doc-3",
                        "text": '{"revenue": 42}',
                        "metadata": {"file_name": "data.json", "reader": "json"},
                    }
                ],
            }
        return {
            "reader_key": "unsupported",
            "reader_library": "none",
            "reader_method": "none",
            "status": "unsupported",
            "documents": [],
        }

    monkeypatch.setattr(debug_execution, "_load_single_asset_for_debug_step", _fake_load_asset, raising=False)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        next_resp = client.post(
            "/benchmarks/runs/run-load-documents-runtime-logs/control",
            json={
                "command_id": "cmd-load-documents-runtime-logs-1",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-load-documents-runtime-logs",
            },
        )
        assert next_resp.status_code == 200

        stream_resp = client.get(
            "/benchmarks/runs/run-load-documents-runtime-logs/debug-stream",
            params={
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-load-documents-runtime-logs",
            },
        )
        assert stream_resp.status_code == 200
        event = stream_resp.json()["events"][0]
        assert event["step_name"] == "load.documents"

        detail_resp = client.get(
            f"/benchmarks/runs/run-load-documents-runtime-logs/debug-step/{event['step_id']}/detail"
        )
        assert detail_resp.status_code == 200
        body = detail_resp.json()
        assert body["outputs"]["operation_telemetry"]["operation_name"] == "load.documents"
        assert body["outputs"]["operation_telemetry"]["reader_dispatch_strategy"] == "per_asset"
        assert body["outputs"]["operation_telemetry"]["document_count"] == 3
        assert body["outputs"]["operation_telemetry"]["loaded_asset_count"] == 2
        assert body["outputs"]["operation_telemetry"]["unsupported_asset_count"] == 1
        assert len(body["outputs"]["documents"]) == 3
        assert len(body["outputs"]["document_loads"]) == 3
        assert any(
            "load.documents: asset_reader" in line
            and "filename=brief.md" in line
            and "reader_method=SimpleDirectoryReader.load_data" in line
            for line in body["executor_logs"]["stdout_lines"]
        )
        assert any(
            "load.documents: asset_reader" in line
            and "filename=data.json" in line
            and "reader_method=JSONReader.load_data" in line
            for line in body["executor_logs"]["stdout_lines"]
        )
        assert any(
            "load.documents: asset_status" in line
            and "filename=table.xlsx" in line
            and "status=unsupported" in line
            for line in body["executor_logs"]["stderr_lines"]
        )
        assert any(
            "load.documents: document" in line
            and "doc_id=doc-1" in line
            and "filename=brief.md" in line
            for line in body["executor_logs"]["stdout_lines"]
        )
        assert any(
            "load.documents: summary documents=3 assets=3 loaded_assets=2 errored_assets=0 unsupported_assets=1" in line
            for line in body["executor_logs"]["stdout_lines"]
        )
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_load_documents_dependency_errors_are_exposed_in_step_logs(monkeypatch) -> None:
    import os

    from ikam.forja import debug_execution
    from modelado.executors import loaders

    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-load-documents-docx-error",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-load-documents-docx-error",
            project_id="proj-load-documents-docx-error",
            operation_id="op-load-documents-docx-error",
            env_type="dev",
            env_id="dev-load-documents-docx-error",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=0,
        )
    )
    STORE.set_debug_runtime_context(
        "run-load-documents-docx-error",
        {
            "source_bytes": b"PK\x03\x04",
            "mime_type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "artifact_id": "proj-load-documents-docx-error:brief.docx",
            "asset_manifest": [
                {
                    "artifact_id": "proj-load-documents-docx-error:brief.docx",
                    "filename": "brief.docx",
                    "mime_type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                    "payload": b"PK\x03\x04",
                }
            ],
            "step_outputs": {},
        },
    )

    def _raise_docx_error(payload: dict[str, object], context: dict[str, object]):
        raise RuntimeError("docx2txt is required to read Microsoft Word files: `pip install docx2txt`")

    monkeypatch.setattr(loaders, "run", _raise_docx_error)

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        next_resp = client.post(
            "/benchmarks/runs/run-load-documents-docx-error/control",
            json={
                "command_id": "cmd-load-documents-docx-error-1",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-load-documents-docx-error",
            },
        )
        assert next_resp.status_code == 200

        stream_resp = client.get(
            "/benchmarks/runs/run-load-documents-docx-error/debug-stream",
            params={
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-load-documents-docx-error",
            },
        )
        assert stream_resp.status_code == 200
        event = stream_resp.json()["events"][0]
        assert event["step_name"] == "load.documents"

        detail_resp = client.get(
            f"/benchmarks/runs/run-load-documents-docx-error/debug-step/{event['step_id']}/detail"
        )
        assert detail_resp.status_code == 200
        body = detail_resp.json()
        assert body["outputs"]["operation_telemetry"]["errored_asset_count"] == 1
        assert body["outputs"]["documents"] == []
        assert body["outputs"]["document_loads"] == [
            {
                "artifact_id": "proj-load-documents-docx-error:brief.docx",
                "filename": "proj-load-documents-docx-error:brief.docx",
                "mime_type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "reader_key": "simple_directory_reader",
                "reader_library": "llama_index.core",
                "reader_method": "SimpleDirectoryReader.load_data",
                "status": "error",
                "document_count": 0,
                "error_message": "docx2txt is required to read Microsoft Word files: `pip install docx2txt`",
            }
        ]
        assert any(
            "load.documents: asset_status" in line
            and "status=error" in line
            and "docx2txt is required to read Microsoft Word files" in line
            for line in body["executor_logs"]["stderr_lines"]
        )
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_step_detail_log_events_returns_ordered_parse_chunk_timeline() -> None:
    document_set_ref = STORE.put_hot_subgraph(
        run_id="run-log-events-detail",
        step_id="step-log-events-detail-load",
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "proj-log-events-detail:run-log-events-detail",
            "subgraph_ref": "hot://run-log-events-detail/document_set/step-log-events-detail-load",
            "document_refs": ["frag-log-events-detail-doc-1"],
        },
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-log-events-detail",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-log-events-detail",
            project_id="proj-log-events-detail",
            operation_id="op-log-events-detail",
            env_type="dev",
            env_id="dev-log-events-detail",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-log-events-detail",
        {
            "source_bytes": b"# parse chunk log events\n\nParagraph one.\n\nParagraph two.",
            "mime_type": "text/markdown",
            "artifact_id": "proj-log-events-detail:run-log-events-detail",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "document_set_ref": document_set_ref,
        },
    )

    import os

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        next_resp = client.post(
            "/benchmarks/runs/run-log-events-detail/control",
            json={
                "command_id": "cmd-log-events-detail-1",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-log-events-detail",
            },
        )
        assert next_resp.status_code == 200

        stream_resp = client.get(
            "/benchmarks/runs/run-log-events-detail/debug-stream",
            params={
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-log-events-detail",
            },
        )
        assert stream_resp.status_code == 200
        event = stream_resp.json()["events"][0]
        assert event["step_name"] == "parse.chunk"

        detail_resp = client.get(
            f"/benchmarks/runs/run-log-events-detail/debug-step/{event['step_id']}/detail"
        )
        assert detail_resp.status_code == 200
        body = detail_resp.json()

        log_events = body["log_events"]
        assert log_events != []
        assert [item["seq"] for item in log_events] == list(range(1, len(log_events) + 1))
        for item in log_events:
            assert {"seq", "at", "source", "stream", "message"}.issubset(item.keys())
            assert isinstance(item["at"], str) and item["at"]
            assert item["source"] in {"system", "executor"}
            assert item["stream"] in {"stdout", "stderr"}
            assert isinstance(item["message"], str) and item["message"]

        system_stdout_messages = [
            item["message"]
            for item in log_events
            if item["source"] == "system" and item["stream"] == "stdout"
        ]
        executor_stdout_messages = [
            item["message"]
            for item in log_events
            if item["source"] == "executor" and item["stream"] == "stdout"
        ]
        assert system_stdout_messages[0] == "executing parse.chunk operation"
        assert system_stdout_messages[1].startswith("[parse.chunk] step started at ")
        assert any(
            "parse.chunk: asset filename=" in message and "source=primary" in message
            for message in executor_stdout_messages
        )
        assert any(
            "parse.chunk: asset_chunks filename=" in message
            and "segments=" in message
            and "max_chars=" in message
            and "total_chars=" in message
            for message in executor_stdout_messages
        )
        assert any(
            "parse.chunk: summary assets=" in message
            and "documents=" in message
            and "chunks=" in message
            and "total_duration_ms=" in message
            for message in executor_stdout_messages
        )
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_compatibility_logs_from_log_events_preserve_same_parse_chunk_content() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-compatibility-logs-detail",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-compatibility-logs-detail",
            project_id="proj-compatibility-logs-detail",
            operation_id="op-compatibility-logs-detail",
            env_type="dev",
            env_id="dev-compatibility-logs-detail",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-compatibility-logs-detail",
        {
            "source_bytes": b"# compatibility logs\n\nParagraph one.\n\nParagraph two.",
            "mime_type": "text/markdown",
            "artifact_id": "proj-compatibility-logs-detail:run-compatibility-logs-detail",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
        },
    )

    import os

    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "0"
    try:
        client = TestClient(app)
        next_resp = client.post(
            "/benchmarks/runs/run-compatibility-logs-detail/control",
            json={
                "command_id": "cmd-compatibility-logs-detail-1",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-compatibility-logs-detail",
            },
        )
        assert next_resp.status_code == 200

        stream_resp = client.get(
            "/benchmarks/runs/run-compatibility-logs-detail/debug-stream",
            params={
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-compatibility-logs-detail",
            },
        )
        assert stream_resp.status_code == 200
        event = stream_resp.json()["events"][0]
        assert event["step_name"] == "parse.chunk"

        detail_resp = client.get(
            f"/benchmarks/runs/run-compatibility-logs-detail/debug-step/{event['step_id']}/detail"
        )
        assert detail_resp.status_code == 200
        body = detail_resp.json()

        log_events = body["log_events"]
        assert log_events != []

        expected_executor_logs = {
            "stdout_lines": [
                item["message"]
                for item in log_events
                if item["source"] == "executor" and item["stream"] == "stdout"
            ],
            "stderr_lines": [
                item["message"]
                for item in log_events
                if item["source"] == "executor" and item["stream"] == "stderr"
            ],
        }
        expected_system_logs = {
            "stdout_lines": [
                item["message"]
                for item in log_events
                if item["source"] == "system" and item["stream"] == "stdout"
            ],
            "stderr_lines": [
                item["message"]
                for item in log_events
                if item["source"] == "system" and item["stream"] == "stderr"
            ],
        }
        expected_legacy_logs = {
            "stdout_lines": [item["message"] for item in log_events if item["stream"] == "stdout"],
            "stderr_lines": [item["message"] for item in log_events if item["stream"] == "stderr"],
        }

        assert body["executor_logs"] == expected_executor_logs
        assert body["system_logs"] == expected_system_logs
        assert body["logs"] == expected_legacy_logs
    finally:
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_async_next_step_emits_running_event_with_stable_step_id() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-async-same-step",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-async-same-step",
            project_id="proj-async-same-step",
            operation_id="op-async-same-step",
            env_type="dev",
            env_id="dev-async-same-step",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-async-same-step",
        {
            "source_bytes": b"# async same step\ntext",
            "mime_type": "text/markdown",
            "artifact_id": "proj-async-same-step:run-async-same-step",
            "step_outputs": {},
        },
    )

    import os
    old_test = os.environ.get("IKAM_PERF_REPORT_TEST_MODE")
    old_gate = os.environ.get("IKAM_ALLOW_DEBUG_INJECTION")
    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_PERF_REPORT_TEST_MODE"] = "1"
    os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = "1"
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "1"
    try:
        with patch(
            "ikam_perf_report.api.benchmarks.execute_step",
            side_effect=_very_slow_blocking_execute_step,
        ):
            control_response: dict[str, object] = {}

            def _run_control_request() -> None:
                with TestClient(app) as control_client:
                    control_response["response"] = control_client.post(
                        "/benchmarks/runs/run-async-same-step/control",
                        json={
                            "command_id": "cmd-async-same-step",
                            "action": "next_step",
                            "pipeline_id": "compression-rerender/v1",
                            "pipeline_run_id": "pipe-async-same-step",
                        },
                    )

            control_thread = Thread(target=_run_control_request)
            control_thread.start()

            observed_event = None
            for _ in range(60):
                running_events = STORE.list_debug_events("run-async-same-step")
                if len(running_events) == 1 and running_events[0].status == "running":
                    observed_event = running_events[0]
                    break
                time.sleep(0.01)

            assert observed_event is not None
            stable_step_id = observed_event.step_id

            with TestClient(app) as detail_client:
                detail_resp = detail_client.get(f"/benchmarks/runs/run-async-same-step/debug-step/{stable_step_id}/detail")
                assert detail_resp.status_code == 200
                detail_body = detail_resp.json()
                assert detail_body["step_id"] == stable_step_id
                assert detail_body["outcome"]["status"] == "running"
                started_message = f"[{observed_event.step_name}] step started at {observed_event.started_at}"
                assert detail_body["system_logs"]["stdout_lines"][:2] == [
                    f"executing {observed_event.step_name} operation",
                    started_message,
                ]
                system_stdout_messages = [
                    item["message"]
                    for item in detail_body["log_events"]
                    if item["source"] == "system" and item["stream"] == "stdout"
                ]
                assert system_stdout_messages[:2] == [
                    f"executing {observed_event.step_name} operation",
                    started_message,
                ]

            control_thread.join(timeout=2)
            assert not control_thread.is_alive()
            response = control_response.get("response")
            assert response is not None
            assert response.status_code == 200
    finally:
        if old_test is None:
            os.environ.pop("IKAM_PERF_REPORT_TEST_MODE", None)
        else:
            os.environ["IKAM_PERF_REPORT_TEST_MODE"] = old_test
        if old_gate is None:
            os.environ.pop("IKAM_ALLOW_DEBUG_INJECTION", None)
        else:
            os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = old_gate
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_running_parse_chunk_detail_preserves_initial_system_log_order() -> None:
    run_id = "run-running-parse-chunk-order"
    pipeline_run_id = "pipe-running-parse-chunk-order"
    document_set_ref = STORE.put_hot_subgraph(
        run_id=run_id,
        step_id="step-running-parse-chunk-order-load",
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": f"proj-running-parse-chunk-order:{run_id}",
            "subgraph_ref": f"hot://{run_id}/document_set/step-running-parse-chunk-order-load",
            "document_refs": ["frag-running-parse-chunk-order-doc-1"],
        },
    )

    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id="ingestion-early-parse",
            pipeline_run_id=pipeline_run_id,
            project_id="proj-running-parse-chunk-order",
            operation_id="op-running-parse-chunk-order",
            env_type="dev",
            env_id="dev-running-parse-chunk-order",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        run_id,
        {
            "source_bytes": b"# running parse chunk order\ntext",
            "mime_type": "text/markdown",
            "artifact_id": f"proj-running-parse-chunk-order:{run_id}",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "document_set_ref": document_set_ref,
        },
    )

    import os

    old_test = os.environ.get("IKAM_PERF_REPORT_TEST_MODE")
    old_gate = os.environ.get("IKAM_ALLOW_DEBUG_INJECTION")
    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_PERF_REPORT_TEST_MODE"] = "1"
    os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = "1"
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "1"
    try:
        with patch(
            "ikam_perf_report.api.benchmarks.execute_step",
            side_effect=_blocking_streaming_execute_step,
        ):
            control_response: dict[str, object] = {}

            def _run_control_request() -> None:
                with TestClient(app) as control_client:
                    control_response["response"] = control_client.post(
                        f"/benchmarks/runs/{run_id}/control",
                        json={
                            "command_id": "cmd-running-parse-chunk-order",
                            "action": "next_step",
                            "pipeline_id": "ingestion-early-parse",
                            "pipeline_run_id": pipeline_run_id,
                        },
                    )

            control_thread = Thread(target=_run_control_request)
            control_thread.start()

            observed_event = None
            for _ in range(60):
                observed_events = STORE.list_debug_events(run_id)
                if len(observed_events) == 1 and observed_events[0].status == "running":
                    observed_event = observed_events[0]
                    break
                time.sleep(0.01)

            assert observed_event is not None
            assert observed_event.step_name == "parse.chunk"

            with TestClient(app) as detail_client:
                detail_resp = detail_client.get(f"/benchmarks/runs/{run_id}/debug-step/{observed_event.step_id}/detail")
                assert detail_resp.status_code == 200
                detail_body = detail_resp.json()
                started_message = f"[parse.chunk] step started at {observed_event.started_at}"
                assert detail_body["outcome"]["status"] == "running"
                assert detail_body["system_logs"]["stdout_lines"][:2] == [
                    "executing parse.chunk operation",
                    started_message,
                ]
                system_stdout_messages = [
                    item["message"]
                    for item in detail_body["log_events"]
                    if item["source"] == "system" and item["stream"] == "stdout"
                ]
                assert system_stdout_messages[:2] == [
                    "executing parse.chunk operation",
                    started_message,
                ]

            control_thread.join(timeout=2)
            assert not control_thread.is_alive()
            response = control_response.get("response")
            assert response is not None
            assert response.status_code == 200
    finally:
        if old_test is None:
            os.environ.pop("IKAM_PERF_REPORT_TEST_MODE", None)
        else:
            os.environ["IKAM_PERF_REPORT_TEST_MODE"] = old_test
        if old_gate is None:
            os.environ.pop("IKAM_ALLOW_DEBUG_INJECTION", None)
        else:
            os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = old_gate
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_running_step_detail_excludes_unrelated_thread_stderr() -> None:
    run_id = "run-running-detail-unrelated-thread-stderr"
    pipeline_run_id = "pipe-running-detail-unrelated-thread-stderr"

    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id="compression-rerender/v1",
            pipeline_run_id=pipeline_run_id,
            project_id="proj-running-detail-unrelated-thread-stderr",
            operation_id="op-running-detail-unrelated-thread-stderr",
            env_type="dev",
            env_id="dev-running-detail-unrelated-thread-stderr",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        run_id,
        {
            "source_bytes": b"# running detail unrelated thread stderr\ntext",
            "mime_type": "text/markdown",
            "artifact_id": f"proj-running-detail-unrelated-thread-stderr:{run_id}",
            "step_outputs": {},
        },
    )

    import os

    old_test = os.environ.get("IKAM_PERF_REPORT_TEST_MODE")
    old_gate = os.environ.get("IKAM_ALLOW_DEBUG_INJECTION")
    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_PERF_REPORT_TEST_MODE"] = "1"
    os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = "1"
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "1"
    try:
        with patch(
            "ikam_perf_report.api.benchmarks.execute_step",
            side_effect=_blocking_execute_step_with_unrelated_thread_stderr,
        ):
            control_response: dict[str, object] = {}

            def _run_control_request() -> None:
                with TestClient(app) as control_client:
                    control_response["response"] = control_client.post(
                        f"/benchmarks/runs/{run_id}/control",
                        json={
                            "command_id": "cmd-running-detail-unrelated-thread-stderr",
                            "action": "next_step",
                            "pipeline_id": "compression-rerender/v1",
                            "pipeline_run_id": pipeline_run_id,
                        },
                    )

            control_thread = Thread(target=_run_control_request)
            control_thread.start()

            observed_event = None
            for _ in range(60):
                observed_events = STORE.list_debug_events(run_id)
                if len(observed_events) == 1 and observed_events[0].status == "running":
                    observed_event = observed_events[0]
                    break
                time.sleep(0.01)

            assert observed_event is not None

            detail_body = None
            with TestClient(app) as detail_client:
                for _ in range(60):
                    detail_resp = detail_client.get(f"/benchmarks/runs/{run_id}/debug-step/{observed_event.step_id}/detail")
                    assert detail_resp.status_code == 200
                    candidate = detail_resp.json()
                    if candidate["executor_logs"]["stdout_lines"] == ["step-owned-stdout"]:
                        detail_body = candidate
                        break
                    time.sleep(0.01)

            assert detail_body is not None
            assert detail_body["outcome"]["status"] == "running"
            assert detail_body["executor_logs"]["stdout_lines"] == ["step-owned-stdout"]
            assert detail_body["executor_logs"]["stderr_lines"] == []
            assert all(item["message"] != "unrelated-thread-stderr" for item in detail_body["log_events"])

            control_thread.join(timeout=2)
            assert not control_thread.is_alive()
            response = control_response.get("response")
            assert response is not None
            assert response.status_code == 200
    finally:
        if old_test is None:
            os.environ.pop("IKAM_PERF_REPORT_TEST_MODE", None)
        else:
            os.environ["IKAM_PERF_REPORT_TEST_MODE"] = old_test
        if old_gate is None:
            os.environ.pop("IKAM_ALLOW_DEBUG_INJECTION", None)
        else:
            os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = old_gate
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_debug_step_detail_returns_partial_logs_for_running_step() -> None:
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-running-detail",
            project_id="proj-running-detail",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-running-detail", fragments=[]),
        )
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-running-detail",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-running-detail",
            project_id="proj-running-detail",
            operation_id="op-running-detail",
            env_type="dev",
            env_id="dev-running-detail",
            execution_mode="manual",
            execution_state="running",
            current_step_name="map.conceptual.normalize.discovery",
            current_attempt_index=1,
        )
    )
    STORE.append_debug_event(
        DebugStepEvent(
            event_id="ev-running-detail",
            run_id="run-running-detail",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-running-detail",
            project_id="proj-running-detail",
            operation_id="op-running-detail",
            env_type="dev",
            env_id="dev-running-detail",
            step_name="map.conceptual.normalize.discovery",
            step_id="step-running-detail",
            status="running",
            attempt_index=1,
            retry_parent_step_id=None,
            started_at="2026-03-22T00:00:00Z",
            ended_at=None,
            duration_ms=None,
            metrics={
                "trace": {
                    "timeline": [
                        {
                            "topic": "execution.progress",
                            "event_type": "execution.running",
                            "status": "running",
                            "occurred_at": "2026-03-22T00:00:00Z",
                            "payload": {"progress": 0.1},
                        }
                    ],
                    "raw_events": [
                        {
                            "topic": "execution.progress",
                            "payload": {"progress": 0.1},
                        }
                    ],
                },
                "logs": {
                    "stdout_lines": ["chunk 1"],
                    "stderr_lines": [],
                },
            },
            error=None,
        )
    )

    client = TestClient(app)
    response = client.get("/benchmarks/runs/run-running-detail/debug-step/step-running-detail/detail")

    assert response.status_code == 200
    body = response.json()
    assert body["outcome"]["status"] == "running"
    assert body["logs"]["stdout_lines"] == ["chunk 1"]
    assert body["logs"]["stderr_lines"] == []
    assert body["executor_logs"]["stdout_lines"] == ["chunk 1"]
    assert body["executor_logs"]["stderr_lines"] == []
    assert body["system_logs"]["stdout_lines"] == []
    assert body["system_logs"]["stderr_lines"] == []
    assert body["trace"]["timeline"] == [
        {
            "topic": "execution.progress",
            "event_type": "execution.running",
            "status": "running",
            "occurred_at": "2026-03-22T00:00:00Z",
            "payload": {"progress": 0.1},
        }
    ]


async def _slow_successful_execute_step(*args, **kwargs):
    import asyncio

    await asyncio.sleep(0.2)
    return {"status": "ok"}


async def _streaming_execute_step(*args, **kwargs):
    import asyncio

    print("chunk 1")
    await asyncio.sleep(0.05)
    print("chunk 2")
    await asyncio.sleep(0.2)
    return {"status": "ok"}


async def _blocking_streaming_execute_step(*args, **kwargs):
    print("chunk 1")
    time.sleep(0.2)
    print("chunk 2")
    return {"status": "ok"}


async def _blocking_execute_step_with_unrelated_thread_stderr(*args, **kwargs):
    import sys

    polluter_done = ThreadEvent()

    def _polluter() -> None:
        print("unrelated-thread-stderr", file=sys.stderr)
        polluter_done.set()

    print("step-owned-stdout")
    polluter = Thread(target=_polluter)
    polluter.start()
    polluter_done.wait(1)
    polluter.join(timeout=1)
    time.sleep(0.3)
    return {"status": "ok"}


async def _very_slow_silent_execute_step(*args, **kwargs):
    import asyncio

    await asyncio.sleep(0.4)
    return {"status": "ok"}


async def _very_slow_blocking_execute_step(*args, **kwargs):
    import time

    time.sleep(0.4)
    return {"status": "ok"}


async def _execute_step_with_unrelated_thread_stderr(*args, **kwargs):
    import sys

    polluter_done = ThreadEvent()

    def _polluter() -> None:
        print("unrelated-thread-stderr", file=sys.stderr)
        polluter_done.set()

    polluter = Thread(target=_polluter)
    polluter.start()
    await asyncio.to_thread(polluter_done.wait, 1)
    polluter.join(timeout=1)
    print("step-owned-stdout")
    return {"status": "ok"}


async def _very_slow_silent_execute_step_with_details(*args, **kwargs):
    import asyncio

    await asyncio.sleep(0.4)
    return {
        "status": "ok",
        "details": {
            "step": "parse.chunk",
            "status": "ok",
            "surface_fragment_count": 1,
        },
    }


async def _immediate_successful_execute_step(*args, **kwargs):
    return {"status": "ok"}


def test_async_next_step_marks_selected_runnable_complete_when_alias_pipeline_finishes() -> None:
    run_id = "run-async-selected-runnable-complete"
    state = DebugRunState(
        run_id=run_id,
        pipeline_id="ingestion-early-parse",
        pipeline_run_id="pipe-async-selected-runnable-complete",
        project_id="proj-async-selected-runnable-complete",
        operation_id="op-async-selected-runnable-complete",
        env_type="dev",
        env_id="dev-async-selected-runnable-complete",
        execution_mode="manual",
        execution_state="paused",
        current_step_name="parse.claims",
        current_attempt_index=1,
    )
    STORE.create_debug_run_state(state)
    STORE.set_debug_runtime_context(
        run_id,
        {
            "source_bytes": b"# async selected runnable complete\ntext",
            "mime_type": "text/markdown",
            "artifact_id": f"proj-async-selected-runnable-complete:{run_id}",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
        },
    )

    with patch(
        "ikam_perf_report.api.benchmarks.execute_step",
        side_effect=_immediate_successful_execute_step,
    ):
        prepared_step, terminal_event = benchmarks_api._prepare_next_pipeline_step(run_id=run_id, state=state)
        assert terminal_event is None
        assert prepared_step is not None
        assert prepared_step.running_event.step_name == "complete"

        asyncio.run(
            benchmarks_api._execute_next_pipeline_step_async(
                run_id=run_id,
                state=state,
                prepared_step=prepared_step,
            )
        )

    stored_state = STORE.get_debug_run_state(run_id)
    assert stored_state is not None
    assert stored_state.current_step_name == "complete"
    assert stored_state.execution_state == "completed"


def test_concurrent_prepared_steps_do_not_cross_capture_executor_logs() -> None:
    first_started = ThreadEvent()
    release_first = ThreadEvent()

    async def _run_both(first_state, second_state, first_prepared, second_prepared):
        first_task = asyncio.create_task(
            benchmarks_api._execute_prepared_pipeline_step(
                run_id="run-log-a",
                state=first_state,
                prepared_step=first_prepared,
            )
        )
        second_task = asyncio.create_task(
            benchmarks_api._execute_prepared_pipeline_step(
                run_id="run-log-b",
                state=second_state,
                prepared_step=second_prepared,
            )
        )
        await asyncio.to_thread(first_started.wait, 1)
        await asyncio.sleep(0.05)
        release_first.set()
        return await asyncio.gather(first_task, second_task)

    async def _concurrent_execute_step(step_name, execution_state, **kwargs):
        artifact_id = execution_state.artifact_id
        if artifact_id.endswith(":run-log-a"):
            print("run-a-start")
            first_started.set()
            await asyncio.to_thread(release_first.wait, 1)
            print("run-a-end")
            return {"status": "ok", "step": step_name}

        await asyncio.to_thread(first_started.wait, 1)
        print("run-b-start")
        await asyncio.sleep(0.05)
        print("run-b-end")
        return {"status": "ok", "step": step_name}

    for suffix in ("a", "b"):
        run_id = f"run-log-{suffix}"
        STORE.create_debug_run_state(
            DebugRunState(
                run_id=run_id,
                pipeline_id="compression-rerender/v1",
                pipeline_run_id=f"pipe-log-{suffix}",
                project_id=f"proj-log-{suffix}",
                operation_id=f"op-log-{suffix}",
                env_type="dev",
                env_id=f"dev-log-{suffix}",
                execution_mode="manual",
                execution_state="paused",
                current_step_name="init.initialize",
                current_attempt_index=1,
            )
        )
        STORE.set_debug_runtime_context(
            run_id,
            {
                "source_bytes": f"# {run_id}\ntext".encode(),
                "mime_type": "text/markdown",
                "artifact_id": f"proj-log-{suffix}:{run_id}",
                "step_outputs": {},
            },
        )

    first_state = STORE.get_debug_run_state("run-log-a")
    second_state = STORE.get_debug_run_state("run-log-b")
    assert first_state is not None
    assert second_state is not None

    with patch(
        "ikam_perf_report.api.benchmarks.execute_step",
        side_effect=_concurrent_execute_step,
    ):
        first_prepared, first_terminal = benchmarks_api._prepare_next_pipeline_step(run_id="run-log-a", state=first_state)
        second_prepared, second_terminal = benchmarks_api._prepare_next_pipeline_step(run_id="run-log-b", state=second_state)
        assert first_terminal is None
        assert second_terminal is None
        assert first_prepared is not None
        assert second_prepared is not None

        first_event, second_event = asyncio.run(
            _run_both(first_state, second_state, first_prepared, second_prepared)
        )

    assert first_event.metrics["executor_logs"]["stdout_lines"] == ["run-a-start", "run-a-end"]
    assert second_event.metrics["executor_logs"]["stdout_lines"] == ["run-b-start", "run-b-end"]


def test_prepared_step_does_not_capture_unrelated_thread_stderr() -> None:
    run_id = "run-unrelated-thread-stderr"
    state = DebugRunState(
        run_id=run_id,
        pipeline_id="compression-rerender/v1",
        pipeline_run_id="pipe-unrelated-thread-stderr",
        project_id="proj-unrelated-thread-stderr",
        operation_id="op-unrelated-thread-stderr",
        env_type="dev",
        env_id="dev-unrelated-thread-stderr",
        execution_mode="manual",
        execution_state="paused",
        current_step_name="init.initialize",
        current_attempt_index=1,
    )
    STORE.create_debug_run_state(state)
    STORE.set_debug_runtime_context(
        run_id,
        {
            "source_bytes": b"# unrelated thread stderr\ntext",
            "mime_type": "text/markdown",
            "artifact_id": f"proj-unrelated-thread-stderr:{run_id}",
            "step_outputs": {},
        },
    )

    with patch(
        "ikam_perf_report.api.benchmarks.execute_step",
        side_effect=_execute_step_with_unrelated_thread_stderr,
    ):
        prepared_step, terminal_event = benchmarks_api._prepare_next_pipeline_step(run_id=run_id, state=state)
        assert terminal_event is None
        assert prepared_step is not None

        event = asyncio.run(
            benchmarks_api._execute_prepared_pipeline_step(
                run_id=run_id,
                state=state,
                prepared_step=prepared_step,
            )
        )

    assert event.metrics["executor_logs"]["stdout_lines"] == ["step-owned-stdout"]
    assert event.metrics["executor_logs"]["stderr_lines"] == []
    assert all(item["message"] != "unrelated-thread-stderr" for item in event.metrics["log_events"])


def test_async_next_step_updates_running_event_logs_before_completion() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-async-log-stream",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-async-log-stream",
            project_id="proj-async-log-stream",
            operation_id="op-async-log-stream",
            env_type="dev",
            env_id="dev-async-log-stream",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-async-log-stream",
        {
            "source_bytes": b"# async log stream\ntext",
            "mime_type": "text/markdown",
            "artifact_id": "proj-async-log-stream:run-async-log-stream",
            "step_outputs": {},
        },
    )

    import os
    old_test = os.environ.get("IKAM_PERF_REPORT_TEST_MODE")
    old_gate = os.environ.get("IKAM_ALLOW_DEBUG_INJECTION")
    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_PERF_REPORT_TEST_MODE"] = "1"
    os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = "1"
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "1"
    try:
        client = TestClient(app)
        with patch(
            "ikam_perf_report.api.benchmarks.execute_step",
            side_effect=_streaming_execute_step,
        ):
            response = client.post(
                "/benchmarks/runs/run-async-log-stream/control",
                json={
                    "command_id": "cmd-async-log-stream",
                    "action": "next_step",
                    "pipeline_id": "compression-rerender/v1",
                    "pipeline_run_id": "pipe-async-log-stream",
                },
            )
            assert response.status_code == 200

            observed_event = None
            for _ in range(20):
                running_events = STORE.list_debug_events("run-async-log-stream")
                if len(running_events) != 1:
                    time.sleep(0.01)
                    continue
                observed_event = running_events[0]
                executor_stdout_lines = observed_event.metrics.get("executor_logs", {}).get("stdout_lines") or []
                if executor_stdout_lines and executor_stdout_lines[0] == "chunk 1":
                    break
                time.sleep(0.01)

            assert observed_event is not None
            assert (observed_event.metrics.get("executor_logs", {}).get("stdout_lines") or [])[0] == "chunk 1"
            system_stdout_lines = observed_event.metrics.get("system_logs", {}).get("stdout_lines") or []
            assert system_stdout_lines[0] == f"executing {observed_event.step_name} operation"

            detail_resp = client.get(f"/benchmarks/runs/run-async-log-stream/debug-step/{observed_event.step_id}/detail")
            assert detail_resp.status_code == 200
            detail_body = detail_resp.json()
            assert detail_body["outcome"]["status"] in {"running", "succeeded"}
            assert detail_body["executor_logs"]["stdout_lines"][0] == "chunk 1"
            assert detail_body["system_logs"]["stdout_lines"][0] == (
                f"executing {observed_event.step_name} operation"
            )
    finally:
        if old_test is None:
            os.environ.pop("IKAM_PERF_REPORT_TEST_MODE", None)
        else:
            os.environ["IKAM_PERF_REPORT_TEST_MODE"] = old_test
        if old_gate is None:
            os.environ.pop("IKAM_ALLOW_DEBUG_INJECTION", None)
        else:
            os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = old_gate
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_async_next_step_seeds_substantial_startup_log_immediately() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-async-start-log",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-async-start-log",
            project_id="proj-async-start-log",
            operation_id="op-async-start-log",
            env_type="dev",
            env_id="dev-async-start-log",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-async-start-log",
        {
            "source_bytes": b"# async start log\ntext",
            "mime_type": "text/markdown",
            "artifact_id": "proj-async-start-log:run-async-start-log",
            "step_outputs": {},
        },
    )

    import os
    old_test = os.environ.get("IKAM_PERF_REPORT_TEST_MODE")
    old_gate = os.environ.get("IKAM_ALLOW_DEBUG_INJECTION")
    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_PERF_REPORT_TEST_MODE"] = "1"
    os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = "1"
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "1"
    try:
        client = TestClient(app)
        with patch(
            "ikam_perf_report.api.benchmarks.execute_step",
            side_effect=_slow_successful_execute_step,
        ):
            response = client.post(
                "/benchmarks/runs/run-async-start-log/control",
                json={
                    "command_id": "cmd-async-start-log",
                    "action": "next_step",
                    "pipeline_id": "compression-rerender/v1",
                    "pipeline_run_id": "pipe-async-start-log",
                },
            )
            assert response.status_code == 200

            observed_events = []
            for _ in range(20):
                observed_events = STORE.list_debug_events("run-async-start-log")
                if len(observed_events) == 1 and observed_events[0].metrics.get("system_logs", {}).get("stdout_lines"):
                    break
                time.sleep(0.01)

            assert len(observed_events) == 1
            assert observed_events[0].metrics.get("system_logs", {}).get("stdout_lines", [])[0] == (
                f"executing {observed_events[0].step_name} operation"
            )
    finally:
        if old_test is None:
            os.environ.pop("IKAM_PERF_REPORT_TEST_MODE", None)
        else:
            os.environ["IKAM_PERF_REPORT_TEST_MODE"] = old_test
        if old_gate is None:
            os.environ.pop("IKAM_ALLOW_DEBUG_INJECTION", None)
        else:
            os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = old_gate
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_async_next_step_emits_heartbeat_logs_for_long_running_silent_step() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-async-heartbeat",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-async-heartbeat",
            project_id="proj-async-heartbeat",
            operation_id="op-async-heartbeat",
            env_type="dev",
            env_id="dev-async-heartbeat",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-async-heartbeat",
        {
            "source_bytes": b"# async heartbeat\ntext",
            "mime_type": "text/markdown",
            "artifact_id": "proj-async-heartbeat:run-async-heartbeat",
            "step_outputs": {},
        },
    )

    import os
    old_test = os.environ.get("IKAM_PERF_REPORT_TEST_MODE")
    old_gate = os.environ.get("IKAM_ALLOW_DEBUG_INJECTION")
    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_PERF_REPORT_TEST_MODE"] = "1"
    os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = "1"
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "1"
    try:
        client = TestClient(app)
        with patch(
            "ikam_perf_report.api.benchmarks.execute_step",
            side_effect=_very_slow_silent_execute_step,
        ), patch(
            "ikam_perf_report.api.benchmarks._RUNNING_LOG_HEARTBEAT_INTERVAL_SECONDS",
            0.05,
        ):
            response = client.post(
                "/benchmarks/runs/run-async-heartbeat/control",
                json={
                    "command_id": "cmd-async-heartbeat",
                    "action": "next_step",
                    "pipeline_id": "compression-rerender/v1",
                    "pipeline_run_id": "pipe-async-heartbeat",
                },
            )
            assert response.status_code == 200

            observed_events = []
            heartbeat_lines = []
            for _ in range(40):
                observed_events = STORE.list_debug_events("run-async-heartbeat")
                if len(observed_events) == 1:
                    stdout_lines = observed_events[0].metrics.get("system_logs", {}).get("stdout_lines") or []
                    heartbeat_lines = [line for line in stdout_lines if "still running" in line]
                    if heartbeat_lines:
                        break
                time.sleep(0.01)

            assert len(observed_events) == 1
            assert heartbeat_lines != []
            assert observed_events[0].metrics.get("executor_logs", {}).get("stdout_lines") == []
    finally:
        if old_test is None:
            os.environ.pop("IKAM_PERF_REPORT_TEST_MODE", None)
        else:
            os.environ["IKAM_PERF_REPORT_TEST_MODE"] = old_test
        if old_gate is None:
            os.environ.pop("IKAM_ALLOW_DEBUG_INJECTION", None)
        else:
            os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = old_gate
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_async_next_step_emits_heartbeat_logs_for_blocking_long_running_step() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-async-heartbeat-blocking",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-async-heartbeat-blocking",
            project_id="proj-async-heartbeat-blocking",
            operation_id="op-async-heartbeat-blocking",
            env_type="dev",
            env_id="dev-async-heartbeat-blocking",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-async-heartbeat-blocking",
        {
            "source_bytes": b"# async heartbeat blocking\ntext",
            "mime_type": "text/markdown",
            "artifact_id": "proj-async-heartbeat-blocking:run-async-heartbeat-blocking",
            "step_outputs": {},
        },
    )

    import os
    old_test = os.environ.get("IKAM_PERF_REPORT_TEST_MODE")
    old_gate = os.environ.get("IKAM_ALLOW_DEBUG_INJECTION")
    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_PERF_REPORT_TEST_MODE"] = "1"
    os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = "1"
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "1"
    try:
        client = TestClient(app)
        with patch(
            "ikam_perf_report.api.benchmarks.execute_step",
            side_effect=_very_slow_blocking_execute_step,
        ), patch(
            "ikam_perf_report.api.benchmarks._RUNNING_LOG_HEARTBEAT_INTERVAL_SECONDS",
            0.05,
        ):
            response = client.post(
                "/benchmarks/runs/run-async-heartbeat-blocking/control",
                json={
                    "command_id": "cmd-async-heartbeat-blocking",
                    "action": "next_step",
                    "pipeline_id": "compression-rerender/v1",
                    "pipeline_run_id": "pipe-async-heartbeat-blocking",
                },
            )
            assert response.status_code == 200

            observed_events = []
            heartbeat_lines = []
            for _ in range(40):
                observed_events = STORE.list_debug_events("run-async-heartbeat-blocking")
                if len(observed_events) == 1:
                    stdout_lines = observed_events[0].metrics.get("system_logs", {}).get("stdout_lines") or []
                    heartbeat_lines = [line for line in stdout_lines if "still running" in line]
                    if heartbeat_lines:
                        break
                time.sleep(0.01)

            assert len(observed_events) == 1
            assert heartbeat_lines != []
            assert observed_events[0].metrics.get("executor_logs", {}).get("stdout_lines") == []
    finally:
        if old_test is None:
            os.environ.pop("IKAM_PERF_REPORT_TEST_MODE", None)
        else:
            os.environ["IKAM_PERF_REPORT_TEST_MODE"] = old_test
        if old_gate is None:
            os.environ.pop("IKAM_ALLOW_DEBUG_INJECTION", None)
        else:
            os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = old_gate
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_system_log_order_keeps_runtime_parse_chunk_lifecycle_sequence() -> None:
    document_set_ref = STORE.put_hot_subgraph(
        run_id="run-system-log-order",
        step_id="step-system-log-order-load",
        contract_type="document_set",
        payload={
            "kind": "document_set",
            "artifact_head_ref": "proj-system-log-order:run-system-log-order",
            "subgraph_ref": "hot://run-system-log-order/document_set/step-system-log-order-load",
            "document_refs": ["frag-system-log-order-doc-1"],
        },
    )
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-system-log-order",
            pipeline_id="ingestion-early-parse",
            pipeline_run_id="pipe-system-log-order",
            project_id="proj-system-log-order",
            operation_id="op-system-log-order",
            env_type="dev",
            env_id="dev-system-log-order",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-system-log-order",
        {
            "source_bytes": b"# system log order\n\nparagraph one\n\nparagraph two",
            "mime_type": "text/markdown",
            "artifact_id": "proj-system-log-order:run-system-log-order",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
            "document_set_ref": document_set_ref,
        },
    )

    client = TestClient(app)
    with patch(
        "ikam_perf_report.api.benchmarks.execute_step",
        side_effect=_very_slow_silent_execute_step_with_details,
    ), patch(
        "ikam_perf_report.api.benchmarks._RUNNING_LOG_HEARTBEAT_INTERVAL_SECONDS",
        0.05,
    ):
        next_resp = client.post(
            "/benchmarks/runs/run-system-log-order/control",
            json={
                "command_id": "cmd-system-log-order",
                "action": "next_step",
                "pipeline_id": "ingestion-early-parse",
                "pipeline_run_id": "pipe-system-log-order",
            },
        )
        assert next_resp.status_code == 200

    stream_resp = client.get(
        "/benchmarks/runs/run-system-log-order/debug-stream",
        params={
            "pipeline_id": "ingestion-early-parse",
            "pipeline_run_id": "pipe-system-log-order",
        },
    )
    assert stream_resp.status_code == 200
    event = stream_resp.json()["events"][0]
    assert event["step_name"] == "parse.chunk"

    detail_resp = client.get(
        f"/benchmarks/runs/run-system-log-order/debug-step/{event['step_id']}/detail"
    )
    assert detail_resp.status_code == 200
    body = detail_resp.json()

    system_stdout_messages = [
        item["message"]
        for item in body["log_events"]
        if item["source"] == "system" and item["stream"] == "stdout"
    ]
    assert system_stdout_messages[0] == "executing parse.chunk operation"
    assert system_stdout_messages[1].startswith("[parse.chunk] step started at ")
    assert system_stdout_messages[-1].startswith("[parse.chunk] step finished at ")

    middle_messages = system_stdout_messages[2:-1]
    assert middle_messages != []
    assert all(message == "[parse.chunk] still running" for message in middle_messages)


def test_async_next_step_for_selected_runnable_updates_state_and_exposes_running_event_before_completion() -> None:
    run_id = "run-async-selected-runnable"
    pipeline_run_id = "pipe-async-selected-runnable"

    STORE.create_debug_run_state(
        DebugRunState(
            run_id=run_id,
            pipeline_id="ingestion-early-parse",
            pipeline_run_id=pipeline_run_id,
            project_id="proj-async-selected-runnable",
            operation_id="op-async-selected-runnable",
            env_type="dev",
            env_id="dev-async-selected-runnable",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="load.documents",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        run_id,
        {
            "source_bytes": b"# async selected runnable\ntext",
            "mime_type": "text/markdown",
            "artifact_id": f"proj-async-selected-runnable:{run_id}",
            "asset_manifest": [],
            "asset_payloads": [],
            "step_outputs": {},
        },
    )

    import os
    old_test = os.environ.get("IKAM_PERF_REPORT_TEST_MODE")
    old_gate = os.environ.get("IKAM_ALLOW_DEBUG_INJECTION")
    old_async = os.environ.get("IKAM_ASYNC_NEXT_STEP")
    os.environ["IKAM_PERF_REPORT_TEST_MODE"] = "1"
    os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = "1"
    os.environ["IKAM_ASYNC_NEXT_STEP"] = "1"
    try:
        client = TestClient(app)
        with client:
            with patch(
                "ikam_perf_report.api.benchmarks.execute_step",
                side_effect=_blocking_streaming_execute_step,
            ):
                control_response: dict[str, object] = {}

                def _run_control_request() -> None:
                    control_response["response"] = client.post(
                        f"/benchmarks/runs/{run_id}/control",
                        json={
                            "command_id": "cmd-async-selected-runnable",
                            "action": "next_step",
                            "pipeline_id": "ingestion-early-parse",
                            "pipeline_run_id": pipeline_run_id,
                        },
                    )
                
                control_thread = Thread(target=_run_control_request)
                control_thread.start()

                stream_body = None
                for _ in range(20):
                    stream_resp = client.get(
                        f"/benchmarks/runs/{run_id}/debug-stream",
                        params={"pipeline_id": "ingestion-early-parse", "pipeline_run_id": pipeline_run_id},
                    )
                    assert stream_resp.status_code == 200
                    stream_body = stream_resp.json()
                    if stream_body["execution_state"] == "running" and stream_body["events"]:
                        break
                    time.sleep(0.01)

                assert stream_body is not None
                assert stream_body["execution_state"] == "running"
                assert [event["step_name"] for event in stream_body["events"]] == ["parse.chunk"]
                assert stream_body["events"][0]["status"] == "running"
                assert stream_body["events"][0]["metrics"]["system_logs"]["stdout_lines"][:1] == [
                    "executing parse.chunk operation",
                ]

                control_thread.join(timeout=2)
                assert not control_thread.is_alive()
                control_resp = control_response.get("response")
                assert control_resp is not None
                assert control_resp.status_code == 200
                control_body = control_resp.json()
                assert control_body["state"]["execution_state"] == "running"
                assert control_body["state"]["current_step_name"] == "parse.chunk"
    finally:
        if old_test is None:
            os.environ.pop("IKAM_PERF_REPORT_TEST_MODE", None)
        else:
            os.environ["IKAM_PERF_REPORT_TEST_MODE"] = old_test
        if old_gate is None:
            os.environ.pop("IKAM_ALLOW_DEBUG_INJECTION", None)
        else:
            os.environ["IKAM_ALLOW_DEBUG_INJECTION"] = old_gate
        if old_async is None:
            os.environ.pop("IKAM_ASYNC_NEXT_STEP", None)
        else:
            os.environ["IKAM_ASYNC_NEXT_STEP"] = old_async


def test_resume_runs_all_remaining_steps_to_completion() -> None:
    """resume action executes all remaining steps from current position
    through project_graph, sets execution_state to 'completed', and
    returns the events generated during the resume."""
    import os
    import psycopg

    db_url = "postgresql://user:pass@localhost:55432/ikam_perf_report"
    try:
        with psycopg.connect(db_url, connect_timeout=3) as conn:
            conn.execute("SELECT 1")
    except Exception:
        pytest.skip("pgvector Postgres not available on port 55432")

    old_db = os.environ.get("DATABASE_URL")
    os.environ["DATABASE_URL"] = db_url
    from modelado.db import reset_pool_for_pytest
    reset_pool_for_pytest()

    try:
        _test_resume_runs_all_remaining_steps_impl()
    finally:
        if old_db is None:
            os.environ.pop("DATABASE_URL", None)
        else:
            os.environ["DATABASE_URL"] = old_db
        reset_pool_for_pytest()


def _test_resume_runs_all_remaining_steps_impl() -> None:
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-resume",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-resume",
            project_id="proj-resume",
            operation_id="op-resume",
            env_type="dev",
            env_id="dev-resume",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-resume",
        {
            "source_bytes": b"# resume test\nContent for full pipeline run.",
            "mime_type": "text/markdown",
            "artifact_id": "proj-resume:run-resume",
            "step_outputs": {},
        },
    )

    client = TestClient(app)

    # Step once manually: prepare_case → decompose
    step_resp = client.post(
        "/benchmarks/runs/run-resume/control",
        json={
            "command_id": "cmd-resume-step-0",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-resume",
        },
    )
    assert step_resp.status_code == 200
    assert step_resp.json()["state"]["current_step_name"] == "map.conceptual.lift.surface_fragments"

    # Now resume: should run all remaining steps (embed_decomposed→...→project_graph)
    resume_resp = client.post(
        "/benchmarks/runs/run-resume/control",
        json={
            "command_id": "cmd-resume-go",
            "action": "resume",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-resume",
        },
    )
    assert resume_resp.status_code == 200
    data = resume_resp.json()

    # Discriminating: state must be "completed", not "paused"
    assert data["state"]["execution_state"] == "completed"

    # Must have reached terminal step
    assert data["state"]["current_step_name"] == "map.reconstructable.build_subgraph.reconstruction"

    # Resume response must include events generated during the resume
    assert "events" in data, "resume response must include events list"
    resume_events = data["events"]
    step_names = [e["step_name"] for e in resume_events]

    # Should have executed: embed_decomposed, lift, embed_lifted, candidate_search, normalize,
    # compose_proposal, verify, promote_commit, project_graph (12 steps)
    assert len(resume_events) == 12, f"Expected 12 resume events, got {len(resume_events)}: {step_names}"
    assert step_names[-1] == "map.reconstructable.build_subgraph.reconstruction"
    assert "map.conceptual.normalize.discovery" in step_names
    assert _get_verify_gate_step() in step_names

    # All resume events should have succeeded
    for ev in resume_events:
        assert ev["status"] == "succeeded", f"Step {ev['step_name']} failed: {ev.get('error')}"


def test_drill_through_returns_404_for_unknown_step_id() -> None:
    """GET /benchmarks/runs/{run_id}/debug-step/{step_id}/detail
    returns 404 when step_id doesn't match any event."""
    STORE.create_debug_run_state(
        DebugRunState(
            run_id="run-drill-404",
            pipeline_id="compression-rerender/v1",
            pipeline_run_id="pipe-drill-404",
            project_id="proj-drill-404",
            operation_id="op-drill-404",
            env_type="dev",
            env_id="dev-drill-404",
            execution_mode="manual",
            execution_state="paused",
            current_step_name="init.initialize",
            current_attempt_index=1,
        )
    )
    STORE.set_debug_runtime_context(
        "run-drill-404",
        {
            "source_bytes": b"# test",
            "mime_type": "text/markdown",
            "artifact_id": "proj-drill-404:run-drill-404",
            "step_outputs": {},
        },
    )

    client = TestClient(app)
    detail_resp = client.get("/benchmarks/runs/run-drill-404/debug-step/nonexistent-step/detail")
    assert detail_resp.status_code == 404


def test_prepare_case_detail_lists_case_artifact_manifest(case_fixtures_root) -> None:
    """prepare_case detail must list fixture artifacts, not a synthetic case-as-artifact id."""
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    assert stream_resp.status_code == 200
    events = stream_resp.json()["events"]
    prepare_event = event_with_step(events, "init.initialize")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{prepare_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()

    outputs = payload["outputs"]
    manifest = outputs.get("artifact_manifest", [])
    assert isinstance(manifest, list)
    assert len(manifest) >= 4

    names = {item["filename"] for item in manifest}
    assert "revenue_plan.md" in names
    assert "metrics.json" in names
    assert "msa.pdf" in names
    assert "assets/image.png" in names
    assert "idea.md" not in names

    lineage = payload["lineage"]
    roots = lineage.get("roots", [])
    assert isinstance(roots, list)
    assert len(roots) == len(manifest)


def test_later_step_detail_lineage_includes_all_env_fragments(case_fixtures_root) -> None:
    """Later-step drill-through should still show environment fragment tree, not just artifact nodes."""
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    # advance to project_graph so we inspect a late step
    for index in range(13):
        control = client.post(
            f"/benchmarks/runs/{run_id}/control",
            json={
                "command_id": f"cmd-late-{index}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": run_id,
            },
        )
        assert control.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    assert stream_resp.status_code == 200
    events = stream_resp.json()["events"]
    decompose_event = event_with_step(events, "map.conceptual.lift.surface_fragments")
    project_event = event_with_step(events, "map.reconstructable.build_subgraph.reconstruction")

    decompose_detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{decompose_event['step_id']}/detail")
    assert decompose_detail_resp.status_code == 200
    decompose_payload = decompose_detail_resp.json()
    decompose_fragment_ids = {
        str(node.get("fragment_id"))
        for node in decompose_payload["lineage"]["nodes"]
        if isinstance(node, dict) and str(node.get("node_id", "")).startswith("fragment:")
    }

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{project_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()

    fragment_ids = {
        str(node.get("fragment_id"))
        for node in payload["lineage"]["nodes"]
        if isinstance(node, dict) and str(node.get("node_id", "")).startswith("fragment:")
    }
    # Late-step lineage should retain decompose fragment tree, not collapse to artifacts only.
    assert decompose_fragment_ids
    assert decompose_fragment_ids.issubset(fragment_ids)


def test_lift_detail_includes_surface_to_ir_transformations(case_fixtures_root) -> None:
    """lift detail should expose per-surface transformation status and IR targets."""
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    for index in range(6):
        control = client.post(
            f"/benchmarks/runs/{run_id}/control",
            json={
                "command_id": f"cmd-lift-detail-{index}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": run_id,
            },
        )
        assert control.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    assert stream_resp.status_code == 200
    events = stream_resp.json()["events"]
    lift_event = event_with_step(events, "map.conceptual.normalize.discovery")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{lift_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()

    outputs = payload.get("outputs", {})
    transformations = outputs.get("lift_transformations", [])
    assert isinstance(transformations, list)
    assert len(transformations) > 0
    assert all(isinstance(item.get("surface_fragment_id"), str) for item in transformations if isinstance(item, dict))
    assert all(item.get("lift_status") in {"lifted", "surface_only"} for item in transformations if isinstance(item, dict))
    assert all(isinstance(item.get("ir_fragment_ids"), list) for item in transformations if isinstance(item, dict))


def test_decompose_detail_uses_full_manifest_inputs_and_file_names(case_fixtures_root) -> None:
    """decompose detail must stay case-wide, not collapse to one synthetic artifact."""
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    events = stream_resp.json()["events"]
    prepare_event = event_with_step(events, "init.initialize")

    prepare_detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{prepare_event['step_id']}/detail")
    assert prepare_detail_resp.status_code == 200
    prepare_detail = prepare_detail_resp.json()
    manifest_ids = [item["artifact_id"] for item in prepare_detail["outputs"]["artifact_manifest"]]
    assert len(manifest_ids) >= 4

    step_resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": "cmd-case-wide-decompose",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": run_id,
        },
    )
    assert step_resp.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    events = stream_resp.json()["events"]
    decompose_event = event_with_step(events, "map.conceptual.lift.surface_fragments")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{decompose_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()

    assert sorted(payload["inputs"]["artifact_ids"]) == sorted(manifest_ids)

    decomposition = payload["outputs"]["decomposition"]
    assert decomposition["artifact_count"] >= 2
    links = decomposition["artifact_surface_links"]
    linked_artifacts = {item["artifact_id"] for item in links}
    assert len(linked_artifacts) >= 2

    fragment_nodes = [
        node
        for node in payload["lineage"]["nodes"]
        if node.get("node_id", "").startswith("fragment:")
    ]
    assert any(node.get("meta", {}).get("file_name") for node in fragment_nodes)


def test_artifact_preview_endpoint_returns_typed_payloads(case_fixtures_root) -> None:
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    prepare_event = event_with_step(stream_resp.json()["events"], "init.initialize")
    detail = client.get(f"/benchmarks/runs/{run_id}/debug-step/{prepare_event['step_id']}/detail").json()
    manifest = detail["outputs"]["artifact_manifest"]
    md_artifact_id = next(item["artifact_id"] for item in manifest if item["filename"].endswith(".md"))
    json_artifact_id = next(item["artifact_id"] for item in manifest if item["filename"].endswith(".json"))
    pdf_artifact_id = next(item["artifact_id"] for item in manifest if item["filename"].endswith(".pdf"))
    docx_entry = next((item for item in manifest if str(item["filename"]).endswith(".docx")), None)

    md_resp = client.get(f"/benchmarks/runs/{run_id}/artifacts/preview", params={"artifact_id": md_artifact_id})
    assert md_resp.status_code == 200
    md_payload = md_resp.json()
    assert md_payload["kind"] == "text"
    assert isinstance(md_payload["preview"].get("text"), str)

    json_resp = client.get(f"/benchmarks/runs/{run_id}/artifacts/preview", params={"artifact_id": json_artifact_id})
    assert json_resp.status_code == 200
    json_payload = json_resp.json()
    assert json_payload["kind"] == "json"
    assert isinstance(json_payload["preview"].get("parsed"), dict)

    pdf_resp = client.get(f"/benchmarks/runs/{run_id}/artifacts/preview", params={"artifact_id": pdf_artifact_id})
    assert pdf_resp.status_code == 200
    pdf_payload = pdf_resp.json()
    assert pdf_payload["kind"] == "pdf"
    assert pdf_payload["preview"].get("encoding") == "base64"
    assert isinstance(pdf_payload["preview"].get("bytes_b64"), str)

    if docx_entry is not None:
        docx_resp = client.get(
            f"/benchmarks/runs/{run_id}/artifacts/preview",
            params={"artifact_id": docx_entry["filename"]},
        )
        assert docx_resp.status_code == 200
        docx_payload = docx_resp.json()
        assert docx_payload["kind"] == "doc"
        assert docx_payload["mime_type"] == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        assert isinstance(docx_payload["preview"].get("paragraphs"), list)


def test_decompose_detail_includes_per_asset_statuses(case_fixtures_root) -> None:
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    step_resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": "cmd-asset-status",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": run_id,
        },
    )
    assert step_resp.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    decompose_event = event_with_step(stream_resp.json()["events"], "map.conceptual.lift.surface_fragments")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{decompose_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()

    statuses = payload["outputs"]["decomposition"].get("asset_statuses")
    assert isinstance(statuses, list)
    assert statuses
    assert all("artifact_id" in item and "status" in item for item in statuses)
    assert all(item["status"] in {"decomposed", "fallback", "failed"} for item in statuses)

    lineage_nodes = payload["lineage"]["nodes"]
    artifact_mime_by_artifact_id = {
        str(node.get("fragment_id")): node.get("mime_type")
        for node in lineage_nodes
        if node.get("kind") == "artifact"
    }
    for item in statuses:
        artifact_id = str(item.get("artifact_id") or "")
        status_mime = str(item.get("mime_type") or "")
        if artifact_id and status_mime and artifact_id in artifact_mime_by_artifact_id:
            assert artifact_mime_by_artifact_id[artifact_id] == status_mime

    docx_artifact_nodes = [
        node
        for node in lineage_nodes
        if node.get("kind") == "artifact"
        and node.get("mime_type") == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    ]
    for node in docx_artifact_nodes:
        meta = node.get("meta") if isinstance(node.get("meta"), dict) else {}
        value_preview = meta.get("value_preview")
        assert isinstance(value_preview, str)
        assert value_preview.strip()


def test_artifact_preview_supports_filename_lookup(case_fixtures_root) -> None:
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    prepare_event = event_with_step(stream_resp.json()["events"], "init.initialize")
    detail = client.get(f"/benchmarks/runs/{run_id}/debug-step/{prepare_event['step_id']}/detail").json()
    manifest = detail["outputs"]["artifact_manifest"]
    text_artifact = next(item for item in manifest if str(item["filename"]).endswith(".md"))

    by_id = client.get(
        f"/benchmarks/runs/{run_id}/artifacts/preview",
        params={"artifact_id": text_artifact["artifact_id"]},
    )
    assert by_id.status_code == 200
    payload_by_id = by_id.json()
    assert payload_by_id["kind"] == "text"
    assert isinstance(payload_by_id["preview"].get("text"), str)

    by_filename = client.get(
        f"/benchmarks/runs/{run_id}/artifacts/preview",
        params={"artifact_id": text_artifact["filename"]},
    )
    assert by_filename.status_code == 200
    payload_by_filename = by_filename.json()
    assert payload_by_filename["kind"] == "text"
    assert isinstance(payload_by_filename["preview"].get("text"), str)


def test_docx_artifact_node_and_preview_include_content(case_fixtures_root) -> None:
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false", "reset": "true"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    step_resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": "cmd-docx-map",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": run_id,
        },
    )
    assert step_resp.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    map_event = event_with_step(stream_resp.json()["events"], "map.conceptual.lift.surface_fragments")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{map_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    detail = detail_resp.json()

    docx_artifact_node = next(
        (
            node
            for node in detail["lineage"]["nodes"]
            if node.get("kind") == "artifact"
            and str(node.get("label", "")).endswith(".docx")
        ),
        None,
    )
    if docx_artifact_node is None:
        pytest.skip("case fixture did not include a DOCX artifact in this test run")
    assert docx_artifact_node is not None
    assert docx_artifact_node["mime_type"] == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    value_preview = (docx_artifact_node.get("meta") or {}).get("value_preview")
    assert isinstance(value_preview, str)
    assert value_preview.strip()

    preview_resp = client.get(
        f"/benchmarks/runs/{run_id}/artifacts/preview",
        params={"artifact_id": docx_artifact_node["label"]},
    )
    assert preview_resp.status_code == 200
    preview_payload = preview_resp.json()
    assert preview_payload["kind"] == "doc"
    assert isinstance(preview_payload["preview"].get("paragraphs"), list)
    assert len(preview_payload["preview"].get("paragraphs") or []) > 0


def test_map_detail_includes_structural_map_contract(case_fixtures_root) -> None:
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    step_resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": "cmd-map-structural-contract",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": run_id,
        },
    )
    assert step_resp.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    map_event = event_with_step(stream_resp.json()["events"], "map.conceptual.lift.surface_fragments")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{map_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()
    outputs = payload["outputs"]
    assert isinstance(outputs.get("map"), dict)
    map_payload = outputs["map"]
    assert map_payload.get("status") in {"ok", "failed"}
    assert isinstance(map_payload.get("map_subgraph"), dict)
    assert isinstance(map_payload.get("structural_map"), dict)
    assert isinstance(map_payload.get("map_dna"), dict)
    assert isinstance(map_payload.get("outline_nodes"), list)
    assert isinstance(map_payload.get("root_node_id"), str)
    assert isinstance(map_payload["map_dna"].get("fingerprint"), str)
    assert map_payload.get("preview_mode_default") == "semantic_map"
    assert isinstance(map_payload.get("node_summaries"), dict)
    assert isinstance(map_payload.get("node_constituents"), dict)
    assert isinstance(map_payload.get("relationships"), list)
    assert isinstance(map_payload.get("segment_anchors"), dict)
    assert isinstance(map_payload.get("segment_candidates"), list)
    assert isinstance(map_payload.get("profile_candidates"), dict)
    assert isinstance(map_payload.get("generation_provenance"), dict)
    assert map_payload["generation_provenance"].get("provider")
    assert map_payload["generation_provenance"].get("model")

    node_summaries = map_payload["node_summaries"]
    node_constituents = map_payload["node_constituents"]
    root_node_id = str(map_payload["root_node_id"])
    assert root_node_id in node_summaries
    assert isinstance(node_summaries[root_node_id], str)
    assert node_summaries[root_node_id].strip()
    assert root_node_id in node_constituents
    assert isinstance(node_constituents[root_node_id], list)

    checks = payload.get("checks", [])
    check_status = {
        str(item.get("name")): str(item.get("status"))
        for item in checks
        if isinstance(item, dict) and item.get("name")
    }
    assert check_status.get("map_node_summary_coverage") == "pass"
    assert check_status.get("map_constituent_links_complete") == "pass"
    assert check_status.get("map_semantic_outline_nontrivial") == "pass"
    assert check_status.get("map_generation_provenance_present") == "pass"


def test_map_detail_lineage_includes_map_root_nodes_and_links(case_fixtures_root) -> None:
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    step_resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": "cmd-map-lineage-contract",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": run_id,
        },
    )
    assert step_resp.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    map_event = event_with_step(stream_resp.json()["events"], "map.conceptual.lift.surface_fragments")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{map_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()

    lineage = payload["lineage"]
    roots = lineage.get("roots", [])
    nodes = lineage.get("nodes", [])
    edges = lineage.get("edges", [])

    map_nodes = [node for node in nodes if isinstance(node, dict) and str(node.get("node_id", "")).startswith("map:")]
    assert map_nodes
    map_root = next((node for node in map_nodes if node.get("kind") == "map_root"), None)
    assert map_root is not None
    assert isinstance(map_root.get("label"), str)
    assert map_root["node_id"] in roots

    segment_map_nodes = [
        node
        for node in map_nodes
        if node.get("kind") == "map_node"
        and str(node.get("meta", {}).get("kind") or "") in {"artifact", "segment"}
    ]
    assert segment_map_nodes

    map_to_artifact_edges = [
        edge
        for edge in edges
        if isinstance(edge, dict)
        and edge.get("relation") == "map_to_artifact"
        and str(edge.get("from", "")).startswith("map:")
        and str(edge.get("to", "")).startswith("artifact:")
    ]
    assert map_to_artifact_edges

    map_contains_edges = [
        edge
        for edge in edges
        if isinstance(edge, dict)
        and edge.get("relation") == "map_contains"
        and str(edge.get("from", "")).startswith("map:")
        and str(edge.get("to", "")).startswith("map:")
    ]
    assert map_contains_edges


def test_decompose_detail_exposes_reconstruction_root_fragments(case_fixtures_root) -> None:
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    step_resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": "cmd-root-fragments",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": run_id,
        },
    )
    assert step_resp.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    decompose_event = event_with_step(stream_resp.json()["events"], "map.conceptual.lift.surface_fragments")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{decompose_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()

    decomposition = payload["outputs"]["decomposition"]
    structural_ids = payload["outputs"]["fragment_ids"]
    root_ids = decomposition["root_fragment_ids"]
    assert isinstance(root_ids, list)
    assert len(root_ids) >= len(structural_ids)
    assert set(structural_ids).issubset(set(root_ids))
    assert len(root_ids) > len(structural_ids)


def test_decompose_detail_exposes_boundary_diagnostics_with_policy_version(case_fixtures_root) -> None:
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    step_resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": "cmd-boundary-diagnostics",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": run_id,
        },
    )
    assert step_resp.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    decompose_event = event_with_step(stream_resp.json()["events"], "map.conceptual.lift.surface_fragments")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{decompose_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()

    diagnostics = payload["outputs"]["decomposition"].get("boundary_diagnostics")
    assert isinstance(diagnostics, list)
    assert diagnostics
    first = diagnostics[0]
    assert first["policy_version"] == "v1"
    assert "structural_coverage_ratio" in first
    assert first["status"] in {"good", "coarse", "failed"}
    for field in (
        "planner_provider",
        "planner_model",
        "planner_prompt_version",
        "planner_confidence",
        "planner_section_label",
        "decomposition_status",
        "decomposition_reason",
    ):
        assert field in first


def test_decompose_boundary_diagnostics_include_pdf_asset_row(case_fixtures_root) -> None:

    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    step_resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": "cmd-boundary-diagnostics-pdf-fail",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": run_id,
        },
    )
    assert step_resp.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    decompose_event = event_with_step(stream_resp.json()["events"], "map.conceptual.lift.surface_fragments")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{decompose_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()

    diagnostics = payload["outputs"]["decomposition"].get("boundary_diagnostics")
    assert isinstance(diagnostics, list)

    pdf_rows = [
        row for row in diagnostics
        if isinstance(row, dict) and row.get("mime_type") == "application/pdf"
    ]
    assert pdf_rows, "Expected PDF row in boundary diagnostics"
    pdf_row = pdf_rows[0]
    assert pdf_row["status"] in {"good", "coarse", "failed"}
    assert pdf_row.get("decomposition_status") in {"mapped", "failed", "decomposed"}


def test_embed_decomposed_detail_includes_projection_and_pairwise_heatmap(case_fixtures_root) -> None:
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    for idx in range(5):
        embed_resp = client.post(
            f"/benchmarks/runs/{run_id}/control",
            json={
                "command_id": f"cmd-embed-shape-step-{idx}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": run_id,
            },
        )
        assert embed_resp.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    assert stream_resp.status_code == 200
    embed_event = event_with_step(stream_resp.json()["events"], "map.conceptual.embed.discovery_index")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{embed_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()
    outputs = payload["outputs"]

    projection = outputs.get("embedding_projection")
    assert isinstance(projection, dict)
    assert projection.get("method") == "pca_2d"
    points = projection.get("points")
    assert isinstance(points, list)
    assert points
    first = points[0]
    assert isinstance(first.get("fragment_id"), str)
    assert isinstance(first.get("x"), (int, float))
    assert isinstance(first.get("y"), (int, float))
    assert "cluster_id" in first

    pairwise = outputs.get("pairwise_similarity")
    assert isinstance(pairwise, dict)
    fragment_ids = pairwise.get("fragment_ids")
    matrix = pairwise.get("matrix")
    assert isinstance(fragment_ids, list)
    assert isinstance(matrix, list)
    assert len(matrix) == len(fragment_ids)
    assert len(matrix) > 0
    for row in matrix:
        assert isinstance(row, list)
        assert len(row) == len(fragment_ids)

    for i in range(len(matrix)):
        diag = matrix[i][i]
        assert isinstance(diag, (int, float))
        assert abs(float(diag) - 1.0) < 1e-6
        for j in range(len(matrix)):
            assert abs(float(matrix[i][j]) - float(matrix[j][i])) < 1e-6

    debug = outputs.get("embedding_debug")
    assert isinstance(debug, dict)
    for key in (
        "expected_count",
        "embedded_count",
        "missing_fragment_ids",
        "singleton_clusters",
        "threshold",
    ):
        assert key in debug


def test_embed_lifted_detail_includes_pairwise_heatmap_and_ir_lineage(case_fixtures_root) -> None:
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    for index in range(7):
        step_resp = client.post(
            f"/benchmarks/runs/{run_id}/control",
            json={
                "command_id": f"cmd-embed-lifted-shape-step-{index + 1}",
                "action": "next_step",
                "pipeline_id": "compression-rerender/v1",
                "pipeline_run_id": run_id,
            },
        )
        assert step_resp.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    assert stream_resp.status_code == 200
    embed_event = event_with_step(stream_resp.json()["events"], "map.reconstructable.embed")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{embed_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()
    outputs = payload["outputs"]

    pairwise = outputs.get("pairwise_similarity")
    assert isinstance(pairwise, dict)
    fragment_ids = pairwise.get("fragment_ids")
    matrix = pairwise.get("matrix")
    assert isinstance(fragment_ids, list)
    assert isinstance(matrix, list)
    assert len(matrix) == len(fragment_ids)
    assert len(matrix) > 0
    for row in matrix:
        assert isinstance(row, list)
        assert len(row) == len(fragment_ids)

    for i in range(len(matrix)):
        diag = matrix[i][i]
        assert isinstance(diag, (int, float))
        assert abs(float(diag) - 1.0) < 1e-6
        for j in range(len(matrix)):
            assert abs(float(matrix[i][j]) - float(matrix[j][i])) < 1e-6

    debug = outputs.get("embedding_debug")
    assert isinstance(debug, dict)
    for key in (
        "expected_count",
        "embedded_count",
        "missing_fragment_ids",
        "singleton_clusters",
        "threshold",
    ):
        assert key in debug

    lineage = payload.get("lineage")
    assert isinstance(lineage, dict)
    nodes = lineage.get("nodes")
    assert isinstance(nodes, list)
    ir_nodes = [node for node in nodes if isinstance(node, dict) and node.get("kind") == "ir"]
    assert len(ir_nodes) > 0
    ir_meta = ir_nodes[0].get("meta") if isinstance(ir_nodes[0], dict) else None
    assert isinstance(ir_meta, dict)
    ir_preview = ir_meta.get("value_preview") if isinstance(ir_meta, dict) else None
    assert isinstance(ir_preview, dict)
    relation_label = ir_preview.get("relation_type") or ir_preview.get("predicate")
    assert isinstance(relation_label, str)
    has_slot_bindings = isinstance(ir_preview.get("slot_bindings"), dict)
    has_spo_shape = all(key in ir_preview for key in ("subject", "object"))
    assert has_slot_bindings or has_spo_shape


def test_decompose_detail_exposes_agent_review_and_elicitation(case_fixtures_root) -> None:
    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    step_resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": "cmd-agent-review",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": run_id,
        },
    )
    assert step_resp.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    decompose_event = event_with_step(stream_resp.json()["events"], "map.conceptual.lift.surface_fragments")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{decompose_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()

    map_outputs = payload["outputs"]["map"]
    assert map_outputs["agent_review"]["decision"] in {"accept", "warn", "reject"}
    assert map_outputs["agent_review"]["executor_id"] == "executor://agent-env-primary"
    assert map_outputs["elicitation"]["approval_mode"] == "human_required"
    assert map_outputs["elicitation"]["approval_request"]["details"]["kind"] == "parse_review_elicitation"
    assert map_outputs["agent_spec"]["logical_name"] == "parse-review-agent"


def test_decompose_hard_fails_when_reconstruction_gate_fails(case_fixtures_root, monkeypatch) -> None:
    from ikam.forja import debug_execution

    def _broken_map_generation(*_args, **_kwargs):
        raise RuntimeError("forced map generation failure")

    monkeypatch.setattr(debug_execution, "_invoke_mcp_map_generation", _broken_map_generation)

    client = TestClient(app)
    run_resp = client.post(
        "/benchmarks/run",
        params={"case_ids": "s-construction-v01", "include_evaluation": "false"},
    )
    assert run_resp.status_code == 200
    run_id = run_resp.json()["runs"][0]["run_id"]

    step_resp = client.post(
        f"/benchmarks/runs/{run_id}/control",
        json={
            "command_id": "cmd-hard-fail-reconstruction",
            "action": "next_step",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": run_id,
        },
    )
    assert step_resp.status_code == 200

    stream_resp = client.get(
        f"/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": "compression-rerender/v1", "pipeline_run_id": run_id},
    )
    decompose_event = event_with_step(stream_resp.json()["events"], "map.conceptual.lift.surface_fragments")
    assert decompose_event["status"] == "failed"
    assert "structural map generation failed" in (decompose_event.get("error") or {}).get("reason", "")

    detail_resp = client.get(f"/benchmarks/runs/{run_id}/debug-step/{decompose_event['step_id']}/detail")
    assert detail_resp.status_code == 200
    payload = detail_resp.json()
    checks = payload.get("checks") or []
    nonempty = next((item for item in checks if item.get("name") == "decomposition_nonempty"), None)
    assert isinstance(nonempty, dict)
    assert nonempty.get("status") == "fail"
