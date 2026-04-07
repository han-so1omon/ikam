"""Canonical workflow definition contracts for orchestration."""

from __future__ import annotations

from typing import Any

from pydantic import BaseModel, ConfigDict, Field, model_validator

from .execution import ResolutionMode
from .transition_validators import RichPetriTransitionValidator

SUPPORTED_WORKFLOW_NODE_KINDS = {
    "dispatch_executor",
    "wait_for_result",
    "request_approval",
    "wait_for_approval",
    "checkpoint",
    "emit_event",
    "await_event",
    "route",
    "complete",
    "fail",
    "emit_mcp_call",
    "await_mcp_response",
    "emit_acp_message",
    "await_acp_message",
}


class WorkflowNode(BaseModel):
    node_id: str = Field(min_length=1)
    kind: str = Field(min_length=1)
    capability: str | None = Field(default=None, min_length=1)
    policy: dict[str, Any] = Field(default_factory=dict)
    operator_selection: dict[str, Any] = Field(default_factory=dict)
    executor_selection: dict[str, Any] = Field(default_factory=dict)
    constraints: dict[str, Any] = Field(default_factory=dict)
    validators: list["RichPetriTransitionValidator"] = Field(default_factory=list)
    payload: dict[str, Any] = Field(default_factory=dict)
    config: dict[str, Any] = Field(default_factory=dict)
    resolution_mode: ResolutionMode = ResolutionMode.CAPABILITY_POLICY
    direct_executor_ref: str | None = Field(default=None, min_length=1)

    model_config = ConfigDict(extra="forbid")

    @model_validator(mode="after")
    def validate_resolution(self) -> "WorkflowNode":
        if self.kind not in SUPPORTED_WORKFLOW_NODE_KINDS:
            raise ValueError(f"workflow node kind '{self.kind}' is not supported")
        if self.kind == "dispatch_executor" and self.capability is None:
            raise ValueError("dispatch_executor nodes require capability")
        if self.kind != "dispatch_executor":
            if self.capability is not None:
                raise ValueError(f"{self.kind} nodes cannot declare capability")
            if self.operator_selection:
                raise ValueError(f"{self.kind} nodes cannot declare operator_selection")
            if self.executor_selection:
                raise ValueError(f"{self.kind} nodes cannot declare executor_selection")
            if self.validators:
                raise ValueError(f"{self.kind} nodes cannot declare validators")
            if self.resolution_mode != ResolutionMode.CAPABILITY_POLICY:
                raise ValueError(f"{self.kind} nodes cannot override resolution mode")
            if self.direct_executor_ref is not None:
                raise ValueError(f"{self.kind} nodes cannot declare direct_executor_ref")
        if self.direct_executor_ref is not None and self.resolution_mode != ResolutionMode.DIRECT_EXECUTOR_REF:
            raise ValueError("direct_executor_ref requires direct_executor_ref resolution mode")
        if self.resolution_mode == ResolutionMode.DIRECT_EXECUTOR_REF and not self.direct_executor_ref:
            raise ValueError("direct_executor_ref is required for direct_executor_ref resolution")
        if self.resolution_mode == ResolutionMode.DIRECT_EXECUTOR_REF and self.capability is None:
            raise ValueError("direct executor override requires capability")
        return self


class WorkflowLink(BaseModel):
    source: str = Field(min_length=1)
    target: str = Field(min_length=1)

    model_config = ConfigDict(extra="forbid")


class WorkflowPublishTarget(BaseModel):
    registry: str = Field(min_length=1)
    key: str | None = Field(default=None, min_length=1)
    title: str | None = Field(default=None, min_length=1)
    goal: str | None = Field(default=None, min_length=1)

    model_config = ConfigDict(extra="forbid")


class WorkflowDefinition(BaseModel):
    workflow_id: str = Field(min_length=1)
    version: str = Field(min_length=1)
    nodes: list[WorkflowNode] = Field(min_length=1)
    links: list[WorkflowLink] = Field(default_factory=list)
    publish: list[WorkflowPublishTarget] = Field(default_factory=list)

    model_config = ConfigDict(extra="forbid")

    @model_validator(mode="after")
    def validate_graph(self) -> "WorkflowDefinition":
        node_ids = [node.node_id for node in self.nodes]
        if len(node_ids) != len(set(node_ids)):
            raise ValueError("workflow node ids must be unique")

        known_nodes = set(node_ids)
        seen_links: set[tuple[str, str]] = set()
        incoming_counts = {node_id: 0 for node_id in node_ids}
        outgoing_counts = {node_id: 0 for node_id in node_ids}
        for link in self.links:
            if link.source not in known_nodes:
                raise ValueError(f"workflow link source '{link.source}' is not defined")
            if link.target not in known_nodes:
                raise ValueError(f"workflow link target '{link.target}' is not defined")
            link_key = (link.source, link.target)
            if link_key in seen_links:
                raise ValueError(f"workflow link '{link.source}' -> '{link.target}' is duplicated")
            seen_links.add(link_key)
            outgoing_counts[link.source] += 1
            incoming_counts[link.target] += 1
        if all(incoming_counts[node_id] > 0 for node_id in node_ids):
            raise ValueError("workflow must define at least one entry node")
        if all(outgoing_counts[node_id] > 0 for node_id in node_ids):
            raise ValueError("workflow must define at least one exit node")
        for target in self.publish:
            if target.key is None:
                target.key = self.workflow_id
        return self
