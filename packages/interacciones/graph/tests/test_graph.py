"""Tests for AgentGraph execution (linear and conditional)."""
import pytest

# Run all tests in this module with pytest-asyncio
pytestmark = pytest.mark.asyncio

from interacciones.graph import (
    AgentGraph,
    FunctionNode,
    EdgeCondition,
    append_reducer,
    update_reducer,
)


async def test_linear_graph_execution():
    async def a(state):
        return {"messages": ["a"], "count": 1}

    def b(state):
        return {"messages": ["b"], "count": state.get("count", 0) + 1}

    graph = AgentGraph(reducers={"messages": append_reducer})
    graph.add_node("a", FunctionNode(a, "a"))
    graph.add_node("b", FunctionNode(b, "b"))
    graph.add_edge("a", "b")

    result = await graph.execute({}, start="a")
    assert result["messages"] == ["a", "b"]
    assert result["count"] == 2


async def test_conditional_graph_execution():
    def parse(state):
        # produce some parsed structure and pass through confidence
        return {"parsed": True, "confidence": state.get("confidence", 0)}

    def auto(state):
        return {"auto": True}

    def human(state):
        return {"human": True}

    def route(state):
        return "high" if state.get("confidence", 0) > 0.8 else "low"

    graph = AgentGraph(reducers={"context": update_reducer})
    graph.add_node("parser", FunctionNode(parse, "parser"))
    graph.add_node("auto", FunctionNode(auto, "auto"))
    graph.add_node("human", FunctionNode(human, "human"))
    graph.add_conditional_edge("parser", EdgeCondition(route, {"high": "auto", "low": "human"}))

    high_result = await graph.execute({"confidence": 0.9}, start="parser")
    low_result = await graph.execute({"confidence": 0.5}, start="parser")

    assert high_result.get("auto") is True
    assert "human" not in high_result
    assert low_result.get("human") is True
    assert "auto" not in low_result


async def test_cycle_detection():
    def a(state):
        return {"x": 1}

    g = AgentGraph()
    g.add_node("a", FunctionNode(a, "a"))
    # Create a self-loop via add_edge, then another to cause detection on second iteration
    g.add_edge("a", "a")

    with pytest.raises(RuntimeError):
        await g.execute({}, start="a")
