from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))
sys.path.insert(0, str(ROOT / "packages/ikam/src"))

from interacciones.schemas import ExecutorDeclaration, ResolutionMode, RichPetriTransition
from ikam.ir.core import OpType
from modelado.graph.lowering_petri import lower_rich_petri_transition


def test_lower_rich_petri_transition_creates_single_executable_operator() -> None:
    transition = RichPetriTransition(
        transition_id="transition:normalize",
        label="normalize",
        capability="python.transform",
        policy={"tier": "standard"},
        constraints={"locality": "local"},
    )

    operator = lower_rich_petri_transition(
        workflow_id="ingestion",
        version="v1",
        transition=transition,
    )

    assert operator.ast.op_type is OpType.REF
    assert operator.ast.params == {
        "workflow_id": "ingestion",
        "workflow_version": "v1",
        "transition_id": "transition:normalize",
        "label": "normalize",
        "capability": "python.transform",
        "policy": {"tier": "standard"},
        "constraints": {"locality": "local"},
        "resolution_mode": "capability_policy",
        "direct_executor_ref": None,
        "approval_hint": {},
        "checkpoint_hint": {},
        "eligible_executor_ids": [],
        "translator_plan": {"input": [], "output": []},
    }


def test_lower_rich_petri_transition_preserves_structural_hints_for_reconstruction() -> None:
    transition = RichPetriTransition(
        transition_id="transition:await-approval",
        label="await-approval",
        approval_hint={"node_kind": "wait_for_approval"},
        checkpoint_hint={"node_kind": "wait_for_approval"},
    )

    operator = lower_rich_petri_transition(
        workflow_id="approval-flow",
        version="v1",
        transition=transition,
    )

    assert operator.ast.params["approval_hint"] == {"node_kind": "wait_for_approval"}
    assert operator.ast.params["checkpoint_hint"] == {"node_kind": "wait_for_approval"}


def test_lower_rich_petri_transition_derives_eligible_executors_from_declarations() -> None:
    transition = RichPetriTransition(
        transition_id="transition:normalize",
        label="normalize",
        capability="python.transform",
    )

    operator = lower_rich_petri_transition(
        workflow_id="ingestion",
        version="v1",
        transition=transition,
        executor_declarations=[
            ExecutorDeclaration(
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capabilities=["python.transform"],
                policy_support=["cost_tier"],
                transport={"kind": "redpanda"},
                runtime={"image": "python:3.11"},
                concurrency={"max_inflight": 4},
                batching={"max_batch_size": 16},
                health={"readiness_path": "/health"},
            ),
            ExecutorDeclaration(
                executor_id="executor://ml-primary",
                executor_kind="ml-executor",
                capabilities=["ml.embed"],
                policy_support=["latency"],
                transport={"kind": "redpanda"},
                runtime={"image": "ml:latest"},
                concurrency={"max_inflight": 2},
                batching={"max_batch_size": 64},
                health={"readiness_path": "/health"},
            ),
        ],
    )

    assert operator.ast.params["eligible_executor_ids"] == ["executor://python-primary"]


def test_lower_rich_petri_transition_rejects_unknown_direct_executor_override() -> None:
    transition = RichPetriTransition(
        transition_id="transition:normalize",
        label="normalize",
        capability="python.transform",
        resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
        direct_executor_ref="executor://missing",
    )

    try:
        lower_rich_petri_transition(
            workflow_id="ingestion",
            version="v1",
            transition=transition,
            executor_declarations=[
                ExecutorDeclaration(
                    executor_id="executor://python-primary",
                    executor_kind="python-executor",
                    capabilities=["python.transform"],
                    policy_support=["cost_tier"],
                    transport={"kind": "redpanda"},
                    runtime={"image": "python:3.11"},
                    concurrency={"max_inflight": 4},
                    batching={"max_batch_size": 16},
                    health={"readiness_path": "/health"},
                )
            ],
        )
    except ValueError as exc:
        assert str(exc) == "direct executor override 'executor://missing' is not declared"
    else:
        raise AssertionError("expected missing direct executor override to be rejected")


def test_lower_rich_petri_transition_rejects_incompatible_direct_executor_override() -> None:
    transition = RichPetriTransition(
        transition_id="transition:normalize",
        label="normalize",
        capability="python.transform",
        resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
        direct_executor_ref="executor://ml-primary",
    )

    try:
        lower_rich_petri_transition(
            workflow_id="ingestion",
            version="v1",
            transition=transition,
            executor_declarations=[
                ExecutorDeclaration(
                    executor_id="executor://ml-primary",
                    executor_kind="ml-executor",
                    capabilities=["ml.embed"],
                    policy_support=["latency"],
                    transport={"kind": "redpanda"},
                    runtime={"image": "ml:latest"},
                    concurrency={"max_inflight": 2},
                    batching={"max_batch_size": 64},
                    health={"readiness_path": "/health"},
                )
            ],
        )
    except ValueError as exc:
        assert str(exc) == "direct executor override 'executor://ml-primary' does not support capability 'python.transform'"
    else:
        raise AssertionError("expected incompatible direct executor override to be rejected")
