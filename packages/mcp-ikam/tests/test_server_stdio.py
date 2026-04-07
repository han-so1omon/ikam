from __future__ import annotations

import sys
from pathlib import Path


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
            "goal": "Generate semantic map",
            "allowed_profiles": ["reasoning/v1"],
            "max_nodes": 24,
            "max_depth": 3,
        },
    }


def test_stdio_list_tools() -> None:
    from mcp_ikam.server_stdio import handle_request

    response = handle_request({"jsonrpc": "2.0", "id": 1, "method": "list_tools", "params": {}})
    assert response["result"]
    assert "generate_structural_map" in response["result"]


def test_stdio_health() -> None:
    from mcp_ikam.server_stdio import handle_request

    response = handle_request({"jsonrpc": "2.0", "id": 9, "method": "health", "params": {}})
    assert response["result"]["status"] == "ok"


def test_stdio_call_tool(monkeypatch) -> None:  # noqa: ANN001
    from modelado.oraculo.ai_client import GenerateResponse
    from mcp_ikam import tools
    from mcp_ikam.server_stdio import handle_request

    class FakeClient:
        async def generate(self, request):  # noqa: ANN001
            return GenerateResponse(
                text='{"map_subgraph":{"root_node_id":"map:project/corpus:root","nodes":[{"id":"map:project/corpus:root","title":"Corpus Outline","kind":"corpus"}],"relationships":[]},"segment_candidates":[],"segment_anchors":{},"profile_candidates":{}}',
                provider="openai",
                model="gpt-4o-mini",
            )

    monkeypatch.setattr(tools.map_generation, "create_ai_client_from_env", lambda: FakeClient())
    response = handle_request(
        {
            "jsonrpc": "2.0",
            "id": 2,
            "method": "call_tool",
            "params": {"name": "generate_structural_map", "payload": _payload()},
        }
    )
    assert "result" in response
    assert response["result"]["map_subgraph"]["root_node_id"] == "map:project/corpus:root"
