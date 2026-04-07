"""Shared topic contracts for orchestration event backbones."""

from __future__ import annotations

import re

from pydantic import BaseModel, ConfigDict, Field, field_validator


_BROKER_SAFE_TOPIC = re.compile(r"^[a-z0-9]+(?:[.-][a-z0-9]+)*$")


class OrchestrationTopicNames(BaseModel):
    execution_requests: str = Field(default="execution.requests", min_length=1)
    execution_progress: str = Field(default="execution.progress", min_length=1)
    execution_results: str = Field(default="execution.results", min_length=1)
    workflow_events: str = Field(default="workflow.events", min_length=1)
    approval_events: str = Field(default="approval.events", min_length=1)
    mcp_events: str = Field(default="mcp.events", min_length=1)
    acp_events: str = Field(default="acp.events", min_length=1)

    model_config = ConfigDict(extra="forbid")
    _TOPIC_FIELDS = {
        "execution_requests",
        "execution_progress",
        "execution_results",
        "workflow_events",
        "approval_events",
        "mcp_events",
        "acp_events",
    }

    @field_validator("*")
    @classmethod
    def _validate_broker_safe_topic(cls, value: str) -> str:
        if not _BROKER_SAFE_TOPIC.fullmatch(value):
            raise ValueError("topic names must be broker-safe lowercase dot or hyphen channels")
        return value

    def topic_for(self, channel: str) -> str:
        if channel not in self._TOPIC_FIELDS:
            raise ValueError(f"unknown orchestration topic channel '{channel}'")
        return getattr(self, channel)

    def matches(self, channel: str, topic: str) -> bool:
        return self.topic_for(channel) == topic
