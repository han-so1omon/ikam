"""Tests for AgentNode and LLMNode minimal behavior."""
import pytest

# Run all tests in this module with pytest-asyncio
pytestmark = pytest.mark.asyncio

from interacciones.graph import AgentNode, LLMNode


async def test_agent_node_wraps_callable_and_sets_agent_id():
    def work(state):
        return {"result": "ok"}

    node = AgentNode("econ-modeler", work, "agent")
    delta = await node.run({})
    assert delta["result"] == "ok"
    assert delta["agent_id"] == "econ-modeler"


async def test_llm_node_wraps_callable():
    async def llm(state):
        return {"text": "hello"}

    node = LLMNode(llm, "llm")
    delta = await node.run({})
    assert delta["text"] == "hello"
