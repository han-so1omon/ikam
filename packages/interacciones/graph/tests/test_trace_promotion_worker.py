from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))

from interacciones.graph.workflow.trace_promotion_sink import InMemoryTracePromotionSink
from interacciones.graph.workflow.trace_promotion_worker import TracePromotionWorker


class FakeOutboxSink:
    def __init__(self) -> None:
        self.claimed_with: list[dict[str, object]] = []
        self.acked: list[tuple[int, str]] = []
        self.items = [
            {
                "outbox_id": 1,
                "workflow_id": "wf-1",
                "run_id": "run-1",
                "policy_mode": "on_failure",
                "trigger_event_type": "execution.failed",
                "trace_ids": ["trace-1", "trace-2"],
                "event_count": 2,
                "lease_owner": "worker-a",
            }
        ]

    def claim_pending(self, *, limit: int, lease_owner: str, lease_expires_at: str) -> list[dict]:
        self.claimed_with.append({"limit": limit, "lease_owner": lease_owner, "lease_expires_at": lease_expires_at})
        return [dict(item) for item in self.items[:limit]]

    def ack(self, outbox_id: int, *, lease_owner: str) -> None:
        self.acked.append((outbox_id, lease_owner))


class FailingPromotionSink:
    def record(self, plan: dict) -> None:
        raise RuntimeError("sink failure")


def test_trace_promotion_worker_claims_records_and_acks_them() -> None:
    outbox = FakeOutboxSink()
    sink = InMemoryTracePromotionSink()
    worker = TracePromotionWorker(outbox_sink=outbox, promotion_sink=sink)

    processed = worker.run_once(limit=1, lease_owner="worker-a", lease_expires_at="2026-03-08T12:05:00Z")

    assert processed == [
        {
            "outbox_id": 1,
            "workflow_id": "wf-1",
            "run_id": "run-1",
            "policy_mode": "on_failure",
            "trigger_event_type": "execution.failed",
            "trace_ids": ["trace-1", "trace-2"],
            "event_count": 2,
            "lease_owner": "worker-a",
        }
    ]
    assert sink.plans == processed
    assert outbox.claimed_with == [
        {"limit": 1, "lease_owner": "worker-a", "lease_expires_at": "2026-03-08T12:05:00Z"}
    ]
    assert outbox.acked == [(1, "worker-a")]


def test_trace_promotion_worker_does_not_ack_when_promotion_sink_raises() -> None:
    outbox = FakeOutboxSink()
    worker = TracePromotionWorker(outbox_sink=outbox, promotion_sink=FailingPromotionSink())

    try:
        worker.run_once(limit=1, lease_owner="worker-a", lease_expires_at="2026-03-08T12:05:00Z")
    except RuntimeError as exc:
        assert str(exc) == "sink failure"
    else:  # pragma: no cover - defensive
        raise AssertionError("expected sink failure")

    assert outbox.acked == []
