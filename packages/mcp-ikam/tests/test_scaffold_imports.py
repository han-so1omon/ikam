from __future__ import annotations

import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))


def test_scaffold_imports_and_router_registry() -> None:
    import mcp_ikam
    from mcp_ikam import client as _client
    from mcp_ikam import contracts as _contracts
    from mcp_ikam.router import list_tools
    from mcp_ikam import server_http as _server_http
    from mcp_ikam import server_stdio as _server_stdio
    from mcp_ikam.tools import map_generation as _map_generation

    assert mcp_ikam is not None
    assert _client is not None
    assert _contracts is not None
    assert _server_http is not None
    assert _server_stdio is not None
    assert _map_generation is not None

    tools = list_tools()
    assert "generate_structural_map" in tools
