"""Interacciones - Universal interaction and graph execution system for multi-agent coordination.

This package provides:
- `interacciones.schemas` — Type-safe interaction models and client
- `interacciones.graph` — Graph execution engine with typed state and conditional routing
- `interacciones.registry` — Agent discovery and capability matching
- `interacciones.streaming` — SSE and Kafka streaming utilities

Example:
    >>> from interacciones.graph import AgentGraph, FunctionNode
    >>> graph = AgentGraph()
    >>> graph.add_node("greeter", FunctionNode(greet_fn, "greeter"))
    >>> await graph.execute({"name": "World"}, execution_id="demo-1")
"""

from pkgutil import extend_path

__path__ = extend_path(__path__, __name__)

__version__ = "0.1.0"

# Note: Sub-packages are imported on-demand to avoid dependency issues.
# Use explicit imports:
#   from interacciones.schemas import InteractionIn, InteractionOut
#   from interacciones.graph import AgentGraph, FunctionNode
#   from interacciones.registry import AgentRegistry
#   from interacciones.streaming import SSEConnectionManager
