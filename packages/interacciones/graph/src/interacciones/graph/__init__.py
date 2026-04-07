"""Interacciones Graph - Universal graph execution engine with typed state and conditional routing.

Provides:
- `AgentGraph` — Main graph executor (linear + conditional routing)
- `FunctionNode` — Prebuilt node wrapper for sync/async callables
- `EdgeCondition` — Conditional routing based on state
- `GraphState` — State merge with per-key reducers
- `append_reducer`, `update_reducer`, `replace_reducer` — Built-in reducers

Example:
    >>> from interacciones.graph import AgentGraph, FunctionNode, EdgeCondition, append_reducer, update_reducer
    >>> def parse_fn(state):
    ...     return {"parsed": True, "messages": ["parsed"]}
    >>> def route(state):
    ...     return "high" if state.get("confidence", 0) > 0.8 else "low"
    >>> graph = AgentGraph(reducers={"messages": append_reducer, "context": update_reducer})
    >>> graph.add_node("parser", FunctionNode(parse_fn, "parser"))
    >>> graph.add_node("auto", FunctionNode(lambda s: {"auto": True}, "auto"))
    >>> graph.add_node("human", FunctionNode(lambda s: {"human": True}, "human"))
    >>> graph.add_conditional_edge("parser", EdgeCondition(route, {"high": "auto", "low": "human"}))
    >>> result = await graph.execute({"confidence": 0.9}, start="parser")

Notes:
- Use `Checkpoint` implementations to persist graph progress in production.
- Reducers must be pure and deterministic to preserve reproducibility.
"""

from .graph import AgentGraph
from .nodes import FunctionNode, HumanNode, AgentNode, LLMNode
from .state import GraphState, append_reducer, replace_reducer, update_reducer
from .edges import EdgeCondition
from .checkpoint import Checkpoint, InMemoryCheckpoint, PostgresCheckpoint
from .checkpoint_provenance import ProvenanceCheckpoint
from .events import GraphEvent

__all__ = [
    "AgentGraph",
    "FunctionNode",
    "HumanNode",
    "AgentNode",
    "LLMNode",
    "GraphState",
    "append_reducer",
    "update_reducer",
    "replace_reducer",
    "EdgeCondition",
    "Checkpoint",
    "InMemoryCheckpoint",
    "PostgresCheckpoint",
    "ProvenanceCheckpoint",
    "GraphEvent",
]
