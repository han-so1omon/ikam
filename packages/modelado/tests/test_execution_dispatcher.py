from __future__ import annotations

from pathlib import Path
import sys
import types

import pytest

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))
sys.path.insert(0, str(ROOT / "packages/ikam/src"))

COMPILER_FILE = ROOT / "packages/modelado/src/modelado/executors/compiler.py"
ENGINE_FILE = ROOT / "packages/modelado/src/modelado/plans/engine.py"

from interacciones.schemas import ExecutionQueued, ExecutionQueueRequest, OrchestrationTopicNames
from modelado.environment_scope import EnvironmentScope
from modelado.executors.bus import ExecutionQueueBus
from modelado.executors import ExecutionDispatcher
from modelado.operators.core import OperatorEnv
from modelado.plans.engine import PetriNetEngine
from modelado.plans.schema import PetriNetArcEndpoint, PetriNetEnvelope, PetriNetMarking, PetriNetTransition


_DEV_TOPICS = OrchestrationTopicNames(
    execution_requests="dev.execution.requests",
    execution_progress="dev.execution.progress",
    execution_results="dev.execution.results",
    workflow_events="dev.workflow.events",
    approval_events="dev.approval.events",
    mcp_events="dev.mcp.events",
    acp_events="dev.acp.events",
)


def test_runtime_dispatcher_uses_execution_dispatcher_name() -> None:
    compiler_source = COMPILER_FILE.read_text()

    assert "class ExecutionDispatcher:" in compiler_source
    assert "class GraphCompiler:" not in compiler_source


def test_petri_engine_references_execution_dispatcher() -> None:
    engine_source = ENGINE_FILE.read_text()

    assert "from modelado.executors import ExecutionDispatcher" in engine_source
    assert "GraphCompiler" not in engine_source


def test_petri_engine_does_not_swallow_dispatcher_value_errors(monkeypatch: pytest.MonkeyPatch) -> None:
    class StubDispatcher:
        def compile_and_execute(self, **_: object) -> object:
            raise ValueError("bad operator config")

    stub_module = types.ModuleType("modelado.executors")
    stub_module.ExecutionDispatcher = StubDispatcher
    monkeypatch.setitem(sys.modules, "modelado.executors", stub_module)

    engine = PetriNetEngine(
        net_envelope=PetriNetEnvelope(
            project_id="project",
            scope_id="scope",
            title="title",
            goal="goal",
            initial_marking_fragment_id="marking:start",
        ),
        transitions={
            "dispatch": PetriNetTransition(
                transition_id="dispatch",
                label="dispatch",
                operation_ref="op.dispatch",
                inputs=[PetriNetArcEndpoint(place_id="place:start")],
                outputs=[PetriNetArcEndpoint(place_id="place:end")],
            )
        },
        env=OperatorEnv(
            seed=1,
            renderer_version="test",
            policy="test",
            env_scope=EnvironmentScope(ref="refs/heads/run/env-1"),
        ),
    )

    with pytest.raises(ValueError, match="bad operator config"):
        engine.fire(
            "dispatch",
            PetriNetMarking(tokens={"place:start": 1}, meta={}),
            transition_fragment_id="transition-fragment",
        )


def test_petri_engine_sets_current_marking_for_dispatcher(monkeypatch: pytest.MonkeyPatch) -> None:
    observed: dict[str, object] = {}

    class StubDispatcher:
        def compile_and_execute(self, **kwargs: object) -> object:
            env = kwargs["env"]
            observed["current_marking"] = env.slots.get("current_marking")
            return {"result": {"ok": True}}

    stub_module = types.ModuleType("modelado.executors")
    stub_module.ExecutionDispatcher = StubDispatcher
    monkeypatch.setitem(sys.modules, "modelado.executors", stub_module)

    engine = PetriNetEngine(
        net_envelope=PetriNetEnvelope(
            project_id="project",
            scope_id="scope",
            title="title",
            goal="goal",
            initial_marking_fragment_id="marking:start",
        ),
        transitions={
            "dispatch": PetriNetTransition(
                transition_id="dispatch",
                label="dispatch",
                operation_ref="op.dispatch",
                inputs=[PetriNetArcEndpoint(place_id="place:start")],
                outputs=[PetriNetArcEndpoint(place_id="place:end")],
                metadata={"output_key": "dispatch_result"},
            )
        },
        env=OperatorEnv(
            seed=1,
            renderer_version="test",
            policy="test",
            env_scope=EnvironmentScope(ref="refs/heads/run/env-1"),
        ),
    )
    marking = PetriNetMarking(tokens={"place:start": 1}, meta={"foo": "bar"})

    new_marking, _ = engine.fire(
        "dispatch",
        marking,
        transition_fragment_id="transition-fragment",
    )

    assert observed["current_marking"] == marking
    assert new_marking.meta["dispatch_result"] == {"ok": True}


def test_execution_dispatcher_builds_shared_queue_request_from_resolved_operator_metadata(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    dispatcher = ExecutionDispatcher()

    predicate_links = {
        ("transition-fragment", "executed_by_operator"): "operator-fragment",
        ("operator-fragment", "executed_by"): "executor-config-fragment",
    }
    fragment_values = {
        "operator-fragment": {
            "ir_profile": "ExpressionIR",
            "module": "modelado.executors.loaders",
            "entrypoint": "run",
            "ast": {
                "params": {
                    "workflow_id": "ingestion-early-steps",
                    "transition_id": "dispatch-parse",
                    "capability": "python.parse_artifacts",
                    "policy": {"cost_tier": "standard"},
                    "constraints": {"locality": "local"},
                    "eligible_executor_ids": ["executor://python-primary"],
                    "translator_plan": {
                        "input": [
                            {
                                "name": "document_set",
                                "mime_type": "application/ikam-document-set+v1+json",
                            }
                        ],
                        "output": [
                            {
                                "name": "chunk_extraction_set",
                                "mime_type": "application/ikam-chunk-extraction-set+v1+json",
                            }
                        ],
                    },
                }
            },
        },
        "executor-config-fragment": {
            "ir_profile": "StructuredDataIR",
            "type": "sidecar",
            "endpoint": "http://ikam-executor-sidecar:8000",
        },
    }

    monkeypatch.setattr(
        dispatcher,
        "get_object_for_predicate",
        lambda subject_id, predicate: predicate_links.get((subject_id, predicate)),
    )
    monkeypatch.setattr(dispatcher, "get_fragment_value", lambda fragment_id: fragment_values.get(fragment_id))
    monkeypatch.setattr("modelado.executors.compiler.get_execution_context", lambda: None)
    monkeypatch.setattr(
        "modelado.executors.compiler.load_executor_declarations",
        lambda: [
            types.SimpleNamespace(
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capabilities=["python.parse_artifacts"],
                transport={"kind": "redpanda", "request_topic": "execution.requests"},
            )
        ],
        raising=False,
    )

    request = dispatcher.build_execution_queue_request(
        transition_fragment_id="transition-fragment",
        fragment={"fragment_id": "doc-1"},
        params=types.SimpleNamespace(name="dispatch-parse", parameters={"input": "hola"}),
        env=OperatorEnv(
            seed=1,
            renderer_version="test",
            policy="test",
            env_scope=EnvironmentScope(ref="refs/heads/run/env-1"),
            slots={
                "current_marking": PetriNetMarking(tokens={"place:start": 1}, meta={"source": "ingest"}),
            },
        ),
    )

    assert request == ExecutionQueueRequest(
        request_id="transition-fragment",
        workflow_id="ingestion-early-steps",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        executor_kind="python-executor",
        capability="python.parse_artifacts",
        policy={"cost_tier": "standard"},
        constraints={"locality": "local"},
        payload={
            "module": "modelado.executors.loaders",
            "entrypoint": "run",
            "payload": {
                "fragment": {"fragment_id": "doc-1"},
                "params": {"input": "hola"},
            },
            "context": {
                "tokens": {"place:start": 1},
                "meta": {"source": "ingest"},
                "env_scope": {"ref": "refs/heads/run/env-1"},
                "translator_plan": {
                    "input": [
                        {
                            "name": "document_set",
                            "mime_type": "application/ikam-document-set+v1+json",
                        }
                    ],
                    "output": [
                        {
                            "name": "chunk_extraction_set",
                            "mime_type": "application/ikam-chunk-extraction-set+v1+json",
                        }
                    ],
                },
            },
        },
        transport={"kind": "redpanda", "request_topic": "execution.requests"},
    )


def test_execution_dispatcher_emits_ref_as_canonical_scope_identity() -> None:
    dispatcher = ExecutionDispatcher()
    env = OperatorEnv(
        seed=1,
        renderer_version="test",
        policy="test",
        env_scope=EnvironmentScope(ref="refs/heads/main"),
    )

    context = dispatcher._build_context(env)

    assert context["env_scope"]["ref"] == "refs/heads/main"
    assert context["env_scope"] == {"ref": "refs/heads/main"}


def test_execution_dispatcher_prefers_direct_executor_override_for_shared_queue_request(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    dispatcher = ExecutionDispatcher()

    monkeypatch.setattr(
        dispatcher,
        "get_object_for_predicate",
        lambda subject_id, predicate: {
            ("transition-fragment", "executed_by_operator"): "operator-fragment",
            ("operator-fragment", "executed_by"): "executor-config-fragment",
        }.get((subject_id, predicate)),
    )
    monkeypatch.setattr(
        dispatcher,
        "get_fragment_value",
        lambda fragment_id: {
            "operator-fragment": {
                "ir_profile": "ExpressionIR",
                "module": "modelado.executors.loaders",
                "entrypoint": "run",
                "ast": {
                    "params": {
                        "workflow_id": "embed-flow",
                        "transition_id": "dispatch-embed",
                        "capability": "ml.embed",
                        "policy": {"latency_tier": "interactive"},
                        "constraints": {},
                        "direct_executor_ref": "executor://ml-primary",
                        "eligible_executor_ids": ["executor://python-primary"],
                    }
                },
            },
            "executor-config-fragment": {
                "ir_profile": "StructuredDataIR",
                "type": "sidecar",
                "endpoint": "http://ikam-executor-sidecar:8000",
            },
        }.get(fragment_id),
    )
    monkeypatch.setattr("modelado.executors.compiler.get_execution_context", lambda: None)
    monkeypatch.setattr(
        "modelado.executors.compiler.load_executor_declarations",
        lambda: [
            types.SimpleNamespace(
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capabilities=["python.parse_artifacts"],
                transport={"kind": "redpanda", "request_topic": "execution.requests"},
            ),
            types.SimpleNamespace(
                executor_id="executor://ml-primary",
                executor_kind="ml-executor",
                capabilities=["ml.embed"],
                transport={"kind": "redpanda", "request_topic": "execution.requests"},
            ),
        ],
        raising=False,
    )

    request = dispatcher.build_execution_queue_request(
        transition_fragment_id="transition-fragment",
        fragment={"fragment_id": "doc-2"},
        params=types.SimpleNamespace(name="dispatch-embed", parameters={"input": "hola"}),
        env=OperatorEnv(
            seed=1,
            renderer_version="test",
            policy="test",
            env_scope=EnvironmentScope(ref="refs/heads/run/env-1"),
        ),
    )

    assert request.executor_id == "executor://ml-primary"
    assert request.executor_kind == "ml-executor"


def test_execution_dispatcher_builds_queue_request_without_legacy_executor_fragment(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    dispatcher = ExecutionDispatcher()

    monkeypatch.setattr(
        dispatcher,
        "get_object_for_predicate",
        lambda subject_id, predicate: {
            ("transition-fragment", "executed_by_operator"): "operator-fragment",
            ("operator-fragment", "executed_by"): None,
        }.get((subject_id, predicate)),
    )
    monkeypatch.setattr(
        dispatcher,
        "get_fragment_value",
        lambda fragment_id: {
            "operator-fragment": {
                "ir_profile": "ExpressionIR",
                "module": "modelado.executors.loaders",
                "entrypoint": "run",
                "ast": {
                    "params": {
                        "workflow_id": "wf-queue-only",
                        "transition_id": "dispatch-parse",
                        "capability": "python.parse_artifacts",
                        "policy": {},
                        "constraints": {},
                        "eligible_executor_ids": ["executor://python-primary"],
                        "translator_plan": {"input": [], "output": []},
                    }
                },
            }
        }.get(fragment_id),
    )
    monkeypatch.setattr("modelado.executors.compiler.get_execution_context", lambda: None)
    monkeypatch.setattr(
        "modelado.executors.compiler.load_executor_declarations",
        lambda: [
            types.SimpleNamespace(
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capabilities=["python.parse_artifacts"],
                transport={"kind": "redpanda", "request_topic": "execution.requests"},
            )
        ],
        raising=False,
    )

    request = dispatcher.build_execution_queue_request(
        transition_fragment_id="transition-fragment",
        fragment={"fragment_id": "doc-queue-only"},
        params=types.SimpleNamespace(name="dispatch-parse", parameters={"input": "hola"}),
        env=OperatorEnv(
            seed=1,
            renderer_version="test",
            policy="test",
            env_scope=EnvironmentScope(ref="refs/heads/run/env-1"),
        ),
    )

    assert request.executor_id == "executor://python-primary"
    assert request.payload["context"]["translator_plan"] == {"input": [], "output": []}


def test_execution_dispatcher_publishes_shared_queue_request_on_transport_topic(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    dispatcher = ExecutionDispatcher()
    bus = ExecutionQueueBus()

    monkeypatch.setattr(
        dispatcher,
        "build_execution_queue_request",
        lambda **_: ExecutionQueueRequest(
            request_id="req-1",
            workflow_id="ingestion-early-steps",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={"module": "modelado.executors.loaders"},
            transport={"kind": "redpanda", "request_topic": "execution.requests"},
        ),
    )

    queued = dispatcher.publish_execution_queue_request(
        transition_fragment_id="transition-fragment",
        fragment={"fragment_id": "doc-1"},
        params=types.SimpleNamespace(name="dispatch-parse", parameters={"input": "hola"}),
        env=OperatorEnv(
            seed=1,
            renderer_version="test",
            policy="test",
            env_scope=EnvironmentScope(ref="refs/heads/run/env-1"),
        ),
        bus=bus,
    )

    assert queued == ExecutionQueued(
        request_id="req-1",
        workflow_id="ingestion-early-steps",
        step_id="dispatch-parse",
        executor_id="executor://python-primary",
        executor_kind="python-executor",
        capability="python.parse_artifacts",
        status="queued",
    )
    assert bus.messages == [
        (
            "execution.requests",
            {
                "request_id": "req-1",
                "workflow_id": "ingestion-early-steps",
                "step_id": "dispatch-parse",
                "executor_id": "executor://python-primary",
                "executor_kind": "python-executor",
                "capability": "python.parse_artifacts",
                "policy": {"cost_tier": "standard"},
                "constraints": {},
                "payload": {"module": "modelado.executors.loaders"},
                "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
            },
        ),
        (
            "workflow.events",
            ExecutionQueued(
                request_id="req-1",
                workflow_id="ingestion-early-steps",
                step_id="dispatch-parse",
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capability="python.parse_artifacts",
            ).model_dump(mode="json"),
        )
    ]


def test_execution_queue_bus_uses_configured_execution_request_topic() -> None:
    bus = ExecutionQueueBus(topics=_DEV_TOPICS)

    bus.publish_request(
        ExecutionQueueRequest(
            request_id="req-bridge",
            workflow_id="wf-bridge",
            step_id="step-bridge",
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capability="python.parse_artifacts",
            policy={},
            constraints={},
            payload={"module": "modelado.executors.loaders"},
            transport={"kind": "redpanda", "request_topic": "execution.requests"},
        )
    )

    assert bus.messages == [
        (
            "dev.execution.requests",
            {
                "request_id": "req-bridge",
                "workflow_id": "wf-bridge",
                "step_id": "step-bridge",
                "executor_id": "executor://python-primary",
                "executor_kind": "python-executor",
                "capability": "python.parse_artifacts",
                "policy": {},
                "constraints": {},
                "payload": {"module": "modelado.executors.loaders"},
                "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
            },
        ),
        (
            "dev.workflow.events",
            ExecutionQueued(
                request_id="req-bridge",
                workflow_id="wf-bridge",
                step_id="step-bridge",
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capability="python.parse_artifacts",
            ).model_dump(mode="json"),
        )
    ]


def test_petri_engine_publishes_execution_queue_request_when_bus_present(monkeypatch: pytest.MonkeyPatch) -> None:
    class StubDispatcher:
        def publish_execution_queue_request(self, **kwargs: object) -> ExecutionQueued:
            bus = kwargs["bus"]
            request = ExecutionQueueRequest(
                request_id="req-engine",
                workflow_id="wf-engine",
                step_id="dispatch",
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capability="python.parse_artifacts",
                policy={},
                constraints={},
                payload={"module": "modelado.executors.loaders"},
                transport={"kind": "redpanda", "request_topic": "execution.requests"},
            )
            bus.publish_request(request)
            return ExecutionQueued(
                request_id="req-engine",
                workflow_id="wf-engine",
                step_id="dispatch",
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capability="python.parse_artifacts",
                status="queued",
            )

        def compile_and_execute(self, **_: object) -> object:
            raise AssertionError("sidecar path should not run when execution_queue_bus is present")

    stub_module = types.ModuleType("modelado.executors")
    stub_module.ExecutionDispatcher = StubDispatcher
    monkeypatch.setitem(sys.modules, "modelado.executors", stub_module)

    bus = ExecutionQueueBus()
    engine = PetriNetEngine(
        net_envelope=PetriNetEnvelope(
            project_id="project",
            scope_id="scope",
            title="title",
            goal="goal",
            initial_marking_fragment_id="marking:start",
        ),
        transitions={
            "dispatch": PetriNetTransition(
                transition_id="dispatch",
                label="dispatch",
                operation_ref="op.dispatch",
                inputs=[PetriNetArcEndpoint(place_id="place:start")],
                outputs=[PetriNetArcEndpoint(place_id="place:end")],
                metadata={"output_key": "dispatch_result"},
            )
        },
        env=OperatorEnv(
            seed=1,
            renderer_version="test",
            policy="test",
            env_scope=EnvironmentScope(ref="refs/heads/run/env-1"),
            slots={"execution_queue_bus": bus},
        ),
    )

    new_marking, _ = engine.fire(
        "dispatch",
        PetriNetMarking(tokens={"place:start": 1}, meta={}),
        transition_fragment_id="transition-fragment",
    )

    assert new_marking.meta["dispatch_result"] == ExecutionQueued(
        request_id="req-engine",
        workflow_id="wf-engine",
        step_id="dispatch",
        executor_id="executor://python-primary",
        executor_kind="python-executor",
        capability="python.parse_artifacts",
        status="queued",
    ).model_dump(mode="json")
    assert bus.messages == [
        (
            "execution.requests",
            {
                "request_id": "req-engine",
                "workflow_id": "wf-engine",
                "step_id": "dispatch",
                "executor_id": "executor://python-primary",
                "executor_kind": "python-executor",
                "capability": "python.parse_artifacts",
                "policy": {},
                "constraints": {},
                "payload": {"module": "modelado.executors.loaders"},
                "transport": {"kind": "redpanda", "request_topic": "execution.requests"},
            },
        ),
        (
            "workflow.events",
            ExecutionQueued(
                request_id="req-engine",
                workflow_id="wf-engine",
                step_id="dispatch",
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capability="python.parse_artifacts",
            ).model_dump(mode="json"),
        )
    ]
