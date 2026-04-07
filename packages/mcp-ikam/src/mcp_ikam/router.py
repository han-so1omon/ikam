from __future__ import annotations

from typing import Any, Callable

from mcp_ikam.tools.map_generation import generate_structural_map


ToolHandler = Callable[[dict[str, Any]], dict[str, Any]]


_TOOLS: dict[str, ToolHandler] = {
    "generate_structural_map": generate_structural_map,
}


def list_tools() -> list[str]:
    return sorted(_TOOLS.keys())


def call_tool(name: str, payload: dict[str, Any]) -> dict[str, Any]:
    if name not in _TOOLS:
        raise ValueError(f"Unknown tool: {name}")
    return _TOOLS[name](payload)
