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
            "goal": "Generate semantic map",
            "allowed_profiles": ["reasoning/v1"],
            "max_nodes": 24,
            "max_depth": 3,
        },
    }


def test_transport_parity_for_generate_structural_map(monkeypatch) -> None:  # noqa: ANN001
    from modelado.oraculo.ai_client import GenerateResponse
    from mcp_ikam import tools
    from mcp_ikam.server_http import app
    from mcp_ikam.server_stdio import handle_request

    class FakeClient:
        async def generate(self, request):  # noqa: ANN001
            return GenerateResponse(
                text='{"map_subgraph":{"root_node_id":"map:project/corpus:root","nodes":[{"id":"map:project/corpus:root","title":"Corpus Outline","kind":"corpus"}],"relationships":[]},"segment_candidates":[],"segment_anchors":{},"profile_candidates":{}}',
                provider="openai",
                model="gpt-4o-mini",
            )

    monkeypatch.setattr(tools.map_generation, "create_ai_client_from_env", lambda: FakeClient())

    stdio_response = handle_request(
        {
            "jsonrpc": "2.0",
            "id": 3,
            "method": "call_tool",
            "params": {"name": "generate_structural_map", "payload": _payload()},
        }
    )
    http_client = TestClient(app)
    http_response = http_client.post("/v1/tools/call", json={"name": "generate_structural_map", "payload": _payload()})
    assert http_response.status_code == 200

    stdio_result = stdio_response["result"]
    http_result = http_response.json()["result"]
    assert stdio_result["map_subgraph"]["root_node_id"] == http_result["map_subgraph"]["root_node_id"]
    assert stdio_result["map_dna"]["fingerprint"] == http_result["map_dna"]["fingerprint"]


def test_transport_parity_for_list_tools_and_health() -> None:
    from mcp_ikam.server_http import app
    from mcp_ikam.server_stdio import handle_request

    stdio_tools = handle_request({"jsonrpc": "2.0", "id": 4, "method": "list_tools", "params": {}})["result"]
    stdio_health = handle_request({"jsonrpc": "2.0", "id": 5, "method": "health", "params": {}})["result"]

    http_client = TestClient(app)
    http_tools = http_client.get("/v1/tools").json()["tools"]
    http_health = http_client.get("/health").json()

    assert stdio_tools == http_tools
    assert stdio_health == http_health


def test_transport_parity_for_unknown_tool_errors() -> None:
    from mcp_ikam.server_http import app
    from mcp_ikam.server_stdio import handle_request

    stdio_response = handle_request(
        {
            "jsonrpc": "2.0",
            "id": 6,
            "method": "call_tool",
            "params": {"name": "unknown_tool", "payload": {}},
        }
    )
    http_client = TestClient(app)
    http_response = http_client.post("/v1/tools/call", json={"name": "unknown_tool", "payload": {}})

    assert "error" in stdio_response
    assert http_response.status_code == 400


def test_transport_parity_for_invalid_payload_shape() -> None:
    from mcp_ikam.server_http import app
    from mcp_ikam.server_stdio import handle_request

    stdio_response = handle_request(
        {
            "jsonrpc": "2.0",
            "id": 7,
            "method": "call_tool",
            "params": {"name": "generate_structural_map", "payload": "not-an-object"},
        }
    )
    http_client = TestClient(app)
    http_response = http_client.post(
        "/v1/tools/call",
        json={"name": "generate_structural_map", "payload": "not-an-object"},
    )

    assert "error" in stdio_response
    assert http_response.status_code == 422


def test_transport_parity_for_falsy_non_object_payload() -> None:
    from mcp_ikam.server_http import app
    from mcp_ikam.server_stdio import handle_request

    stdio_response = handle_request(
        {
            "jsonrpc": "2.0",
            "id": 8,
            "method": "call_tool",
            "params": {"name": "generate_structural_map", "payload": 0},
        }
    )
    http_client = TestClient(app)
    http_response = http_client.post(
        "/v1/tools/call",
        json={"name": "generate_structural_map", "payload": 0},
    )

    assert "error" in stdio_response
    assert http_response.status_code == 422
