# Interacciones [![Docs](https://img.shields.io/badge/docs-Read%20the%20Docs-blue)](https://narraciones.readthedocs.io/en/latest/interacciones/)

**Universal interaction and graph execution system for multi-agent coordination**

## Overview

`interacciones` is a modular Python package for building adaptive, observable multi-agent systems. It provides:

- **Graph execution engine** (`interacciones.graph`) — Execute workflows as directed graphs with typed state, conditional routing, checkpointing, and streaming
- **Agent registry** (`interacciones.registry`) — Discover, register, and route to agents based on capabilities
- **Shared orchestration schemas** (`interacciones.schemas`) — Shared workflow, richer Petri, executor, approval, trace, and event contracts
- **Workflow frontend compiler** (`interacciones.workflow`) — Compile authored workflow definitions into richer Petri workflows
- **Event streaming** (`interacciones.streaming`) — SSE and Kafka utilities for real-time broadcasting

Inspired by LangGraph but designed for event-driven, multi-user systems with Kafka integration.

## Installation

```bash
# Core package
pip install interacciones

# With PostgreSQL support
pip install interacciones[postgres]

# With Kafka support
pip install interacciones[kafka]

# With LLM support (OpenAI, Anthropic)
pip install interacciones[llm]

# Everything
pip install interacciones[all]
```

### Consuming Project Options

- Editable install (dev): `pip install -e /path/to/narraciones/packages/interacciones`
- Wheels (release): `pip install /path/to/narraciones/packages/interacciones/dist/*.whl`
- Poetry path deps:
    ```toml
    [tool.poetry.dependencies]
    interacciones = { path = "../narraciones/packages/interacciones", develop = true }
    ```

## Documentation

- API and guides: https://narraciones.readthedocs.io/en/latest/interacciones/

## Quick Start

### Simple Graph Execution

```python
from interacciones.graph import AgentGraph, FunctionNode

async def greet(state: dict) -> dict:
    return {"message": f"Hello, {state['name']}!"}

graph = AgentGraph()
graph.add_node("greeter", FunctionNode(greet, "greeter"))

result = await graph.execute({"name": "World"}, execution_id="demo-1")
print(result["message"])  # "Hello, World!"
```

### Conditional Routing

```python
from interacciones.graph import AgentGraph, FunctionNode, EdgeCondition

def route_by_confidence(state: dict) -> str:
    return "high" if state["confidence"] > 0.8 else "low"

graph = AgentGraph()
graph.add_node("parser", FunctionNode(parse_fn, "parser"))
graph.add_node("auto_execute", FunctionNode(execute_fn, "executor"))
graph.add_node("ask_human", FunctionNode(ask_fn, "human"))

graph.add_conditional_edge(
    "parser",
    EdgeCondition(
        route_by_confidence,
        {"high": "auto_execute", "low": "ask_human"}
    )
)
```

### Human-in-the-Loop with Checkpointing

```python
from interacciones.graph import AgentGraph, HumanNode, PostgresCheckpoint

checkpoint = PostgresCheckpoint(db_pool)
graph = AgentGraph(
    checkpointer=checkpoint,
    interrupt_before=["human_approval"]
)

graph.add_node("approval", HumanNode("Approve operation?"))

# Execute - pauses at approval node
await graph.execute(initial_state, execution_id="exec-123")

# Hours later, resume
state = await checkpoint.load("exec-123")
state["approved"] = True
await graph.resume("exec-123", state)
```

### Integration with IKAM Fragments

This is a conceptual integration sketch, not a guarantee that the exact import path shown below is the current supported repo-local API.

```python
from interacciones.graph import AgentGraph, FunctionNode
from narraciones_ikam import decompose_document, Fragment

async def decompose_and_store(state: dict) -> dict:
    fragments = decompose_document(
        content=state["document"],
        artifact_id=state["artifact_id"]
    )
    return {"fragments": fragments, "count": len(fragments)}

graph = AgentGraph()
graph.add_node("decomposer", FunctionNode(decompose_and_store, "decomposer"))

result = await graph.execute({
    "document": "# Report\n\nRevenue: $1.5M",
    "artifact_id": "report-123"
})
print(f"Decomposed into {result['count']} fragments")
```

## Architecture Position

```
Layer 0 (Standalone): ikam (domain models)
Layer 1 (Frameworks):  interacciones + modelado ← YOU ARE HERE
Layer 2 (Application): narraciones (business logic)
```

**Dependencies:** `ikam` (Layer 0), Pydantic v2, optional PostgreSQL/Kafka/LLM integrations

## Orchestration Boundary Freeze

This section describes the target architecture and dependency contract for upcoming orchestration work. It is not a claim that all of these packages already exist today.

Task 1 for Redpanda orchestration freezes the package boundary contract in `docs/plans/2026-03-06-interacciones-orchestration-architecture.md`.

- `interacciones.schemas` owns shared workflow, richer Petri, executor, approval, trace, and event-envelope contracts and must stay dependency-light.
- `interacciones.workflow` owns workflow-template loading, validation, symbolic resolution, deterministic ids, and compilation from authored workflow definitions into richer Petri workflows.
- Runtime `interacciones` packages own orchestration state, approval flow, executor routing, and transport adapters.
- `modelado` owns the single general graph compiler and lowers richer Petri workflows into IKAM-native executable graph artifacts without depending on runtime orchestrator internals.
- `ikam` remains the lower layer for artifacts, fragments, and provenance and must not depend upward.
- Executor services such as `python-executor` and `ml-executor` are runtime peers, not package dependencies of core layers.

Allowed direction:

```text
ikam <- interacciones.schemas <- interacciones.workflow
  ^             ^                     ^
  |             |                     |
  +--------- modelado <----- runtime interacciones packages
```

Forbidden direction:

- `ikam` must not import `interacciones`, `modelado`, or executor services.
- `interacciones.schemas` must not import runtime orchestration modules or executor implementations.
- `interacciones.workflow` must not own runtime bus/state logic.
- `modelado` must not depend on runtime orchestrator internals.
- Core packages must not depend on concrete executor service packages.

Ownership split:

- Shared transport contracts and message schemas live in `interacciones.schemas`.
- Redpanda bus adapters and workflow-state handling live in runtime `interacciones` packages.
- Richer Petri workflow contracts live in `interacciones.schemas`.
- Authored `WorkflowDefinition` templates -> richer Petri compilation lives in `interacciones.workflow`.
- Richer Petri -> IKAM-native executable graph lowering lives in `modelado`.
- Persisted `RichPetriWorkflow` artifacts are optional but default-on; lowered IKAM-native executable graphs are the always-stored execution form.

## Package Structure

Current repo layout (today):

```text
packages/interacciones/
  graph/
  registry/
  schemas/
  streaming/
```

Target orchestration layout (planned):

```text
packages/interacciones/
  graph/      # runtime orchestration and graph execution
  registry/   # executor/agent discovery and registry support
  schemas/    # shared orchestration, richer Petri, and transport contracts
  streaming/  # transport and streaming helpers
  workflow/   # workflow-template loading and richer Petri compilation
```

Conceptual split:

- `graph/` runs orchestration state machines, transport adapters, and workflow execution.
- `registry/` supports discovery and registry-facing lookup behavior.
- `schemas/` owns shared workflow, richer Petri, executor, approval, trace, and event contracts.
- `streaming/` owns SSE/Kafka/Redpanda-facing streaming helpers.
- `workflow/` owns authored workflow loading and richer-Petri compilation, not final IKAM-native lowering.

## Key Features

### Typed State with Reducers

Define explicit merge semantics for parallel execution:

```python
from typing import TypedDict, Annotated
from interacciones.graph import append_reducer, update_reducer

class MyState(TypedDict):
    messages: Annotated[list[dict], append_reducer]  # Lists concatenate
    context: Annotated[dict, update_reducer]          # Dicts merge
    count: Annotated[int, lambda old, new: old + new] # Custom reducer
```

### Runtime Planning (Advanced)

This section is aspirational and describes planned advanced orchestration patterns rather than APIs guaranteed to exist today.

Use LLMs or neural networks to select reducers and routing at runtime:

```python
from interacciones.graph import LLMReducerSelector, LLMRoutingPlanner

# LLM decides how to merge state
reducer_selector = LLMReducerSelector(llm_client, reducer_registry)
reducer_registry.set_selector(reducer_selector)

# LLM decides routing at each node
routing_planner = LLMRoutingPlanner(llm_client)
graph.add_conditional_edge("parser", EdgeCondition(planner=routing_planner, ...))
```

### Streaming Execution

Stream execution events in real-time:

```python
async for event in graph.stream(initial_state, execution_id):
    if event["event"] == "node_start":
        print(f"Starting {event['node_id']}...")
    elif event["event"] == "node_complete":
        print(f"Completed {event['node_id']}")
```

## Testing

```bash
# Run all tests
pytest

# Run with coverage
pytest --cov=interacciones --cov-report=html

# Type checking
mypy packages/interacciones
```

## Documentation

- Package overview: `packages/interacciones/README.md`
- Boundary freeze: `docs/plans/2026-03-06-interacciones-orchestration-architecture.md`
- Additional package docs should be added alongside each subproject as the orchestration split lands.

## Related Projects

- **LangGraph** — Inspiration for graph execution patterns
- **Kafka** — Event streaming backbone
- **FastAPI** — REST API framework
- **Pydantic** — Data validation

## License

MIT

## Contributing

See [CONTRIBUTING.md](../../CONTRIBUTING.md) for guidelines.

## Status

🚧 **Alpha** — API is unstable and subject to change.

Current focus: alpha hardening and package-boundary cleanup.

## Releasing

Interacciones publishes independently via tag-driven CI.

1. Bump `version` in `packages/interacciones/pyproject.toml`.
2. Commit and push.
3. Tag and push:
    ```bash
    git tag -a interacciones-v0.1.1 -m "interacciones v0.1.1"
    git push origin interacciones-v0.1.1
    ```
4. CI builds and publishes to GitHub Packages.
5. Consumers install via:
    ```bash
    pip install narraciones-interacciones --extra-index-url https://pypi.pkg.github.com/<owner>/
    ```

### Release Checklist

- Update `packages/interacciones/pyproject.toml` `version`
- Add entry to `packages/interacciones/CHANGELOG.md`
- Commit changes
- Create and push tag `interacciones-vX.Y.Z`
