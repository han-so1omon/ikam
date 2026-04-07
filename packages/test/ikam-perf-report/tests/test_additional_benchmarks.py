from ikam_perf_report.benchmarks.runner import (
    run_benchmark,
    run_interacciones_benchmark,
    run_merge_benchmark,
)


def test_interacciones_benchmark_emits_decisions(case_fixtures_root):
    result = run_interacciones_benchmark(case_ids="s-construction-v01")
    assert result["decisions"]


def test_merge_benchmark_emits_synergy_risk(case_fixtures_root, monkeypatch):
    from ikam_perf_report.benchmarks import runner

    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": []},
    )
    run_output = run_benchmark(case_ids="s-construction-v01")
    graph_id = run_output["runs"][0]["graph_id"]
    result = run_merge_benchmark(graph_ids=graph_id)
    assert result["synergy_risk"]
    assert result["proposed_edges"]
    assert result["proposed_relational_fragments"]


def test_merge_apply_updates_graph_state(case_fixtures_root, monkeypatch):
    from ikam_perf_report.benchmarks import runner
    from ikam_perf_report.benchmarks.store import STORE

    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": []},
    )
    run_output = run_benchmark(case_ids="s-construction-v01")
    graph_id = run_output["runs"][0]["graph_id"]
    result = run_merge_benchmark(graph_ids=graph_id, apply=True)
    graph = STORE.get_graph(graph_id)
    assert result["applied"] is True
    assert result["apply_result"]["edge_updates"] >= 1
    assert graph is not None
    assert graph.relational_fragments
