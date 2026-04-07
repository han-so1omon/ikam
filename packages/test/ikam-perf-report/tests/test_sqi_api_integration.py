import pytest
from fastapi.testclient import TestClient
from unittest.mock import AsyncMock, MagicMock, patch
import os

from ikam_perf_report.main import app
from ikam_perf_report.benchmarks.store import STORE, BenchmarkRunRecord, GraphSnapshot

def test_sqi_search_endpoint_integration():
    """
    Test that the /graph/search endpoint correctly invokes the SQI pipeline.
    """
    STORE.reset()
    STORE.add_run(
        BenchmarkRunRecord(
            run_id="run-retail-1",
            project_id="s-local-retail-v01",
            case_id="s-local-retail-v01",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(
                graph_id="s-local-retail-v01",
                fragments=[
                    {
                        "id": "f1",
                        "label": "Retail Performance Node",
                        "mime_type": "application/ikam-proposition",
                        "value": {
                            "artifact_id": "perf-report",
                            "fragment_id": "f1",
                            "profile": "modelado/reasoning@1",
                            "statement": {"subject": "Retail Performance Node"},
                            "evidence_refs": [{"fragment_id": "f1"}]
                        },
                        "meta": {
                            "env_type": "committed",
                            "env_id": "prod"
                        }
                    }
                ],
                nodes=[
                    {"id": "f1", "label": "Retail Performance Node", "type": "proposition"}
                ],
                edges=[]
            )
        )
    )

    client = TestClient(app)
    
    # Mock AI response
    mock_ai_response = MagicMock()
    mock_ai_response.text = '{"interpretation": "Test interpretation [f1].", "attribution": [{"claim": "test", "fragment_ids": ["f1"]}]}'
    
    mock_ai_client = MagicMock()
    mock_ai_client.generate = AsyncMock(return_value=mock_ai_response)
    
    # Patch create_ai_client_from_env to return our mock
    with patch("ikam_perf_report.api.graph.create_ai_client_from_env", return_value=mock_ai_client):
        # Use a real case ID that should be loaded by load_registry() in main.py
        # Based on existing tests, s-local-retail-v01 or s-construction-v01 might be available
        payload = {
            "query": "retail",
            "graph_id": "s-local-retail-v01",
            "max_hops": 1,
            "directives": ["Be concise."]
        }
        
        response = client.post("/graph/search", json=payload)
        
        assert response.status_code == 200
        data = response.json()
        
        assert data["query"] == "retail"
        assert data["query_type"] == "sqi-framework"
        assert "interpretation" in data
        assert "attribution" in data
        assert data["interpretation"] == "Test interpretation [f1]."
        assert len(data["attribution"]) == 1
        assert data["attribution"][0]["claim"] == "test"
        
        # Verify AI client was called
        assert mock_ai_client.generate.called
