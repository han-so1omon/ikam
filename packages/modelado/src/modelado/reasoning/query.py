from __future__ import annotations
from datetime import datetime
from enum import StrEnum
from typing import Any, Dict, List, Optional, Union
from pydantic import BaseModel, Field, ConfigDict, field_validator

from ikam.ir.core import PropositionIR, ExpressionIR, StructuredDataIR
from modelado.environment_scope import EnvironmentScope
from modelado.graph_edge_event_log import GraphEdgeEvent

class QueryConstraints(BaseModel):
    """Filters for subgraph discovery and search space restriction."""
    env_scope: Optional[EnvironmentScope] = Field(
        default=None, 
        description="D19 Isolation: The environment tier and ID to search within."
    )
    node_types: List[str] = Field(
        default_factory=list, 
        description="Filter by IR kind (e.g. ['Proposition', 'Expression'])."
    )
    max_hops: int = Field(
        default=3, 
        description="Depth of relation traversal from anchor nodes."
    )
    salience_min: float = Field(
        default=0.0, 
        ge=0.0, 
        le=1.0, 
        description="Importance threshold for including nodes."
    )
    recency: Optional[datetime] = Field(
        default=None, 
        description="Only include nodes created after this timestamp."
    )
    extra_filters: Dict[str, Any] = Field(
        default_factory=dict, 
        description="Generic bag for strategy-specific algorithmic constraints."
    )
    base_refs: List[str] = Field(
        default_factory=list,
        description="Additional base or ancestor refs explicitly visible to this query.",
    )
    
    model_config = ConfigDict(populate_by_name=True)

    @field_validator("base_refs", mode="before")
    @classmethod
    def _normalize_base_refs(cls, value: object) -> list[str]:
        if value is None:
            return []
        refs = value if isinstance(value, list) else list(value)
        return [EnvironmentScope(ref=str(ref)).ref for ref in refs]

class SearchStrategy(BaseModel):
    """Configuration for the pluggable discovery algorithm."""
    name: str = Field(
        default="default", 
        description="Name of the registered search strategy (e.g. 'BFS', 'DFS_AUDIT')."
    )
    parameters: Dict[str, Any] = Field(
        default_factory=dict, 
        description="Arbitrary weights or settings for the chosen strategy."
    )
    
    model_config = ConfigDict(populate_by_name=True)

class InterpretationContext(BaseModel):
    """Instructions and metadata for the generative synthesis phase."""
    directives: List[str] = Field(
        ..., 
        min_length=1, 
        description="Natural language instructions (e.g. ['simple language', 'in Spanish'])."
    )
    audience: Optional[str] = Field(
        default=None, 
        description="Who is the target reader? (e.g. 'VC', 'General Public')."
    )
    purpose: Optional[str] = Field(
        default=None, 
        description="The intent behind the interpretation (e.g. 'Explain a dip')."
    )
    supplementary_data: Dict[str, Any] = Field(
        default_factory=dict, 
        description="Additional context to aid the synthesis."
    )
    
    model_config = ConfigDict(populate_by_name=True)

class SemanticQuery(BaseModel):
    """The complete user/system query packet for the SQI Framework."""
    intent: str = Field(
        ..., 
        description="Natural language query or intent string."
    )
    constraints: QueryConstraints = Field(default_factory=QueryConstraints)
    search_strategy: SearchStrategy = Field(default_factory=SearchStrategy)
    interpretation: InterpretationContext
    
    model_config = ConfigDict(populate_by_name=True)

class Subgraph(BaseModel):
    """The result of a discovery traversal: a coherent set of nodes and relations."""
    nodes: List[Union[PropositionIR, ExpressionIR, StructuredDataIR]] = Field(
        default_factory=list, 
        description="A list of IR-hydrated fragments found during discovery."
    )
    edges: List[GraphEdgeEvent] = Field(
        default_factory=list, 
        description="The relevant graph edges (replayed from GraphEdgeEvent log)."
    )
    anchor_ids: List[str] = Field(
        default_factory=list, 
        description="The initial fragment IDs from which the search originated."
    )
    
    model_config = ConfigDict(populate_by_name=True, arbitrary_types_allowed=True)
