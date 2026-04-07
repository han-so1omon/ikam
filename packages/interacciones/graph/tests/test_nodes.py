"""Tests for FunctionNode behavior."""
import pytest

# Run all tests in this module with pytest-asyncio
pytestmark = pytest.mark.asyncio

from interacciones.graph import FunctionNode


async def test_function_node_sync():
    def fn(state):
        return {"x": state.get("x", 0) + 1}

    node = FunctionNode(fn, "n1")
    delta = await node.run({"x": 1})
    assert delta == {"x": 2}


async def test_function_node_async():
    async def fn(state):
        return {"y": (state.get("y", 0) * 2) or 2}

    node = FunctionNode(fn, "n2")
    delta = await node.run({"y": 3})
    assert delta == {"y": 6}


async def test_function_node_returns_non_mapping_raises():
    def fn(state):  # type: ignore[override]
        return 123

    node = FunctionNode(fn, "n3")
    with pytest.raises(TypeError):
        await node.run({})
