"""Shared richer Petri workflow contracts for orchestration."""

from __future__ import annotations

from enum import Enum
from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field, model_validator

from .execution import ResolutionMode
from .transition_validators import RichPetriTransitionValidator
from .traces import TracePersistencePolicy
from .workflows import WorkflowDefinition, WorkflowPublishTarget


class SourceWorkflowStorageMode(str, Enum):
    DEFAULT_ON = "default_on"
    OPTIONAL = "optional"
    OMIT = "omit"


class SourceWorkflowStoragePolicy(BaseModel):
    mode: SourceWorkflowStorageMode = SourceWorkflowStorageMode.DEFAULT_ON
    reconstructable_from_lowered_graph: bool = True

    model_config = ConfigDict(extra="forbid")

    @model_validator(mode="after")
    def validate_reconstructability(self) -> "SourceWorkflowStoragePolicy":
        if (
            self.mode in {SourceWorkflowStorageMode.OPTIONAL, SourceWorkflowStorageMode.OMIT}
            and not self.reconstructable_from_lowered_graph
        ):
            raise ValueError("lowered graph must remain reconstructable when source storage is disabled")
        return self


class RichPetriPlace(BaseModel):
    place_id: str = Field(min_length=1)
    label: str = Field(min_length=1)
    metadata: dict[str, Any] = Field(default_factory=dict)

    model_config = ConfigDict(extra="forbid")


class RichPetriTransition(BaseModel):
    transition_id: str = Field(min_length=1)
    label: str = Field(min_length=1)
    capability: str | None = Field(default=None, min_length=1)
    policy: dict[str, Any] = Field(default_factory=dict)
    constraints: dict[str, Any] = Field(default_factory=dict)
    validators: list[RichPetriTransitionValidator] = Field(default_factory=list)
    resolution_mode: ResolutionMode = ResolutionMode.CAPABILITY_POLICY
    direct_executor_ref: str | None = Field(default=None, min_length=1)
    trace_policy: TracePersistencePolicy | None = None
    approval_hint: dict[str, Any] = Field(default_factory=dict)
    checkpoint_hint: dict[str, Any] = Field(default_factory=dict)

    model_config = ConfigDict(extra="forbid")

    @model_validator(mode="after")
    def validate_intent(self) -> "RichPetriTransition":
        if self.direct_executor_ref is not None and self.resolution_mode != ResolutionMode.DIRECT_EXECUTOR_REF:
            raise ValueError("direct_executor_ref requires direct_executor_ref resolution mode")
        if self.resolution_mode == ResolutionMode.DIRECT_EXECUTOR_REF and self.direct_executor_ref is None:
            raise ValueError("direct_executor_ref is required for direct executor override resolution")
        if self.direct_executor_ref is not None and self.capability is None:
            raise ValueError("direct_executor_ref is an override and requires a capability target")
        if self.capability is None and not self.approval_hint and not self.checkpoint_hint:
            raise ValueError("transition requires capability intent or an approval/checkpoint hint")
        return self


class RichPetriArc(BaseModel):
    source_kind: Literal["place", "transition"]
    source_id: str = Field(min_length=1)
    target_kind: Literal["place", "transition"]
    target_id: str = Field(min_length=1)

    model_config = ConfigDict(extra="forbid")

    @model_validator(mode="after")
    def validate_bipartite_connection(self) -> "RichPetriArc":
        if self.source_kind == self.target_kind:
            raise ValueError("rich Petri arcs must connect places to transitions")
        return self


class RichPetriWorkflow(BaseModel):
    workflow_id: str = Field(min_length=1)
    version: str = Field(min_length=1)
    places: list[RichPetriPlace] = Field(min_length=1)
    transitions: list[RichPetriTransition] = Field(min_length=1)
    arcs: list[RichPetriArc] = Field(min_length=1)
    publish: list[WorkflowPublishTarget] = Field(default_factory=list)
    source_workflow_storage: SourceWorkflowStoragePolicy = Field(default_factory=SourceWorkflowStoragePolicy)
    source_workflow_definition: WorkflowDefinition | None = None

    model_config = ConfigDict(extra="forbid")

    @model_validator(mode="after")
    def validate_graph(self) -> "RichPetriWorkflow":
        place_ids = [place.place_id for place in self.places]
        transition_ids = [transition.transition_id for transition in self.transitions]

        if len(place_ids) != len(set(place_ids)):
            raise ValueError("rich Petri place ids must be unique")
        if len(transition_ids) != len(set(transition_ids)):
            raise ValueError("rich Petri transition ids must be unique")

        known_places = set(place_ids)
        known_transitions = set(transition_ids)
        for arc in self.arcs:
            if arc.source_kind == "place" and arc.source_id not in known_places:
                raise ValueError(f"rich Petri arc source place '{arc.source_id}' is not defined")
            if arc.source_kind == "transition" and arc.source_id not in known_transitions:
                raise ValueError(f"rich Petri arc source transition '{arc.source_id}' is not defined")
            if arc.target_kind == "place" and arc.target_id not in known_places:
                raise ValueError(f"rich Petri arc target place '{arc.target_id}' is not defined")
            if arc.target_kind == "transition" and arc.target_id not in known_transitions:
                raise ValueError(f"rich Petri arc target transition '{arc.target_id}' is not defined")
        return self
