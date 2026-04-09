"""Graph-native runtime contracts for slices and declarative deltas."""

from __future__ import annotations

from typing import Annotated, Any, Literal

from pydantic import BaseModel, Field

ATOMIC_GRAPH_APPLY_MODE = "atomic"


class GraphAnchor(BaseModel):
    """Runtime anchor into a graph-native structure."""

    handle: str
    path: tuple[str | int, ...] = Field(default_factory=tuple)


class GraphRegion(BaseModel):
    """Scoped graph region rooted at an anchor."""

    anchor: GraphAnchor
    extent: Literal["node", "subtree"] = "node"


class UpsertGraphDeltaOp(BaseModel):
    """Insert or replace a runtime value at an anchor."""

    op: Literal["upsert"]
    anchor: GraphAnchor
    value: Any


class RemoveGraphDeltaOp(BaseModel):
    """Remove a scoped graph region."""

    op: Literal["remove"]
    region: GraphRegion


IKAMGraphDeltaOp = Annotated[
    UpsertGraphDeltaOp | RemoveGraphDeltaOp,
    Field(discriminator="op"),
]


class IKAMGraphSlice(BaseModel):
    """Graph-native runtime payload scoped to a region."""

    region: GraphRegion
    payload: Any
    mime_type: str


class IKAMGraphDelta(BaseModel):
    """Declarative delta with v1 atomic-only apply semantics."""

    ops: list[IKAMGraphDeltaOp]
    apply_mode: Literal["atomic"] = ATOMIC_GRAPH_APPLY_MODE
