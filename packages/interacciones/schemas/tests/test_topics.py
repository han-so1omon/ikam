"""Tests for orchestration topic contracts."""

from __future__ import annotations

from pathlib import Path
import sys

import pytest


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.schemas import OrchestrationTopicNames


def test_orchestration_topic_names_default_to_shared_backbone_families() -> None:
    topics = OrchestrationTopicNames()

    assert topics.execution_requests == "execution.requests"
    assert topics.execution_progress == "execution.progress"
    assert topics.execution_results == "execution.results"
    assert topics.workflow_events == "workflow.events"
    assert topics.approval_events == "approval.events"
    assert topics.mcp_events == "mcp.events"
    assert topics.acp_events == "acp.events"


def test_orchestration_topic_names_do_not_bake_tier_prefixes_into_topics() -> None:
    topics = OrchestrationTopicNames()

    assert topics.execution_requests == "execution.requests"
    assert topics.execution_progress == "execution.progress"
    assert topics.execution_results == "execution.results"
    assert topics.workflow_events == "workflow.events"


def test_orchestration_topic_names_remove_prefix_shim() -> None:
    assert not hasattr(OrchestrationTopicNames, "with_prefix")


def test_orchestration_topic_names_reject_broker_unsafe_values() -> None:
    with pytest.raises(ValueError):
        OrchestrationTopicNames(execution_requests="refs/heads/main.execution.requests")
