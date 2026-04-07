from __future__ import annotations
from enum import Enum
from typing import Any, Dict, List, Optional, Literal, Union
from pydantic import BaseModel, Field, ConfigDict

class PollockEdgeType(str, Enum):
    SUPPORTS = "supports"
    REBUTS = "rebuts"
    UNDERCUTS = "undercuts"

class PollockTargetType(str, Enum):
    FRAGMENT = "fragment"
    RELATION = "relation"

class PollockRelation(BaseModel):
    """
    Represents a defeasible relation between fragments or relations in the Pollock argument graph.
    Satisfies Architecture Decisions D15, D16.
    """
    model_config = ConfigDict(extra="forbid")

    relation_id: str = Field(..., description="Unique ID for this relation")
    source_id: str = Field(..., description="ID of the evidence or reasoner fragment")
    target_id: str = Field(..., description="ID of the target fragment or relation")
    target_type: PollockTargetType = Field(
        default=PollockTargetType.FRAGMENT,
        description="Whether the target is a fragment or another relation"
    )
    edge_type: PollockEdgeType = Field(..., description="Nature of the defeasible link")
    confidence: float = Field(default=1.0, ge=0.0, le=1.0)
    metadata: Dict[str, Any] = Field(default_factory=dict)

class PollockGraph(BaseModel):
    """
    A collection of fragments and pollock relations forming an argument graph.
    """
    relations: List[PollockRelation] = Field(default_factory=list)

    def add_support(self, source: str, target: str, confidence: float = 1.0) -> PollockRelation:
        rel = PollockRelation(
            relation_id=f"rel_{source}_{target}_sup",
            source_id=source,
            target_id=target,
            edge_type=PollockEdgeType.SUPPORTS,
            confidence=confidence
        )
        self.relations.append(rel)
        return rel

    def add_rebuttal(self, source: str, target: str, confidence: float = 1.0) -> PollockRelation:
        rel = PollockRelation(
            relation_id=f"rel_{source}_{target}_reb",
            source_id=source,
            target_id=target,
            edge_type=PollockEdgeType.REBUTS,
            confidence=confidence
        )
        self.relations.append(rel)
        return rel

    def add_undercut(self, source: str, target_rel_id: str, confidence: float = 1.0) -> PollockRelation:
        rel = PollockRelation(
            relation_id=f"rel_{source}_{target_rel_id}_und",
            source_id=source,
            target_id=target_rel_id,
            target_type=PollockTargetType.RELATION,
            edge_type=PollockEdgeType.UNDERCUTS,
            confidence=confidence
        )
        self.relations.append(rel)
        return rel

    def get_incoming(self, target_id: str, target_type: PollockTargetType = PollockTargetType.FRAGMENT) -> List[PollockRelation]:
        return [r for r in self.relations if r.target_id == target_id and r.target_type == target_type]
