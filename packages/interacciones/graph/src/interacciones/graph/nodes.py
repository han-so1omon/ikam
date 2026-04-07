"""Node protocol and FunctionNode implementation."""
from __future__ import annotations

import inspect
from typing import Any, Awaitable, Callable, Dict, Mapping, Protocol


class Node(Protocol):
    """A graph node that transforms state and returns a partial delta.

    Contract:
      - Input: mapping state (dict-like)
      - Output: partial state delta (merged by graph using reducers)
      - May be sync or async callable
    """

    id: str

    async def run(self, state: Mapping[str, Any]) -> Mapping[str, Any]:
        ...


async def _maybe_await(fn: Callable[..., Any], *args: Any, **kwargs: Any) -> Any:
    result = fn(*args, **kwargs)
    if inspect.isawaitable(result):
        return await result  # type: ignore[no-any-return]
    return result


class FunctionNode:
    """Wrap a function (sync or async) as a Node.

    The function must accept a mapping state and return a mapping delta.
    """

    def __init__(self, func: Callable[[Mapping[str, Any]], Mapping[str, Any] | Awaitable[Mapping[str, Any]]], node_id: str) -> None:
        self.func = func
        self.id = node_id

    async def run(self, state: Mapping[str, Any]) -> Mapping[str, Any]:
        delta = await _maybe_await(self.func, state)
        if not isinstance(delta, Mapping):
            raise TypeError(f"Node {self.id} returned non-mapping delta: {type(delta)!r}")
        return dict(delta)


class HumanNode:
    """Represents a human approval/interaction step.

    Typically used with `interrupt_before` to pause before running this node.
    If executed, emits a simple prompt hint in the state.
    """

    def __init__(self, prompt: str, node_id: str) -> None:
        self.prompt = prompt
        self.id = node_id

    async def run(self, state: Mapping[str, Any]) -> Mapping[str, Any]:
        return {"human_prompt": self.prompt}


class AgentNode:
    """Represents an agent dispatch; wraps a callable like FunctionNode but names the agent."""

    def __init__(
        self,
        agent_id: str,
        func: Callable[[Mapping[str, Any]], Mapping[str, Any] | Awaitable[Mapping[str, Any]]],
        node_id: str,
    ) -> None:
        self.agent_id = agent_id
        self.func = func
        self.id = node_id

    async def run(self, state: Mapping[str, Any]) -> Mapping[str, Any]:
        delta = await _maybe_await(self.func, state)
        if not isinstance(delta, Mapping):
            raise TypeError(f"Node {self.id} returned non-mapping delta: {type(delta)!r}")
        out = {**delta, "agent_id": self.agent_id}
        return out


class LLMNode:
    """Represents an LLM invocation; wraps a callable returning a mapping delta."""

    def __init__(
        self,
        func: Callable[[Mapping[str, Any]], Mapping[str, Any] | Awaitable[Mapping[str, Any]]],
        node_id: str,
    ) -> None:
        self.func = func
        self.id = node_id

    async def run(self, state: Mapping[str, Any]) -> Mapping[str, Any]:
        delta = await _maybe_await(self.func, state)
        if not isinstance(delta, Mapping):
            raise TypeError(f"Node {self.id} returned non-mapping delta: {type(delta)!r}")
        return dict(delta)
