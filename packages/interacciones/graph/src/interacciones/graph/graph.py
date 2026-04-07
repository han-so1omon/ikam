"""Basic graph executor supporting linear and conditional routing."""
from __future__ import annotations

from typing import Any, AsyncIterator, Dict, Mapping, Optional, Iterable, Tuple

from .edges import EdgeCondition
from .nodes import Node
from .state import GraphState
from .checkpoint import Checkpoint
from .events import GraphEvent


class AgentGraph:
    """Execute a directed graph of nodes, merging state deltas with reducers.

    Usage:
        graph = AgentGraph()
        graph.add_node("a", FunctionNode(fn_a, "a"))
        graph.add_node("b", FunctionNode(fn_b, "b"))
        graph.add_edge("a", "b")
        result = await graph.execute({"x": 1}, start="a")
    """

    def __init__(
        self,
        reducers: Optional[Mapping[str, Any]] = None,
        checkpointer: Optional[Checkpoint] = None,
        interrupt_before: Optional[Iterable[str]] = None,
    ) -> None:
        self._nodes: Dict[str, Node] = {}
        self._edges: Dict[str, str] = {}  # simple linear edges: from -> to
        self._conds: Dict[str, EdgeCondition] = {}
        self._state = GraphState(reducers)
        self._checkpointer = checkpointer
        self._interrupt_before = set(interrupt_before or [])

    # --- Build API ---
    def add_node(self, node_id: str, node: Node) -> None:
        if node_id != getattr(node, "id", None):
            raise ValueError(f"Node id mismatch: key={node_id!r} != node.id={getattr(node, 'id', None)!r}")
        if node_id in self._nodes:
            raise ValueError(f"Node '{node_id}' already exists")
        self._nodes[node_id] = node

    def add_edge(self, from_id: str, to_id: str) -> None:
        self._validate_node(from_id)
        self._validate_node(to_id)
        if from_id in self._edges or from_id in self._conds:
            raise ValueError(f"Edges from '{from_id}' already defined")
        self._edges[from_id] = to_id

    def add_conditional_edge(self, from_id: str, condition: EdgeCondition) -> None:
        self._validate_node(from_id)
        # Validate targets exist
        for target in condition.targets.values():
            self._validate_node(target)
        if from_id in self._edges or from_id in self._conds:
            raise ValueError(f"Edges from '{from_id}' already defined")
        self._conds[from_id] = condition

    # --- Execution ---
    async def execute(
        self,
        initial_state: Mapping[str, Any],
        start: str,
        execution_id: Optional[str] = None,
        skip_interrupt_on: Optional[str] = None,
    ) -> Dict[str, Any]:
        """Execute the graph without streaming events. Returns final state."""
        state = dict(initial_state)
        async for event in self._run(state, start, execution_id, skip_interrupt_on):
            if event.event == "graph_complete" and event.state:
                return dict(event.state)
        return state  # Fallback if no graph_complete event

    async def stream(
        self,
        initial_state: Mapping[str, Any],
        start: str,
        execution_id: Optional[str] = None,
        skip_interrupt_on: Optional[str] = None,
    ) -> AsyncIterator[GraphEvent]:
        """Execute the graph and yield events (node_start, node_complete, etc.)."""
        state = dict(initial_state)
        async for event in self._run(state, start, execution_id, skip_interrupt_on):
            yield event

    async def _run(
        self,
        initial_state: Dict[str, Any],
        start: str,
        execution_id: Optional[str] = None,
        skip_interrupt_on: Optional[str] = None,
    ) -> AsyncIterator[GraphEvent]:
        """Internal runner that yields events during execution."""
        self._validate_node(start)
        state: Dict[str, Any] = dict(initial_state)
        current = start
        visited = set()

        while current is not None:
            # Optional interrupt before running this node
            if (
                self._checkpointer
                and current in self._interrupt_before
                and (skip_interrupt_on is None or current != skip_interrupt_on)
            ):
                if execution_id is None:
                    raise ValueError("execution_id is required when using interrupts/checkpointing")
                await self._checkpointer.save(execution_id, state, current)
                yield GraphEvent(
                    event="checkpoint_saved",
                    node_id=current,
                    state=state,
                    data={"execution_id": execution_id},
                )
                # Pause execution here; caller can resume later
                yield GraphEvent(event="graph_complete", state=state)
                return

            if current in visited:
                raise RuntimeError(f"Cycle detected at node '{current}'")
            visited.add(current)

            yield GraphEvent(event="node_start", node_id=current, state=state)

            node = self._nodes[current]
            delta = await node.run(state)
            state = self._state.merge(state, delta)

            yield GraphEvent(event="node_complete", node_id=current, state=state, data=delta)

            # Determine next
            if current in self._edges:
                current = self._edges[current]
            elif current in self._conds:
                cond = self._conds[current]
                current = cond.select(state)
            else:
                # Terminal node
                current = None

        yield GraphEvent(event="graph_complete", state=state)
        return

    # --- Helpers ---
    def _validate_node(self, node_id: str) -> None:
        if node_id not in self._nodes:
            raise KeyError(f"Unknown node id: {node_id!r}")

    # --- Resume ---
    async def resume(self, execution_id: str, updated_state: Optional[Mapping[str, Any]] = None) -> Dict[str, Any]:
        if not self._checkpointer:
            raise RuntimeError("No checkpointer configured for this graph")
        saved = await self._checkpointer.load(execution_id)
        if saved is None:
            raise KeyError(f"No checkpoint found for execution_id={execution_id!r}")
        state, next_node = saved
        if updated_state:
            state = self._state.merge(state, updated_state)
        # Continue from next_node
        return await self.execute(
            state,
            start=next_node,
            execution_id=execution_id,
            skip_interrupt_on=next_node,
        )
