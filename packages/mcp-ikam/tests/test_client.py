from __future__ import annotations

import sys
from pathlib import Path
from urllib.error import URLError

import pytest


ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))


class _FakeResponse:
    def __init__(self, body: bytes) -> None:
        self._body = body

    def read(self) -> bytes:
        return self._body

    def __enter__(self) -> "_FakeResponse":
        return self

    def __exit__(self, exc_type, exc, tb) -> None:  # noqa: ANN001
        return None


def test_client_requires_base_url() -> None:
    from mcp_ikam.client import MCPIkamClient

    with pytest.raises(ValueError, match="base_url"):
        MCPIkamClient("")


def test_client_generate_structural_map_success(monkeypatch: pytest.MonkeyPatch) -> None:
    from mcp_ikam.client import MCPIkamClient
    from mcp_ikam import client as client_module

    def _urlopen(req, timeout):  # noqa: ANN001
        return _FakeResponse(b'{"result": {"root_node_id": "map:root"}}')

    monkeypatch.setattr(client_module.urllib_request, "urlopen", _urlopen)

    client = MCPIkamClient("http://mcp-ikam:18081")
    result = client.generate_structural_map({"artifact_bundle": {}, "map_definition": {"goal": "g", "allowed_profiles": ["p"]}})
    assert result["root_node_id"] == "map:root"


def test_client_generate_structural_map_replays_trace_events(monkeypatch: pytest.MonkeyPatch) -> None:
    from mcp_ikam.client import MCPIkamClient
    from mcp_ikam import client as client_module

    def _urlopen(req, timeout):  # noqa: ANN001
        return _FakeResponse(
            b'{"result": {"root_node_id": "map:root", "trace_events": ['
            b'{"phase": "request_validated", "message": "accepted"},'
            b'{"phase": "llm_plan_started", "message": "llm call", "model": "gpt-4o-mini"}'
            b']}}'
        )

    seen: list[dict[str, object]] = []

    monkeypatch.setattr(client_module.urllib_request, "urlopen", _urlopen)

    client = MCPIkamClient("http://mcp-ikam:18081")
    result = client.generate_structural_map(
        {"artifact_bundle": {}, "map_definition": {"goal": "g", "allowed_profiles": ["p"]}},
        on_trace_event=seen.append,
    )

    assert result["root_node_id"] == "map:root"
    assert [event["phase"] for event in seen] == ["request_validated", "llm_plan_started"]


def test_client_generate_structural_map_transport_error(monkeypatch: pytest.MonkeyPatch) -> None:
    from mcp_ikam.client import MCPIkamClient
    from mcp_ikam import client as client_module

    def _urlopen(req, timeout):  # noqa: ANN001
        raise URLError("timeout")

    monkeypatch.setattr(client_module.urllib_request, "urlopen", _urlopen)

    client = MCPIkamClient("http://mcp-ikam:18081")
    with pytest.raises(RuntimeError, match="transport error"):
        client.generate_structural_map({"artifact_bundle": {}, "map_definition": {"goal": "g", "allowed_profiles": ["p"]}})


def test_client_generate_structural_map_malformed_response(monkeypatch: pytest.MonkeyPatch) -> None:
    from mcp_ikam.client import MCPIkamClient
    from mcp_ikam import client as client_module

    def _urlopen(req, timeout):  # noqa: ANN001
        return _FakeResponse(b'{"unexpected": 1}')

    monkeypatch.setattr(client_module.urllib_request, "urlopen", _urlopen)

    client = MCPIkamClient("http://mcp-ikam:18081")
    with pytest.raises(RuntimeError, match="missing result"):
        client.generate_structural_map({"artifact_bundle": {}, "map_definition": {"goal": "g", "allowed_profiles": ["p"]}})
