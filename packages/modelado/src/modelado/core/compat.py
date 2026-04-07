"""Compatibility shim for deprecated enum-based commands.

⚠️ DEPRECATED: This module exists only for backward compatibility with test infrastructure.
DO NOT use these enums/classes in new code. Use Semantic commands instead (SemanticEconomicCommand, SemanticStoryCommand).

The enum-based command pattern was removed as part of the shift to fully generative,
semantic operations. See AGENTS.md for details.

Migration guide:
- Old: EconomicCommand(action=EconomicAction.GET_MODEL, ...)
- New: SemanticEconomicCommand(instruction="get economic model", context={...})

This file will be removed once test infrastructure is migrated to semantic handlers.
See: docs/sprints/PHASE8_TASK7_STATUS.md for migration plan.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from enum import Enum
from typing import Any, Dict, Optional, Protocol

# Import current status enums (these are still valid)
from .commands import EconomicStatus, StoryStatus

# Deprecation notice is documented in docstrings; warning suppressed to avoid noise in tests


class EconomicAction(str, Enum):
    """⚠️ DEPRECATED: Use SemanticEconomicCommand instead.
    
    Hardcoded actions violate generative operations architecture.
    Kept only for test infrastructure compatibility.
    """
    # Model operations
    GET_MODEL = "get_model"
    PUT_MODEL = "put_model"
    CLEAR_MODEL = "clear_model"
    
    # Items (line items, cost drivers, etc.)
    LIST_ITEMS = "list_items"
    LIST_REQUIRED_ITEMS = "list_required_items"
    PUT_ITEMS = "put_items"
    DELETE_ITEM = "delete_item"
    
    # Offerings (products, services)
    LIST_OFFERINGS = "list_offerings"
    PUT_OFFERINGS = "put_offerings"
    APPEND_OFFERING = "append_offering"
    DELETE_OFFERING = "delete_offering"
    CLEAR_OFFERINGS = "clear_offerings"
    
    # Variables (revenue drivers, assumptions)
    LIST_VARIABLES = "list_variables"
    PUT_VARIABLES = "put_variables"
    DELETE_VARIABLE = "delete_variable"
    
    # Attributes (metadata, properties)
    LIST_ATTRIBUTES = "list_attributes"
    PUT_ATTRIBUTES = "put_attributes"
    DELETE_ATTRIBUTE = "delete_attribute"
    
    # Formulas (calculations, derivations)
    LIST_FORMULAS = "list_formulas"
    PUT_FORMULAS = "put_formulas"
    DELETE_FORMULA = "delete_formula"
    
    # Relationships (dependencies, constraints)
    LIST_RELATIONSHIPS = "list_relationships"
    PUT_RELATIONSHIPS = "put_relationships"
    DELETE_RELATIONSHIP = "delete_relationship"
    
    # Analysis operations (scenarios, sensitivity)
    RUN_SCENARIO = "run_scenario"
    SENSITIVITY_ANALYSIS = "sensitivity_analysis"
    FORECAST = "forecast"


class StoryAction(str, Enum):
    """⚠️ DEPRECATED: Use SemanticStoryCommand instead.
    
    Hardcoded actions violate generative operations architecture.
    Kept only for test infrastructure compatibility.
    """
    # Story operations
    # NOTE: This enum is deprecated, but is still referenced by some runtime
    # services (e.g., MCP story worker) and test infrastructure.
    GET_STORY = "get_story"

    GET_PLAN = "get_plan"
    PUT_PLAN = "put_plan"
    CLEAR_PLAN = "clear_plan"
    
    # Slides/sections
    LIST_SLIDES = "list_slides"
    PUT_SLIDES = "put_slides"
    REPLACE_SLIDES = "replace_slides"
    APPEND_SLIDE = "append_slide"
    DELETE_SLIDE = "delete_slide"
    CLEAR_SLIDES = "clear_slides"

    # Planning + publishing
    APPLY_PLAN = "apply_plan"
    GENERATE_IMAGES = "generate_images"
    PUBLISH_UPDATE = "publish_update"

    # Pending changes
    LIST_PENDING_CHANGES = "list_pending_changes"
    APPLY_PENDING_CHANGE = "apply_pending_change"
    DISCARD_PENDING_CHANGE = "discard_pending_change"
    
    # Themes (narrative arcs, messaging)
    LIST_THEMES = "list_themes"
    PUT_THEMES = "put_themes"
    DELETE_THEME = "delete_theme"
    
    # Content generation
    GENERATE_NARRATIVE = "generate_narrative"
    GENERATE_SLIDES = "generate_slides"
    APPLY_THEME = "apply_theme"


@dataclass(frozen=True)
class EconomicCommand:
    """⚠️ DEPRECATED: Use SemanticEconomicCommand instead.
    
    Compatibility class for test infrastructure only.
    """
    project_id: str
    action: EconomicAction
    requested_at: int
    payload: Optional[Dict[str, Any]] = None
    correlation_id: Optional[str] = None
    reply_topic: Optional[str] = None


@dataclass(frozen=True)
class EconomicResult:
    """⚠️ DEPRECATED: Use SemanticEconomicResult instead.
    
    Compatibility class for test infrastructure only.
    """
    project_id: str
    status: EconomicStatus
    completed_at: int
    payload: Optional[Dict[str, Any]] = None
    error: Optional[str] = None
    correlation_id: Optional[str] = None


@dataclass(frozen=True)
class StoryCommand:
    """⚠️ DEPRECATED: Use SemanticStoryCommand instead.
    
    Compatibility class for test infrastructure only.
    """
    project_id: str
    action: StoryAction
    requested_at: int
    payload: Optional[Dict[str, Any]] = None
    correlation_id: Optional[str] = None
    reply_topic: Optional[str] = None


@dataclass(frozen=True)
class StoryResult:
    """⚠️ DEPRECATED: Use SemanticStoryResult instead.
    
    Compatibility class for test infrastructure only.
    """
    project_id: str
    status: StoryStatus
    completed_at: int
    payload: Optional[Dict[str, Any]] = None
    error: Optional[str] = None
    correlation_id: Optional[str] = None


# Kafka infrastructure compatibility (also deprecated)

@dataclass(frozen=True)
class ModelingTopicConfig:
    """⚠️ DEPRECATED: Kafka topic configuration for legacy infrastructure."""
    request_topic: str
    result_topic: str
    broadcast_topic: Optional[str] = None
    dlq_topic: Optional[str] = None


class _SerdeAdapter(Protocol):
    def serialize(self, value: Dict[str, Any]) -> bytes: ...
    def deserialize(self, payload: bytes) -> Dict[str, Any]: ...


@dataclass(frozen=True)
class ModelingCommandCodec:
    """⚠️ DEPRECATED: Serialize commands/results for legacy Kafka infrastructure."""

    command_serde: Optional[_SerdeAdapter] = None
    result_serde: Optional[_SerdeAdapter] = None

    def encode_command(self, command: EconomicCommand | StoryCommand) -> bytes:
        record = _command_to_record(command)
        if self.command_serde:
            return self.command_serde.serialize(record)
        return json.dumps(record).encode("utf-8")

    def decode_command(self, payload: bytes) -> EconomicCommand | StoryCommand:
        raw = (
            self.command_serde.deserialize(payload)
            if self.command_serde
            else json.loads(payload.decode("utf-8"))
        )
        return _record_to_command(raw)

    def encode_result(self, result: EconomicResult | StoryResult) -> bytes:
        record = _result_to_record(result)
        if self.result_serde:
            return self.result_serde.serialize(record)
        return json.dumps(record).encode("utf-8")

    def decode_result(self, payload: bytes) -> EconomicResult | StoryResult:
        raw = (
            self.result_serde.deserialize(payload)
            if self.result_serde
            else json.loads(payload.decode("utf-8"))
        )
        return _record_to_result(raw)


def _command_to_record(command: EconomicCommand | StoryCommand) -> Dict[str, Any]:
    base: Dict[str, Any] = {
        "projectId": command.project_id,
        "action": command.action.value,
        "requestedAt": command.requested_at,
        "payload": command.payload or {},
    }
    if command.correlation_id:
        base["correlationId"] = command.correlation_id
    if command.reply_topic:
        base["replyTopic"] = command.reply_topic
    return base


def _result_to_record(result: EconomicResult | StoryResult) -> Dict[str, Any]:
    base: Dict[str, Any] = {
        "projectId": result.project_id,
        "status": result.status.value,
        "completedAt": result.completed_at,
        "payload": result.payload or {},
    }
    if result.error:
        base["error"] = result.error
    if result.correlation_id:
        base["correlationId"] = result.correlation_id
    return base


def _record_to_command(data: Dict[str, Any]) -> EconomicCommand | StoryCommand:
    action = data.get("action")
    project_id = data.get("projectId")
    requested_at = int(data.get("requestedAt", 0))
    payload = data.get("payload") or {}
    corr = data.get("correlationId")
    reply_topic = data.get("replyTopic")

    if action is None or project_id is None:
        raise ValueError("Invalid command payload: missing action or projectId")

    if action in EconomicAction._value2member_map_:
        econ_action = EconomicAction(action)
        return EconomicCommand(
            project_id=project_id,
            action=econ_action,
            requested_at=requested_at,
            payload=payload,
            correlation_id=corr,
            reply_topic=reply_topic,
        )

    if action in StoryAction._value2member_map_:
        story_action = StoryAction(action)
        return StoryCommand(
            project_id=project_id,
            action=story_action,
            requested_at=requested_at,
            payload=payload,
            correlation_id=corr,
            reply_topic=reply_topic,
        )

    raise ValueError(f"Unknown modeling action: {action}")


def _record_to_result(data: Dict[str, Any]) -> EconomicResult | StoryResult:
    status = data.get("status")
    project_id = data.get("projectId")
    completed_at = int(data.get("completedAt", 0))
    payload = data.get("payload") or {}
    error = data.get("error")
    corr = data.get("correlationId")

    if status is None or project_id is None:
        raise ValueError("Invalid result payload: missing status or projectId")

    if status in EconomicStatus._value2member_map_:
        econ_status = EconomicStatus(status)
        return EconomicResult(
            project_id=project_id,
            status=econ_status,
            completed_at=completed_at,
            payload=payload,
            error=error,
            correlation_id=corr,
        )

    if status in StoryStatus._value2member_map_:
        story_status = StoryStatus(status)
        return StoryResult(
            project_id=project_id,
            status=story_status,
            completed_at=completed_at,
            payload=payload,
            error=error,
            correlation_id=corr,
        )

    raise ValueError(f"Unknown result status: {status}")


# Export all for backward compatibility
__all__ = [
    "EconomicAction",
    "StoryAction",
    "EconomicCommand",
    "EconomicResult",
    "StoryCommand",
    "StoryResult",
    "ModelingTopicConfig",
    "ModelingCommandCodec",
]
