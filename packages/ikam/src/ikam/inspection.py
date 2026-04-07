from __future__ import annotations

from typing import Any, Literal, Protocol, runtime_checkable

from pydantic import BaseModel, ConfigDict, Field, create_model


InspectionKind = Literal[
    "artifact",
    "fragment",
    "subgraph",
    "context_anchor",
    "execution_anchor",
    "edge_proxy",
]

InspectionRelation = Literal[
    "contains",
    "derives",
    "references",
    "source_subgraph",
    "anchors",
    "emits",
    "related_to",
]


_CANONICAL_REF_KEYS = {
    "subgraph": "subgraph_ref",
    "fragment": "cas_id",
    "artifact": "artifact_id",
    "edge": "edge_id",
    "context": "context_anchor",
}


_NODE_ID_KEYS = {
    "fragment": "cas_id",
    "artifact": "artifact_id",
    "subgraph": "subgraph_ref",
}


def node_id_for(kind: str, identity: dict[str, Any]) -> str:
    key = _NODE_ID_KEYS[kind]
    return f"{kind}:{identity[key]}"


def edge_id_for(relation: str, from_node_id: str, to_node_id: str) -> str:
    return f"edge:{relation}:{from_node_id}:{to_node_id}"


class InspectionRef(BaseModel):
    backend: str
    locator: dict[str, Any]
    hint: str | None = None

    @classmethod
    def parse(cls, value: str) -> InspectionRef:
        prefix = "inspect://"
        if not value.startswith(prefix):
            raise ValueError(f"Unsupported inspection ref: {value}")
        category, _, ref_value = value[len(prefix) :].partition("/")
        key = _CANONICAL_REF_KEYS.get(category)
        if key is None:
            raise ValueError(f"Unsupported inspection ref category: {category}")
        return cls(backend="inspect", locator={"category": category, key: ref_value})


class InspectionNode(BaseModel):
    id: str
    kind: InspectionKind
    ir_kind: str | None = None
    label: str | None = None
    summary: str | None = None
    payload: dict[str, Any] = Field(default_factory=dict)
    preview: Any | None = None
    refs: dict[str, InspectionRef] = Field(default_factory=dict)
    provenance: dict[str, Any] = Field(default_factory=dict)
    capabilities: dict[str, Any] = Field(default_factory=dict)
    children: dict[str, Any] = Field(default_factory=dict)


InspectionEdge = create_model(
    "InspectionEdge",
    __base__=BaseModel,
    __config__=ConfigDict(populate_by_name=True),
    id=(str, ...),
    **{
        "from": (str, ...),
        "to": (str, ...),
        "relation": (InspectionRelation, ...),
        "label": (str | None, None),
        "summary": (str | None, None),
        "payload": (dict[str, Any], Field(default_factory=dict)),
        "refs": (dict[str, InspectionRef], Field(default_factory=dict)),
        "provenance": (dict[str, Any], Field(default_factory=dict)),
        "capabilities": (dict[str, Any], Field(default_factory=dict)),
    },
)


class InspectionSubgraph(BaseModel):
    schema_version: str
    root_node_id: str
    nodes: list[InspectionNode] = Field(default_factory=list)
    edges: list[InspectionEdge] = Field(default_factory=list)
    navigation: dict[str, Any] = Field(default_factory=dict)


class ResolveInspectionRequest(BaseModel):
    inspection_ref: InspectionRef
    max_depth: int
    include_payload: bool
    include_edges: bool
    include_provenance: bool


@runtime_checkable
class InspectionResolver(Protocol):
    def resolve(self, request: ResolveInspectionRequest) -> InspectionSubgraph: ...


__all__ = [
    "node_id_for",
    "edge_id_for",
    "InspectionRef",
    "InspectionNode",
    "InspectionEdge",
    "InspectionSubgraph",
    "ResolveInspectionRequest",
    "InspectionResolver",
    "InspectionKind",
    "InspectionRelation",
]
