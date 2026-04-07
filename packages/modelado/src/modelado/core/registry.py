"""Helpers for encoding modeling commands over Kafka."""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Dict, Optional, Protocol, TypeVar

from .commands import (
    EconomicCommand,
    EconomicResult,
    EconomicStatus,
    ModelingCommand,
    ModelingResult,
    StoryCommand,
    StoryResult,
    StoryStatus,
)

CmdT = TypeVar("CmdT", EconomicCommand, StoryCommand)
ResT = TypeVar("ResT", EconomicResult, StoryResult)


@dataclass(frozen=True)
class ModelingTopicConfig:
    request_topic: str
    result_topic: str
    broadcast_topic: Optional[str] = None
    dlq_topic: Optional[str] = None


class _SerdeAdapter(Protocol):
    def serialize(self, value: Dict[str, Any]) -> bytes: ...

    def deserialize(self, payload: bytes) -> Dict[str, Any]: ...


@dataclass(frozen=True)
class ModelingCommandCodec:
    """Serialize commands/results using optional Avro serdes."""

    command_serde: Optional[_SerdeAdapter] = None
    result_serde: Optional[_SerdeAdapter] = None

    def encode_command(self, command: ModelingCommand) -> bytes:
        record = _command_to_record(command)
        if self.command_serde:
            return self.command_serde.serialize(record)
        return json.dumps(record).encode("utf-8")

    def decode_command(self, payload: bytes) -> ModelingCommand:
        raw = (
            self.command_serde.deserialize(payload)
            if self.command_serde
            else json.loads(payload.decode("utf-8"))
        )
        return _record_to_command(raw)

    def encode_result(self, result: ModelingResult) -> bytes:
        record = _result_to_record(result)
        if self.result_serde:
            return self.result_serde.serialize(record)
        return json.dumps(record).encode("utf-8")

    def decode_result(self, payload: bytes) -> ModelingResult:
        raw = (
            self.result_serde.deserialize(payload)
            if self.result_serde
            else json.loads(payload.decode("utf-8"))
        )
        return _record_to_result(raw)


def _command_to_record(command: ModelingCommand) -> Dict[str, Any]:
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


def _result_to_record(result: ModelingResult) -> Dict[str, Any]:
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


def _record_to_command(data: Dict[str, Any]) -> ModelingCommand:
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


def _record_to_result(data: Dict[str, Any]) -> ModelingResult:
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

    raise ValueError(f"Unknown modeling status: {status}")
