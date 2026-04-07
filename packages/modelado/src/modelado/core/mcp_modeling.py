"""
MCP-friendly command and result schema for modeling agents.

This module codifies a minimal, versioned contract used by the base API and
MCP servers (econ/story) to exchange commands and results.

Versioning strategy:
- MAJOR.MINOR.PATCH where MAJOR breaks backward compatibility.
- Include `schema_version` in envelopes; base API can reject incompatible versions.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Dict, Optional

SCHEMA_VERSION = "1.0.0"


class EconomicAction(str, Enum):
    UPDATE_INPUTS = "econ.update_inputs"
    RECALCULATE = "econ.recalculate"
    GET_MODEL = "econ.get_model"


class StoryAction(str, Enum):
    REGENERATE = "story.regenerate"
    UPDATE_THEME = "story.update_theme"
    GET_STORY = "story.get_story"


class ResultStatus(str, Enum):
    RUNNING = "RUNNING"
    SUCCEEDED = "SUCCEEDED"
    FAILED = "FAILED"


@dataclass
class CommandEnvelope:
    """Generic command envelope for MCP dispatch."""
    command: str
    parameters: Dict[str, Any]
    instruction_id: str
    team_id: str
    project_id: str
    user_id: str
    schema_version: str = SCHEMA_VERSION


@dataclass
class ResultEnvelope:
    """Generic result envelope returned by MCP servers."""
    instruction_id: str
    status: ResultStatus
    payload: Optional[Dict[str, Any]] = None
    error: Optional[str] = None
    schema_version: str = SCHEMA_VERSION

    @property
    def ok(self) -> bool:
        return self.status == ResultStatus.SUCCEEDED


__all__ = [
    "SCHEMA_VERSION",
    "EconomicAction",
    "StoryAction",
    "ResultStatus",
    "CommandEnvelope",
    "ResultEnvelope",
]
