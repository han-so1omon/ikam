from __future__ import annotations

import sys
from pathlib import Path

from fastapi.testclient import TestClient


ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
MODELADO_SRC = Path(__file__).resolve().parents[3] / "packages" / "modelado" / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))
if str(MODELADO_SRC) not in sys.path:
    sys.path.insert(0, str(MODELADO_SRC))


def _payload() -> dict[str, object]:
    return {
        "artifact_bundle": {
            "corpus_id": "project/corpus",
            "artifacts": [{"artifact_id": "project/a", "file_name": "a.md", "mime_type": "text/markdown"}],
        },
        "map_definition": {
            "goal": "Build semantic map",
            "allowed_profiles": ["modelado/prose-backbone@1"],
            "max_nodes": 12,
            "max_depth": 3,
        },
    }


def test_http_health_and_tools() -> None:
    from mcp_ikam.server_http import app

    client = TestClient(app)
    health = client.get("/health")
    assert health.status_code == 200
    assert health.json()["status"] == "ok"

    tools = client.get("/v1/tools")
    assert tools.status_code == 200
    assert "generate_structural_map" in tools.json()["tools"]


def test_http_call_tool(monkeypatch) -> None:  # noqa: ANN001
    from modelado.oraculo.ai_client import GenerateResponse
    from mcp_ikam import tools
    from mcp_ikam.server_http import app

    class FakeClient:
        async def generate(self, request):  # noqa: ANN001
            return GenerateResponse(
                text='{"map_subgraph":{"root_node_id":"map:project/corpus:root","nodes":[{"id":"map:project/corpus:root","title":"Corpus","kind":"corpus"}],"relationships":[]},"segment_anchors":{},"segment_candidates":[],"profile_candidates":{}}',
                provider="openai",
                model="gpt-4o-mini",
            )

    monkeypatch.setattr(tools.map_generation, "create_ai_client_from_env", lambda: FakeClient())
    client = TestClient(app)
    response = client.post("/v1/tools/call", json={"name": "generate_structural_map", "payload": _payload()})
    assert response.status_code == 200
    assert response.json()["result"]["map_subgraph"]["root_node_id"] == "map:project/corpus:root"
