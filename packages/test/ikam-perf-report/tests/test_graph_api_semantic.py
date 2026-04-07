from fastapi.testclient import TestClient

from ikam_perf_report.benchmarks import runner
from ikam_perf_report.main import app


def test_graph_summary_includes_semantic_counts(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": [{"id": "r1"}]},
    )
    run_output = runner.run_benchmark(case_ids="s-construction-v01")
    graph_id = run_output["runs"][0]["graph_id"]
    client = TestClient(app)
    resp = client.get("/graph/summary", params={"graph_id": graph_id})
    assert "semantic_entities" in resp.json()
