from fastapi.testclient import TestClient
from unittest.mock import AsyncMock, MagicMock, patch

from ikam_perf_report.benchmarks import runner
from ikam_perf_report.main import app


def test_graph_summary_endpoint_empty_store():
    """GET /graph/summary returns zero counts when no graph has been seeded."""
    client = TestClient(app)
    resp = client.get("/graph/summary")
    assert resp.status_code == 200
    body = resp.json()
    for key in ("nodes", "edges", "semantic_entities", "semantic_relations"):
        assert key in body, f"missing key: {key}"
        assert isinstance(body[key], int) and body[key] >= 0, f"{key} must be int >= 0"


def test_graph_summary_endpoint_with_graph(case_fixtures_root, monkeypatch):
    """GET /graph/summary returns graph_id and correct counts after seeding via legacy runner."""
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "entity-1"}], "relations": [{"id": "rel-1"}]},
    )
    result = runner.run_benchmark_legacy(case_ids="s-construction-v01")
    graph_id = result["runs"][0]["graph_id"]

    client = TestClient(app)
    resp = client.get("/graph/summary")
    assert resp.status_code == 200
    body = resp.json()

    assert body.get("graph_id") == graph_id, "graph_id must match the seeded graph"
    for key in ("nodes", "edges", "semantic_entities", "semantic_relations"):
        assert key in body, f"missing key: {key}"
        assert isinstance(body[key], int) and body[key] >= 0, f"{key} must be int >= 0"


def test_cases_endpoint_returns_case_metadata(case_fixtures_root):
    client = TestClient(app)
    resp = client.get("/benchmarks/cases")
    assert resp.status_code == 200
    body = resp.json()
    assert "cases" in body
    assert body["cases"]
    assert body["cases"][0]["case_id"] == "s-construction-v01"
    assert body["cases"][0]["domain"] == "construction"
    assert body["cases"][0]["size_tier"] == "s"


def test_run_endpoint_accepts_case_ids(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": []},
    )
    client = TestClient(app)
    resp = client.post("/benchmarks/run?case_ids=s-construction-v01")
    assert resp.status_code == 200
    body = resp.json()
    assert body["runs"]
    assert body["runs"][0]["graph_id"].startswith("s-construction-v01#")


def test_run_endpoint_initializes_debug_stream_workflow(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": []},
    )
    client = TestClient(app)
    run_resp = client.post("/benchmarks/run?case_ids=s-construction-v01")
    assert run_resp.status_code == 200
    run_payload = run_resp.json()["runs"][0]

    debug_resp = client.get(
        f"/benchmarks/runs/{run_payload['run_id']}/debug-stream",
        params={
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": run_payload["run_id"],
        },
    )
    assert debug_resp.status_code == 200
    body = debug_resp.json()
    assert body["status"] == "ok"
    assert body["execution_mode"] == "manual"
    assert body["execution_state"] == "paused"
    assert body["control_availability"] == {
        "can_resume": True,
        "can_next_step": True,
    }
    assert len(body["events"]) >= 1
    assert body["events"][0]["step_name"] == "init.initialize"
    assert body["events"][0]["attempt_index"] == 1


def test_run_endpoint_can_skip_evaluation_for_fast_debug_init(case_fixtures_root, monkeypatch):
    """POST /benchmarks/run returns prepare-only payload with debug_state.

    After the prepare-only refactor, run no longer returns evaluation data.
    Instead it returns debug_state at prepare_case/paused. The debug-stream
    endpoint still works and returns the initial prepare_case event.
    """
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": []},
    )
    client = TestClient(app)
    run_resp = client.post("/benchmarks/run?case_ids=s-construction-v01&include_evaluation=false")
    assert run_resp.status_code == 200
    run_payload = run_resp.json()["runs"][0]

    # Prepare-only: debug_state present, no evaluation key
    assert "debug_state" in run_payload
    assert run_payload["debug_state"]["current_step_name"] == "init.initialize"
    assert run_payload["debug_state"]["execution_state"] == "paused"

    debug_resp = client.get(
        f"/benchmarks/runs/{run_payload['run_id']}/debug-stream",
        params={
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": run_payload["run_id"],
        },
    )
    assert debug_resp.status_code == 200
    body = debug_resp.json()
    assert body["status"] == "ok"
    assert body["events"][0]["step_name"] == "init.initialize"


def test_runs_endpoint_returns_newest_first(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": []},
    )
    client = TestClient(app)
    first = client.post("/benchmarks/run?case_ids=s-construction-v01").json()["runs"][0]["run_id"]
    second = client.post("/benchmarks/run?case_ids=s-construction-v01").json()["runs"][0]["run_id"]

    listed = client.get("/benchmarks/runs")
    assert listed.status_code == 200
    runs = listed.json()
    assert runs
    assert runs[0]["run_id"] == second
    assert any(run["run_id"] == first for run in runs)


def test_ingest_poc_endpoint(case_fixtures_root, monkeypatch):
    from ikam_perf_report.api import benchmarks as benchmarks_api

    monkeypatch.setattr(
        benchmarks_api,
        "run_staging_normalize_promote_enrich_poc",
        lambda: {
            "staging": {"session_id": "sess-1", "row_count": 4, "valid": True},
            "promotion": {"first_promoted": 4, "second_promoted": 0, "permanent_count": 4},
            "metrics": {"cas_hits_delta": 4, "cas_misses_delta": 4, "cas_hit_rate": 0.5},
            "enrichment": {"entity_fragments": 2, "relation_fragments": 1},
        },
    )
    client = TestClient(app)
    resp = client.post("/benchmarks/ingest-poc")
    assert resp.status_code == 200
    body = resp.json()
    assert body["staging"]["valid"] is True
    assert body["promotion"]["first_promoted"] >= 1
    assert body["metrics"]["cas_misses_delta"] >= 1


def test_ingest_poc_endpoint_contract(case_fixtures_root, monkeypatch):
    from ikam_perf_report.benchmarks import runner
    from unittest.mock import AsyncMock, MagicMock, patch
    import json

    async def mock_call_model(prompt: str, model: str, temperature: float):
        if "atomic concepts" in prompt:
            return MagicMock(output=json.dumps(["concept 1", "concept 2"]))
        elif "entities and relations" in prompt:
            return MagicMock(output=json.dumps({
                "entities": ["entity 1", "entity 2"],
                "relations": [{"source": "entity 1", "target": "entity 2", "predicate": "links to"}]
            }))
        return MagicMock(output="[]")

    mock_ai_client = MagicMock()
    mock_ai_client.call_model = AsyncMock(side_effect=mock_call_model)

    client = TestClient(app)
    with patch(
        "ikam_perf_report.benchmarks.runner.UnifiedCallModelClient.from_env",
        return_value=mock_ai_client,
    ):
        resp = client.post("/benchmarks/ingest-poc")
        assert resp.status_code == 200
        body = resp.json()

    assert set(body.keys()) >= {"staging", "normalization", "enrichment", "promotion", "metrics"}
    assert body["staging"]["valid"] is True
    assert body["staging"]["row_count"] >= 1
    assert body["normalization"]["concept_fragments"] >= 1
    assert body["enrichment"]["entity_fragments"] >= 1
    assert body["enrichment"]["relation_fragments"] >= 1
    assert body["promotion"]["permanent_count"] == body["staging"]["row_count"]
    assert 0.0 <= body["metrics"]["cas_hit_rate"] <= 1.0


def test_graph_nodes_and_edges_include_explainability_fields(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "entity-1"}], "relations": [{"id": "relation-1"}]},
    )
    # Seed graph via legacy runner (prepare-only run_benchmark doesn't build graph)
    result = runner.run_benchmark_legacy(case_ids="s-construction-v01")
    graph_id = result["runs"][0]["graph_id"]

    client = TestClient(app)
    nodes_resp = client.get("/graph/nodes", params={"graph_id": graph_id})
    edges_resp = client.get("/graph/edges", params={"graph_id": graph_id})
    assert nodes_resp.status_code == 200
    assert edges_resp.status_code == 200

    nodes = nodes_resp.json()
    edges = edges_resp.json()
    assert isinstance(nodes, list) and nodes
    assert isinstance(edges, list)

    first_node = nodes[0]
    assert "kind" in first_node
    assert "meta" in first_node
    assert "origin" in first_node["meta"]
    assert "run_id" in first_node["meta"]
    assert "semantic_entity_ids" in first_node["meta"]

    if edges:
        first_edge = edges[0]
        assert "kind" in first_edge
        assert "meta" in first_edge
        assert "origin" in first_edge["meta"]
        assert "decision_ref" in first_edge["meta"]


def test_graph_nodes_relation_tags(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {
            "entities": [{"id": "entity-1"}],
            "relations": [{"id": "relation-1", "kind": "evaluated"}],
        },
    )
    # Seed graph via legacy runner (prepare-only run_benchmark doesn't build graph)
    result = runner.run_benchmark_legacy(case_ids="s-construction-v01")
    graph_id = result["runs"][0]["graph_id"]

    client = TestClient(app)
    nodes_resp = client.get("/graph/nodes", params={"graph_id": graph_id})
    assert nodes_resp.status_code == 200
    nodes = nodes_resp.json()
    assert isinstance(nodes, list) and nodes

    first_node = nodes[0]
    assert "meta" in first_node
    meta = first_node["meta"]
    assert "semantic_relation_ids" in meta
    assert "semantic_relation_labels" in meta
    assert meta["semantic_relation_ids"]
    assert meta["semantic_relation_labels"]
    assert meta["semantic_relation_labels"][0] == "evaluated"


def test_graph_wiki_generate_and_fetch(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "entity-1"}], "relations": [{"id": "relation-1"}]},
    )
    client = TestClient(app)
    run_resp = client.post("/benchmarks/run?case_ids=s-construction-v01")
    assert run_resp.status_code == 200
    graph_id = run_resp.json()["runs"][0]["graph_id"]

    gen_resp = client.post("/graph/wiki/generate", params={"graph_id": graph_id})
    assert gen_resp.status_code == 200
    wiki = gen_resp.json()
    assert wiki["graph_id"] == graph_id
    assert "sections" in wiki
    assert "ikam_breakdown" in wiki
    assert wiki["ikam_breakdown"]["title"] == "IKAM Breakdown"
    assert "generation_provenance" in wiki["ikam_breakdown"]
    assert wiki["ikam_breakdown"]["generation_provenance"]["model_id"]
    assert wiki["ikam_breakdown"]["generation_provenance"]["harness_id"]

    get_resp = client.get("/graph/wiki", params={"graph_id": graph_id})
    assert get_resp.status_code == 200
    fetched = get_resp.json()
    assert fetched["graph_id"] == graph_id
    assert fetched["ikam_breakdown"]["title"] == "IKAM Breakdown"


def test_graph_search_contract(case_fixtures_root, monkeypatch):
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot
    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="s-construction-v01",
            case_id="s-construction-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-construction-v01",
                fragments=[
                    {
                        "id": "f1",
                        "label": "Unit Economics Data",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report",
                            "fragment_id": "f1",
                            "profile": "modelado/reasoning@1",
                            "statement": {"subject": "Unit Economics Data"},
                            "evidence_refs": [{"fragment_id": "f1"}],
                        },
                    }
                ],
                nodes=[{"id": "f1", "label": "Unit Economics Data", "type": "fragment"}],
                edges=[],
            ),
        )
    )
    graph_id = "s-construction-v01"

    mock_ai_response = MagicMock()
    mock_ai_response.text = '{"interpretation": "Unit economics are positive [f1].", "attribution": [{"claim": "positive", "fragment_ids": ["f1"]}]}'

    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)

    client = TestClient(app)
    with patch(
        "ikam_perf_report.api.graph.create_ai_client_from_env",
        return_value=mock_ai_client,
    ):
        resp = client.post(
            "/graph/search",
            json={"query": "unit economics", "graph_id": graph_id},
        )
        assert resp.status_code == 200
        body = resp.json()
        assert body["query"] == "unit economics"
        assert body["query_type"] == "sqi-framework"
        assert "results" in body
        assert "groups" in body
        assert "interpretation" in body
        assert "attribution" in body
        assert body["interpretation"] == "Unit economics are positive [f1]."
        assert len(body["attribution"]) == 1

        assert body["results"]
        first_result = body["results"][0]
        assert "node_id" in first_result
        assert "confidence" in first_result
        assert first_result["node_id"] == "f1"

        assert body["scores"]
        first_score = body["scores"][0]
        assert first_score["node_id"] == "f1"
        assert first_score["confidence"] == 1.0


def test_graph_search_blends_query_heads(case_fixtures_root, monkeypatch):
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot
    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="s-construction-v01",
            case_id="s-construction-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-construction-v01",
                fragments=[
                    {
                        "id": "alpha-node",
                        "label": "alpha signal",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report",
                            "fragment_id": "alpha-node",
                            "profile": "modelado/reasoning@1",
                            "statement": {"subject": "alpha signal"},
                            "evidence_refs": [{"fragment_id": "alpha-node"}],
                        },
                    },
                    {
                        "id": "beta-node",
                        "label": "beta signal",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report",
                            "fragment_id": "beta-node",
                            "profile": "modelado/reasoning@1",
                            "statement": {"subject": "beta signal"},
                            "evidence_refs": [{"fragment_id": "beta-node"}],
                        },
                    },
                ],
                nodes=[
                    {"id": "alpha-node", "label": "alpha signal", "type": "fragment"},
                    {"id": "beta-node", "label": "beta signal", "type": "fragment"},
                ],
                edges=[],
            ),
        )
    )
    graph_id = "s-construction-v01"

    mock_ai_response = MagicMock()
    mock_ai_response.text = '{"interpretation": "Blended signals [alpha-node].", "attribution": [{"claim": "alpha", "fragment_ids": ["alpha-node"]}]}'
    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)

    client = TestClient(app)
    with patch(
        "ikam_perf_report.api.graph.create_ai_client_from_env",
        return_value=mock_ai_client,
    ):
        resp = client.post(
            "/graph/search", json={"query": "alpha", "graph_id": graph_id}
        )
        assert resp.status_code == 200
        body = resp.json()
        assert body["results"]
        assert body["results"][0]["node_id"] == "alpha-node"
        assert body["groups"][0]["label"] == "Discovered Evidence"


def test_graph_search_returns_weights(case_fixtures_root, monkeypatch):
    """SQI Framework does not return weights, checking for query_type instead."""
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot
    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="s-construction-v01",
            case_id="s-construction-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-construction-v01",
                fragments=[
                    {
                        "id": "alpha-node",
                        "label": "alpha signal",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report",
                            "fragment_id": "alpha-node",
                            "profile": "modelado/reasoning@1",
                            "statement": {"subject": "alpha signal"},
                            "evidence_refs": [{"fragment_id": "alpha-node"}],
                        },
                    }
                ],
                nodes=[{"id": "alpha-node", "label": "alpha signal", "type": "fragment"}],
                edges=[],
            ),
        )
    )
    graph_id = "s-construction-v01"

    mock_ai_response = MagicMock()
    mock_ai_response.text = '{"interpretation": "alpha [alpha-node]", "attribution": [{"claim": "alpha", "fragment_ids": ["alpha-node"]}]}'
    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)

    client = TestClient(app)
    with patch(
        "ikam_perf_report.api.graph.create_ai_client_from_env",
        return_value=mock_ai_client,
    ):
        resp = client.post(
            "/graph/search", json={"query": "alpha", "graph_id": graph_id}
        )
        assert resp.status_code == 200
        body = resp.json()
        assert body["query_type"] == "sqi-framework"


def test_graph_search_business_identity_query_uses_generic_pipeline(
    case_fixtures_root, monkeypatch
):
    """Contract: All queries use the SQI pipeline."""
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot
    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="s-construction-v01",
            case_id="s-construction-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-construction-v01",
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
                nodes=[{"id": "f1", "label": "business identity", "type": "fragment"}],
                edges=[],
            ),
        )
    )
    graph_id = "s-construction-v01"

    mock_ai_response = MagicMock()
    mock_ai_response.text = '{"interpretation": "This is a construction business [f1].", "attribution": [{"claim": "construction business", "fragment_ids": ["f1"]}]}'
    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)

    client = TestClient(app)
    with patch(
        "ikam_perf_report.api.graph.create_ai_client_from_env",
        return_value=mock_ai_client,
    ):
        resp = client.post(
            "/graph/search",
            json={
                "query": "business identity",
                "graph_id": graph_id,
            },
        )
        assert resp.status_code == 200
        body = resp.json()

        assert body["query_type"] == "sqi-framework"
        assert body["results"]
        assert body["interpretation"]


def test_graph_search_rejects_missing_anchors(case_fixtures_root):
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot

    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="s-construction-v01",
            case_id="s-construction-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-construction-v01",
                fragments=[
                    {
                        "id": "f1",
                        "label": "business identity",
                        "mime_type": "text/markdown",
                        "value": {"text": "construction business"},
                    }
                ],
                nodes=[{"id": "f1", "label": "business identity", "type": "fragment"}],
                edges=[],
            ),
        )
    )

    client = TestClient(app)
    resp = client.post(
        "/graph/search",
        json={"query": "does-not-match-anything", "graph_id": "s-construction-v01"},
    )
    assert resp.status_code == 422


def test_graph_search_rejects_interpretation_without_bracketed_citations(case_fixtures_root):
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot

    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="s-construction-v01",
            case_id="s-construction-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-construction-v01",
                fragments=[
                    {
                        "id": "f1",
                        "label": "unit economics",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report",
                            "fragment_id": "f1",
                            "profile": "modelado/reasoning@1",
                            "statement": {"subject": "unit economics"},
                            "evidence_refs": [{"fragment_id": "f1"}],
                        },
                    }
                ],
                nodes=[{"id": "f1", "label": "unit economics", "type": "proposition"}],
                edges=[],
            ),
        )
    )

    mock_ai_response = MagicMock()
    mock_ai_response.text = '{"interpretation": "No citation here.", "attribution": [{"claim": "c", "fragment_ids": ["f1"]}]}'
    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)

    client = TestClient(app)
    with patch("ikam_perf_report.api.graph.create_ai_client_from_env", return_value=mock_ai_client):
        resp = client.post(
            "/graph/search",
            json={"query": "unit economics", "graph_id": "s-construction-v01"},
        )
    assert resp.status_code == 422


def test_graph_search_fails_on_unauthorized_scope_traversal(case_fixtures_root):
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot

    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="s-construction-v01",
            case_id="s-construction-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-construction-v01",
                fragments=[
                    {
                        "id": "f1",
                        "label": "unit economics",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report",
                            "fragment_id": "f1",
                            "profile": "modelado/reasoning@1",
                            "statement": {"subject": "unit economics"},
                            "evidence_refs": [{"fragment_id": "f1"}],
                        },
                    },
                    {
                        "id": "f2",
                        "label": "dev-only node",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report",
                            "fragment_id": "f2",
                            "profile": "modelado/reasoning@1",
                            "statement": {"subject": "dev-only"},
                            "evidence_refs": [{"fragment_id": "f2"}],
                        },
                    },
                ],
                nodes=[
                    {"id": "f1", "label": "unit economics", "type": "proposition"},
                    {"id": "f2", "label": "dev-only node", "type": "proposition"},
                ],
                edges=[
                    {
                        "source": "f1",
                        "target": "f2",
                        "kind": "reference",
                        "meta": {"env_type": "dev", "env_id": "run-abc"},
                    }
                ],
            ),
        )
    )

    mock_ai_response = MagicMock()
    mock_ai_response.text = '{"interpretation": "Unit economics [f1].", "attribution": [{"claim": "c", "fragment_ids": ["f1"]}]}'
    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)

    client = TestClient(app)
    with patch("ikam_perf_report.api.graph.create_ai_client_from_env", return_value=mock_ai_client):
        resp = client.post(
            "/graph/search",
            json={
                "query": "unit economics",
                "graph_id": "s-construction-v01",
                "env_type": "committed",
                "env_id": "prod",
                "max_hops": 2,
            },
        )
    assert resp.status_code == 422


def test_graph_search_rejects_empty_attribution_payload(case_fixtures_root):
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot

    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="s-construction-v01",
            case_id="s-construction-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-construction-v01",
                fragments=[
                    {
                        "id": "f1",
                        "label": "unit economics",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report",
                            "fragment_id": "f1",
                            "profile": "modelado/reasoning@1",
                            "statement": {"subject": "unit economics"},
                            "evidence_refs": [{"fragment_id": "f1"}],
                        },
                    }
                ],
                nodes=[{"id": "f1", "label": "unit economics", "type": "proposition"}],
                edges=[],
            ),
        )
    )

    mock_ai_response = MagicMock()
    mock_ai_response.text = '{"interpretation": "Unit economics [f1].", "attribution": []}'
    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)

    client = TestClient(app)
    with patch("ikam_perf_report.api.graph.create_ai_client_from_env", return_value=mock_ai_client):
        resp = client.post(
            "/graph/search",
            json={"query": "unit economics", "graph_id": "s-construction-v01"},
        )
    assert resp.status_code == 422


def test_graph_search_rejects_unknown_strategy(case_fixtures_root):
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot

    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="s-construction-v01",
            case_id="s-construction-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-construction-v01",
                fragments=[
                    {
                        "id": "f1",
                        "label": "unit economics",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report",
                            "fragment_id": "f1",
                            "profile": "modelado/reasoning@1",
                            "statement": {"subject": "unit economics"},
                            "evidence_refs": [{"fragment_id": "f1"}],
                        },
                    }
                ],
                nodes=[{"id": "f1", "label": "unit economics", "type": "proposition"}],
                edges=[],
            ),
        )
    )

    mock_ai_response = MagicMock()
    mock_ai_response.text = '{"interpretation": "Unit economics [f1].", "attribution": [{"claim": "c", "fragment_ids": ["f1"]}]}'
    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)

    client = TestClient(app)
    with patch("ikam_perf_report.api.graph.create_ai_client_from_env", return_value=mock_ai_client):
        resp = client.post(
            "/graph/search",
            json={
                "query": "unit economics",
                "graph_id": "s-construction-v01",
                "search_strategy": "does-not-exist",
            },
        )
    assert resp.status_code == 422


def test_graph_search_rejects_unsupported_hydration_payload(case_fixtures_root):
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot

    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="s-construction-v01",
            case_id="s-construction-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-construction-v01",
                fragments=[
                    {
                        "id": "f1",
                        "label": "unit economics",
                        "mime_type": "text/markdown",
                        "value": {"text": "unit economics"},
                    }
                ],
                nodes=[{"id": "f1", "label": "unit economics", "type": "fragment"}],
                edges=[],
            ),
        )
    )

    mock_ai_response = MagicMock()
    mock_ai_response.text = '{"interpretation": "Unit economics [f1].", "attribution": [{"claim": "c", "fragment_ids": ["f1"]}]}'
    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)

    client = TestClient(app)
    with patch("ikam_perf_report.api.graph.create_ai_client_from_env", return_value=mock_ai_client):
        resp = client.post(
            "/graph/search",
            json={"query": "unit economics", "graph_id": "s-construction-v01"},
        )
    assert resp.status_code == 422
    assert "Hydration failed: unsupported mime_type" in str(resp.json().get("detail", ""))


def test_graph_search_rejects_unlinked_claim_attribution(case_fixtures_root):
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot

    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="s-construction-v01",
            case_id="s-construction-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-construction-v01",
                fragments=[
                    {
                        "id": "f1",
                        "label": "unit economics",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report",
                            "fragment_id": "f1",
                            "profile": "modelado/reasoning@1",
                            "statement": {"subject": "unit economics"},
                            "evidence_refs": [{"fragment_id": "f1"}],
                        },
                    }
                ],
                nodes=[{"id": "f1", "label": "unit economics", "type": "proposition"}],
                edges=[],
            ),
        )
    )

    mock_ai_response = MagicMock()
    mock_ai_response.text = '{"interpretation": "Unit economics improved [f1].", "attribution": [{"claim": "Margins improved", "fragment_ids": ["f1"]}]}'
    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)

    client = TestClient(app)
    with patch("ikam_perf_report.api.graph.create_ai_client_from_env", return_value=mock_ai_client):
        resp = client.post(
            "/graph/search",
            json={"query": "unit economics", "graph_id": "s-construction-v01"},
        )
    assert resp.status_code == 422


def test_graph_search_rejects_invalid_supported_mime_payload_with_detail(case_fixtures_root):
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot

    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-1",
            project_id="s-construction-v01",
            case_id="s-construction-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-construction-v01",
                fragments=[
                    {
                        "id": "f1",
                        "label": "unit economics",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report"
                        },
                    }
                ],
                nodes=[{"id": "f1", "label": "unit economics", "type": "proposition"}],
                edges=[],
            ),
        )
    )

    mock_ai_response = MagicMock()
    mock_ai_response.text = '{"interpretation": "Unit economics [f1].", "attribution": [{"claim": "c", "fragment_ids": ["f1"]}]}'
    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)

    client = TestClient(app)
    with patch("ikam_perf_report.api.graph.create_ai_client_from_env", return_value=mock_ai_client):
        resp = client.post(
            "/graph/search",
            json={"query": "unit economics", "graph_id": "s-construction-v01"},
        )

    assert resp.status_code == 422
    detail = str(resp.json().get("detail", ""))
    assert "Hydration failed: invalid payload" in detail


def test_history_endpoints_expose_refs_commits_and_semantic_graph() -> None:
    from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot

    STORE.reset()
    run_id = "run-history-1"
    STORE.add_run(
        BenchmarkRunRecord(
            run_id=run_id,
            project_id="proj-history-1",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-history-1", fragments=[]),
        )
    )
    STORE.set_debug_runtime_context(
        run_id,
        {
            "history": {
                "commit_entries": [
                    {
                        "id": "commit-1",
                        "mime_type": "application/ikam-structured-data+json",
                        "profile": "modelado/commit-entry@1",
                        "content": {
                            "ref": "refs/heads/main",
                            "parents": [],
                            "commit_policy": "semantic_relations_only",
                            "commit_item_ids": ["prop-1"],
                            "commit_id": "commit-1",
                        },
                    }
                ],
                "ref_heads": {
                    "refs/heads/main": {
                        "ref": "refs/heads/main",
                        "commit_id": "commit-1",
                    }
                },
                "commit_items": {
                    "commit-1": [
                        {"id": "prop-1", "kind": "proposition", "value": {"id": "prop-1"}}
                    ]
                },
            }
        },
    )

    client = TestClient(app)
    refs_resp = client.get("/history/refs", params={"run_id": run_id})
    assert refs_resp.status_code == 200
    refs = refs_resp.json()["refs"]
    assert refs[0]["ref"] == "refs/heads/main"
    assert refs[0]["commit_id"] == "commit-1"

    commits_resp = client.get("/history/commits", params={"run_id": run_id})
    assert commits_resp.status_code == 200
    commits = commits_resp.json()["commits"]
    assert commits[0]["id"] == "commit-1"

    commit_detail_resp = client.get("/history/commits/commit-1", params={"run_id": run_id})
    assert commit_detail_resp.status_code == 200
    assert commit_detail_resp.json()["commit"]["id"] == "commit-1"

    semantic_graph_resp = client.get("/history/commits/commit-1/semantic-graph", params={"run_id": run_id})
    assert semantic_graph_resp.status_code == 200
    graph_payload = semantic_graph_resp.json()
    assert graph_payload["commit_id"] == "commit-1"
    assert graph_payload["nodes"][0]["id"] == "prop-1"
