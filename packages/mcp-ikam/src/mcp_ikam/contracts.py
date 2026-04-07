from __future__ import annotations

from typing import Any

from pydantic import BaseModel, ConfigDict, Field, model_validator


class MCPModel(BaseModel):
    model_config = ConfigDict(extra="forbid", str_strip_whitespace=True)


class SurfaceFragmentInput(MCPModel):
    id: str = Field(min_length=1)
    artifact_id: str | None = None
    mime_type: str | None = None
    text: str | None = None

    @model_validator(mode="after")
    def _validate_content_pointer(self) -> "SurfaceFragmentInput":
        if not (self.text or self.artifact_id):
            raise ValueError("surface fragment requires text or artifact_id")
        return self


class ArtifactDescriptor(MCPModel):
    artifact_id: str = Field(min_length=1)
    file_name: str | None = None
    mime_type: str | None = None


class ArtifactBundle(MCPModel):
    corpus_id: str = Field(min_length=1)
    artifacts: list[ArtifactDescriptor] = Field(min_length=1)


class MapDefinition(MCPModel):
    goal: str = Field(min_length=1)
    allowed_profiles: list[str] = Field(min_length=1)
    max_nodes: int = Field(default=24, ge=1)
    max_depth: int = Field(default=3, ge=1)


class MapGenerationContext(MCPModel):
    project_id: str | None = None
    case_id: str | None = None


class GenerationProvenance(MCPModel):
    provider: str = Field(min_length=1)
    model: str = Field(min_length=1)
    prompt_version: str = Field(min_length=1)
    temperature: float | None = Field(default=None, ge=0, le=2)
    seed: int | None = Field(default=None, ge=0)


class TraceEvent(MCPModel):
    phase: str = Field(min_length=1)
    message: str = Field(min_length=1)
    provider: str | None = None
    model: str | None = None


class MapDNA(MCPModel):
    fingerprint: str = Field(min_length=1)
    structural_hashes: list[str] = Field(default_factory=list)
    version: str = Field(default="1", min_length=1)


class OutlineNode(MCPModel):
    id: str = Field(min_length=1)
    title: str = Field(min_length=1)
    kind: str = Field(min_length=1)
    level: int
    parent_id: str | None = None


class NodeRelationship(MCPModel):
    type: str = Field(min_length=1)
    source: str = Field(min_length=1)
    target: str = Field(min_length=1)


class SegmentAnchor(MCPModel):
    artifact_id: str = Field(min_length=1)
    locator_type: str = Field(min_length=1)
    locator: str = Field(min_length=1)
    confidence: float = Field(ge=0, le=1)


class SegmentCandidate(MCPModel):
    segment_id: str = Field(min_length=1)
    title: str = Field(min_length=1)
    artifact_ids: list[str] = Field(min_length=1)
    rationale: str = Field(min_length=1)


class MapGenerationRequest(MCPModel):
    artifact_bundle: ArtifactBundle
    map_definition: MapDefinition
    context: MapGenerationContext | None = None
    available_tools: list["AgenticToolDefinition"] | None = None
    mapping_mode: str | None = None
    document_fragment_refs: list[str] | None = None


class MapGenerationResponse(MCPModel):
    map_subgraph: dict[str, Any] | None = None
    map_dna: MapDNA | None = None
    segment_anchors: dict[str, list[SegmentAnchor]] | None = None
    segment_candidates: list[SegmentCandidate] | None = None
    profile_candidates: dict[str, list[str]] | None = None
    generation_provenance: GenerationProvenance
    trace_events: list[TraceEvent] = Field(default_factory=list)
    selected_tool_id: str | None = None
    tool_arguments: dict[str, Any] | None = None

    @model_validator(mode="after")
    def _validate_cross_field_consistency(self) -> "MapGenerationResponse":
        if self.map_subgraph is None:
            return self

        root_id = self.map_subgraph.get("root_node_id") if isinstance(self.map_subgraph, dict) else None
        if not isinstance(root_id, str) or not root_id:
            raise ValueError("map_subgraph.root_node_id is required")
        nodes = self.map_subgraph.get("nodes") if isinstance(self.map_subgraph, dict) else None
        if not isinstance(nodes, list):
            raise ValueError("map_subgraph.nodes must be a list")
        node_ids = {
            str(node.get("id"))
            for node in nodes
            if isinstance(node, dict) and isinstance(node.get("id"), str) and node.get("id")
        }
        if root_id not in node_ids:
            raise ValueError("map_subgraph.root_node_id must exist in map_subgraph.nodes")
        relationships = self.map_subgraph.get("relationships") if isinstance(self.map_subgraph, dict) else None
        if not isinstance(relationships, list):
            raise ValueError("map_subgraph.relationships must be a list")
        for rel in relationships:
            if not isinstance(rel, dict):
                raise ValueError("map_subgraph.relationships entries must be objects")
            source = rel.get("source")
            target = rel.get("target")
            if not isinstance(source, str) or source not in node_ids:
                raise ValueError("map_subgraph relationship source not found in nodes")
            if not isinstance(target, str) or target not in node_ids:
                raise ValueError("map_subgraph relationship target not found in nodes")
        
        if self.segment_candidates:
            for segment in self.segment_candidates:
                if segment.segment_id not in node_ids:
                    raise ValueError("segment_candidate.segment_id must exist in map_subgraph.nodes")
        if self.segment_anchors:
            for segment_id in self.segment_anchors:
                if segment_id not in node_ids:
                    raise ValueError("segment_anchors keys must exist in map_subgraph.nodes")
        if self.profile_candidates:
            for segment_id, profiles in self.profile_candidates.items():
                if segment_id not in node_ids:
                    raise ValueError("profile_candidates keys must exist in map_subgraph.nodes")
                if not profiles:
                    raise ValueError("profile_candidates values must contain at least one profile")
        return self

class ToolParameterProperty(MCPModel):
    type: str = Field(min_length=1)
    description: str | None = None
    items: dict[str, Any] | None = None

class ToolParameters(MCPModel):
    type: str = Field(default="object", min_length=1)
    properties: dict[str, ToolParameterProperty]
    required: list[str] = Field(default_factory=list)

class AgenticToolDefinition(MCPModel):
    name: str = Field(min_length=1)
    description: str = Field(min_length=1)
    input_schema: ToolParameters

class AgenticToolCall(MCPModel):
    call_id: str = Field(min_length=1)
    tool_name: str = Field(min_length=1)
    parameters: dict[str, Any]

class AgenticToolResult(MCPModel):
    call_id: str = Field(min_length=1)
    content: str
    is_error: bool = False
