"""Event streaming types for graph execution."""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Literal, Mapping


EventType = Literal["node_start", "node_complete", "checkpoint_saved", "graph_complete"]


@dataclass
class GraphEvent:
    """Event emitted during graph execution.

    Attributes:
        event: Type of event (node_start, node_complete, checkpoint_saved, graph_complete)
        node_id: ID of the node (if applicable)
        state: Current state snapshot (optional)
        data: Additional event-specific data
    """

    event: EventType
    node_id: str | None = None
    state: Mapping[str, Any] | None = None
    data: Mapping[str, Any] | None = None
