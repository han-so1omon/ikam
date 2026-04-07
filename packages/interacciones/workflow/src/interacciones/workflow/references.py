"""Normalized routing references for workflow nodes."""

from __future__ import annotations

from dataclasses import dataclass
from types import MappingProxyType
from typing import Mapping, TypeAlias

from interacciones.schemas import ResolutionMode, WorkflowNode

FrozenPolicyValue: TypeAlias = (
    str
    | int
    | float
    | bool
    | None
    | tuple["FrozenPolicyValue", ...]
    | Mapping[str, "FrozenPolicyValue"]
)


@dataclass(frozen=True)
class NodeRoutingReference:
    capability_ref: str | None
    policy_ref: Mapping[str, FrozenPolicyValue]
    resolution_mode: ResolutionMode
    direct_executor_ref: str | None


def build_routing_reference(node: WorkflowNode) -> NodeRoutingReference:
    return NodeRoutingReference(
        capability_ref=node.capability,
        policy_ref=_freeze_mapping(node.policy),
        resolution_mode=node.resolution_mode,
        direct_executor_ref=node.direct_executor_ref,
    )


def _freeze_mapping(value: Mapping[str, object]) -> Mapping[str, FrozenPolicyValue]:
    return MappingProxyType({key: _freeze_value(item) for key, item in value.items()})


def _freeze_value(value: object) -> FrozenPolicyValue:
    if isinstance(value, Mapping):
        return _freeze_mapping(value)
    if isinstance(value, list | tuple):
        return tuple(_freeze_value(item) for item in value)
    if isinstance(value, str | int | float | bool) or value is None:
        return value
    raise TypeError(f"unsupported workflow policy value type: {type(value).__name__}")
