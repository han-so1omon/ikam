from __future__ import annotations

from pathlib import Path
import sys
from typing import Any


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.graph.workflow.trace_promotion import build_trace_promotion_plan, should_promote_trace
from interacciones.graph.workflow.trace_promotion_sink import IkamTracePromotionSink
from interacciones.schemas import TracePersistenceMode, TracePersistencePolicy


def test_trace_promotion_policy_promotes_on_failure_events() -> None:
    policy = TracePersistencePolicy(mode=TracePersistenceMode.ON_FAILURE)

    assert should_promote_trace(
        policy,
        event={"event_type": "execution.failed", "payload": {"retryable": False}},
        is_final=False,
    ) is True


def test_trace_promotion_policy_promotes_on_approval_events() -> None:
    policy = TracePersistencePolicy(mode=TracePersistenceMode.ON_APPROVAL)

    assert should_promote_trace(
        policy,
        event={"event_type": "approval.requested", "payload": {}},
        is_final=False,
    ) is True


def test_trace_promotion_policy_promotes_every_step_for_per_step_mode() -> None:
    policy = TracePersistencePolicy(mode=TracePersistenceMode.PER_STEP)

    assert should_promote_trace(
        policy,
        event={"event_type": "execution.progress", "payload": {"progress": 0.5}},
        is_final=False,
    ) is True


def test_trace_promotion_policy_promotes_only_when_final_for_final_only_mode() -> None:
    policy = TracePersistencePolicy(mode=TracePersistenceMode.FINAL_ONLY)

    assert should_promote_trace(
        policy,
        event={"event_type": "execution.completed", "payload": {}},
        is_final=False,
    ) is False
    assert should_promote_trace(
        policy,
        event={"event_type": "execution.completed", "payload": {}},
        is_final=True,
    ) is True


def test_trace_promotion_policy_does_not_promote_for_none_or_batch_by_default() -> None:
    assert should_promote_trace(
        TracePersistencePolicy(mode=TracePersistenceMode.NONE),
        event={"event_type": "execution.failed", "payload": {}},
        is_final=True,
    ) is False
    assert should_promote_trace(
        TracePersistencePolicy(mode=TracePersistenceMode.BATCH),
        event={"event_type": "execution.failed", "payload": {}},
        is_final=True,
    ) is False


def test_trace_promotion_plan_collects_hot_events_when_policy_triggers() -> None:
    policy = TracePersistencePolicy(mode=TracePersistenceMode.ON_FAILURE)
    events = [
        {
            "trace_id": "trace-1",
            "workflow_id": "wf-1",
            "run_id": "run-1",
            "ref": "refs/heads/run/run-1",
            "event_type": "execution.dispatched",
        },
        {
            "trace_id": "trace-2",
            "workflow_id": "wf-1",
            "run_id": "run-1",
            "ref": "refs/heads/run/run-1",
            "event_type": "execution.failed",
        },
    ]

    plan = build_trace_promotion_plan(policy, hot_events=events, event=events[-1], is_final=False)

    assert plan == {
        "workflow_id": "wf-1",
        "run_id": "run-1",
        "target_ref": "refs/heads/run/run-1",
        "policy_mode": "on_failure",
        "trigger_event_type": "execution.failed",
        "trace_ids": ["trace-1", "trace-2"],
        "event_count": 2,
    }


def test_trace_promotion_plan_returns_none_when_policy_does_not_trigger() -> None:
    policy = TracePersistencePolicy(mode=TracePersistenceMode.FINAL_ONLY)
    events = [
        {"trace_id": "trace-1", "workflow_id": "wf-1", "run_id": "run-1", "event_type": "execution.progress"},
    ]

    plan = build_trace_promotion_plan(policy, hot_events=events, event=events[-1], is_final=False)

    assert plan is None


def test_ikam_trace_promotion_sink_records_committed_fragment_id_from_writer() -> None:
    recorded: list[dict[str, Any]] = []

    def writer(plan: dict[str, Any]) -> str:
        recorded.append(dict(plan))
        return "fragment://trace-bundle-1"

    sink = IkamTracePromotionSink(writer=writer)
    plan = {
        "workflow_id": "wf-1",
        "run_id": "run-1",
        "target_ref": "refs/heads/run/run-1",
        "policy_mode": "on_failure",
        "trigger_event_type": "execution.failed",
        "trace_ids": ["trace-1", "trace-2"],
        "event_count": 2,
    }

    sink.record(plan)

    assert recorded == [plan]
    assert sink.records == [
        {
            "workflow_id": "wf-1",
            "run_id": "run-1",
            "target_ref": "refs/heads/run/run-1",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": ["trace-1", "trace-2"],
            "event_count": 2,
            "committed_trace_fragment_id": "fragment://trace-bundle-1",
        }
    ]
