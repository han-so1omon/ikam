"""Tests for checkpointing and interrupt-before HumanNode behavior."""
import pytest

# Run all tests in this module with pytest-asyncio
pytestmark = pytest.mark.asyncio

from interacciones.graph import (
    AgentGraph,
    FunctionNode,
    HumanNode,
    InMemoryCheckpoint,
    append_reducer,
)


async def test_interrupt_before_human_and_resume():
    def parse(state):
        return {"parsed": True, "messages": ["parse"]}

    def finalize(state):
        return {"final": True, "messages": ["final"]}

    cp = InMemoryCheckpoint()
    graph = AgentGraph(
        reducers={"messages": append_reducer},
        checkpointer=cp,
        interrupt_before=["approval"],
    )

    graph.add_node("parse", FunctionNode(parse, "parse"))
    graph.add_node("approval", HumanNode("Approve operation?", "approval"))
    graph.add_node("finalize", FunctionNode(finalize, "finalize"))

    graph.add_edge("parse", "approval")
    graph.add_edge("approval", "finalize")

    # First execute pauses before approval
    state = await graph.execute({}, start="parse", execution_id="ex-1")
    assert state["parsed"] is True
    assert state["messages"] == ["parse"]

    # Check that checkpoint saved next node correctly
    saved = await cp.load("ex-1")
    assert saved is not None
    saved_state, next_node = saved
    assert next_node == "approval"

    # Simulate human approval by adding to state and resuming
    resumed = await graph.resume("ex-1", updated_state={"approved": True})
    assert resumed["final"] is True
    assert resumed["approved"] is True
    assert resumed["messages"] == ["parse", "final"]
