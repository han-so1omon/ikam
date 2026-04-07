"""Tests for event streaming during graph execution."""
import pytest

# Run all tests in this module with pytest-asyncio
pytestmark = pytest.mark.asyncio

from interacciones.graph import (
    AgentGraph,
    FunctionNode,
    HumanNode,
    EdgeCondition,
    InMemoryCheckpoint,
    append_reducer,
)


async def test_stream_linear_execution():
    def a(state):
        return {"step": "a"}

    def b(state):
        return {"step": "b"}

    graph = AgentGraph(reducers={"messages": append_reducer})
    graph.add_node("a", FunctionNode(a, "a"))
    graph.add_node("b", FunctionNode(b, "b"))
    graph.add_edge("a", "b")

    events = []
    async for event in graph.stream({}, start="a"):
        events.append(event)

    # Expected: node_start(a), node_complete(a), node_start(b), node_complete(b), graph_complete
    assert len(events) == 5
    assert events[0].event == "node_start"
    assert events[0].node_id == "a"
    assert events[1].event == "node_complete"
    assert events[1].node_id == "a"
    assert events[2].event == "node_start"
    assert events[2].node_id == "b"
    assert events[3].event == "node_complete"
    assert events[3].node_id == "b"
    assert events[4].event == "graph_complete"


async def test_stream_conditional_execution():
    def parse(state):
        return {"parsed": True}

    def auto(state):
        return {"auto": True}

    def human(state):
        return {"human": True}

    def route(state):
        return "high" if state.get("confidence", 0) > 0.8 else "low"

    graph = AgentGraph()
    graph.add_node("parse", FunctionNode(parse, "parse"))
    graph.add_node("auto", FunctionNode(auto, "auto"))
    graph.add_node("human", FunctionNode(human, "human"))
    graph.add_conditional_edge("parse", EdgeCondition(route, {"high": "auto", "low": "human"}))

    events = []
    async for event in graph.stream({"confidence": 0.9}, start="parse"):
        events.append(event)

    # Should route to auto
    node_ids = [e.node_id for e in events if e.event in ("node_start", "node_complete")]
    assert "parse" in node_ids
    assert "auto" in node_ids
    assert "human" not in node_ids


async def test_stream_with_checkpoint_interrupt():
    def a(state):
        return {"step": "a"}

    cp = InMemoryCheckpoint()
    graph = AgentGraph(checkpointer=cp, interrupt_before=["approval"])
    graph.add_node("a", FunctionNode(a, "a"))
    graph.add_node("approval", HumanNode("Approve?", "approval"))
    graph.add_edge("a", "approval")

    events = []
    async for event in graph.stream({}, start="a", execution_id="ex-1"):
        events.append(event)

    # Should emit: node_start(a), node_complete(a), checkpoint_saved(approval), graph_complete
    event_types = [e.event for e in events]
    assert "checkpoint_saved" in event_types
    assert "graph_complete" in event_types

    # Checkpoint should have saved before approval
    saved = await cp.load("ex-1")
    assert saved is not None
    _, next_node = saved
    assert next_node == "approval"
