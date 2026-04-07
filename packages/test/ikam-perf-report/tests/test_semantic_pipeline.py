import os

import pytest

from ikam_perf_report.benchmarks.semantic_pipeline import run_semantic_pipeline


def test_semantic_pipeline_returns_entities_and_relations():
    api_key = os.getenv("OPENAI_API_KEY", "")
    if not api_key or api_key == "dummy":
        pytest.skip("OPENAI_API_KEY unavailable for semantic pipeline integration test")
    result = run_semantic_pipeline("Revenue grows faster than costs.")
    assert result["entities"]
    assert result["relations"]
