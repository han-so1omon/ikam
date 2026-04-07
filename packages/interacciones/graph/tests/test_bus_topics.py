from __future__ import annotations

from pathlib import Path
import sys


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.graph.workflow.bus import PublishedMessage, WorkflowBus
from interacciones.graph.workflow.event_handlers import WorkflowExecutionEventHandlers
from interacciones.graph.workflow.topic_consumer import WorkflowTopicConsumer
from interacciones.schemas.topics import OrchestrationTopicNames


def test_workflow_bus_routes_messages_to_shared_topics() -> None:
    bus = WorkflowBus(topics=OrchestrationTopicNames())

    published = [
        bus.publish_execution_request({"workflow_id": "wf", "ref": "refs/heads/run/run-123"}),
        bus.publish_execution_progress({"step": "dispatch"}),
        bus.publish_execution_result({"status": "ok"}),
        bus.publish_workflow_event({"event": "entered"}),
        bus.publish_approval_event({"approval": "requested"}),
        bus.publish_mcp_event({"call": "tools/list"}),
        bus.publish_acp_event({"message": "handoff"}),
    ]

    assert published == [
        PublishedMessage(topic="execution.requests", payload={"workflow_id": "wf", "ref": "refs/heads/run/run-123"}),
        PublishedMessage(topic="execution.progress", payload={"step": "dispatch"}),
        PublishedMessage(topic="execution.results", payload={"status": "ok"}),
        PublishedMessage(topic="workflow.events", payload={"event": "entered"}),
        PublishedMessage(topic="approval.events", payload={"approval": "requested"}),
        PublishedMessage(topic="mcp.events", payload={"call": "tools/list"}),
        PublishedMessage(topic="acp.events", payload={"message": "handoff"}),
    ]

    assert published[0].topic.startswith("execution")
    assert published[0].topic == "execution.requests"
    assert published[0].payload["ref"] == "refs/heads/run/run-123"


def test_workflow_bus_exposes_topic_lookup_by_channel() -> None:
    bus = WorkflowBus()

    assert bus.topic_for("execution_requests") == "execution.requests"
    assert bus.topic_for("workflow_events") == "workflow.events"


def test_workflow_topic_consumer_uses_stable_topics_without_ref_prefixes() -> None:
    seen: list[dict[str, object]] = []

    class Handlers(WorkflowExecutionEventHandlers):
        def __init__(self) -> None:
            pass

        def handle_queued(self, payload: dict) -> None:
            seen.append(payload)

    consumer = WorkflowTopicConsumer(Handlers(), topics=OrchestrationTopicNames())

    consumer.consume("workflow.events", {"status": "queued", "ref": "refs/heads/main", "workflow_id": "wf-1"})

    assert seen == [{"status": "queued", "ref": "refs/heads/main", "workflow_id": "wf-1"}]


def test_workflow_bus_copies_payload_before_publishing() -> None:
    bus = WorkflowBus()
    payload = {"workflow_id": "wf", "ref": "refs/heads/run/run-123"}

    published = bus.publish_execution_request(payload)
    payload["ref"] = "refs/heads/main"

    assert published.payload["ref"] == "refs/heads/run/run-123"
