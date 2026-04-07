from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))
sys.path.insert(0, str(ROOT / "packages/ikam/src"))

from interacciones.schemas import ExecutorDeclaration, RichPetriArc, RichPetriPlace, RichPetriTransition, RichPetriWorkflow
from ikam.ir.core import ExpressionIR, PropositionIR, StructuredDataIR
from modelado.graph.compiler import GraphCompiler


def test_graph_compiler_lowers_rich_petri_to_ikam_native_fragments() -> None:
    workflow = RichPetriWorkflow(
        workflow_id="ingestion",
        version="v1",
        places=[
            RichPetriPlace(place_id="place:start", label="place:start"),
            RichPetriPlace(place_id="place:end", label="place:end"),
        ],
        transitions=[
            RichPetriTransition(
                transition_id="transition:normalize",
                label="normalize",
                capability="python.transform",
                policy={"tier": "standard"},
            )
        ],
        arcs=[
            RichPetriArc(source_kind="place", source_id="place:start", target_kind="transition", target_id="transition:normalize"),
            RichPetriArc(source_kind="transition", source_id="transition:normalize", target_kind="place", target_id="place:end"),
        ],
    )

    compiled = GraphCompiler().compile(workflow)

    assert isinstance(compiled.executable_graph, StructuredDataIR)
    assert [type(fragment) for fragment in compiled.fragments] == [
        StructuredDataIR,
        ExpressionIR,
        PropositionIR,
        PropositionIR,
        StructuredDataIR,
        PropositionIR,
    ]
    assert [operator.ast.params["transition_id"] for operator in compiled.operators] == ["transition:normalize"]
    assert compiled.executable_graph.data["operator_fragment_ids"] == [compiled.operators[0].fragment_id]


def test_graph_compiler_persists_source_workflow_by_default_and_links_it() -> None:
    workflow = RichPetriWorkflow(
        workflow_id="approval-flow",
        version="v1",
        places=[
            RichPetriPlace(place_id="place:start", label="place:start"),
            RichPetriPlace(place_id="place:end", label="place:end"),
        ],
        transitions=[
            RichPetriTransition(
                transition_id="transition:approve",
                label="approve",
                approval_hint={"node_kind": "request_approval"},
            )
        ],
        arcs=[
            RichPetriArc(source_kind="place", source_id="place:start", target_kind="transition", target_id="transition:approve"),
            RichPetriArc(source_kind="transition", source_id="transition:approve", target_kind="place", target_id="place:end"),
        ],
    )

    compiled = GraphCompiler().compile(workflow)

    assert compiled.source_workflow is not None
    assert compiled.source_workflow.profile == "rich_petri_workflow"
    assert compiled.source_graph_link is not None
    assert compiled.source_graph_link.statement == {
        "subject": compiled.source_workflow.fragment_id,
        "predicate": "lowered_to_executable_graph",
        "object": compiled.executable_graph.fragment_id,
    }


def test_graph_compiler_can_omit_source_workflow_and_keep_reconstructable_graph() -> None:
    workflow = RichPetriWorkflow(
        workflow_id="single-node",
        version="v1",
        places=[
            RichPetriPlace(place_id="place:start", label="place:start"),
            RichPetriPlace(place_id="place:end", label="place:end"),
        ],
        transitions=[
            RichPetriTransition(
                transition_id="transition:complete",
                label="complete",
                checkpoint_hint={"node_kind": "complete"},
            )
        ],
        arcs=[
            RichPetriArc(source_kind="place", source_id="place:start", target_kind="transition", target_id="transition:complete"),
            RichPetriArc(source_kind="transition", source_id="transition:complete", target_kind="place", target_id="place:end"),
        ],
    )

    compiled = GraphCompiler().compile(workflow, persist_source_workflow=False)

    assert compiled.source_workflow is None
    assert compiled.source_graph_link is None
    assert compiled.executable_graph.data["reconstructable_from_rich_petri"] is True
    assert compiled.executable_graph.data["rich_petri_snapshot"]["workflow_id"] == "single-node"


def test_graph_compiler_records_declaration_and_support_inputs() -> None:
    workflow = RichPetriWorkflow(
        workflow_id="dispatch-flow",
        version="v1",
        places=[
            RichPetriPlace(place_id="place:start", label="place:start"),
            RichPetriPlace(place_id="place:end", label="place:end"),
        ],
        transitions=[
            RichPetriTransition(
                transition_id="transition:dispatch",
                label="dispatch",
                capability="python.transform",
            )
        ],
        arcs=[
            RichPetriArc(source_kind="place", source_id="place:start", target_kind="transition", target_id="transition:dispatch"),
            RichPetriArc(source_kind="transition", source_id="transition:dispatch", target_kind="place", target_id="place:end"),
        ],
    )
    declarations = [
        ExecutorDeclaration(
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capabilities=["python.transform", "python.fetch"],
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
    ]

    compiled = GraphCompiler().compile(
        workflow,
        executor_declarations=declarations,
        support_inputs={"operator_registry_version": "2026-03-06"},
    )

    assert compiled.operators[0].ast.params["eligible_executor_ids"] == ["executor://python-primary"]
    assert compiled.executable_graph.data["executor_declaration_ids"] == [
        "executor://python-primary",
        "executor://ml-primary",
    ]
    assert compiled.executable_graph.data["support_inputs"] == {"operator_registry_version": "2026-03-06"}


def test_graph_compiler_namespaces_fragment_ids_by_workflow_version() -> None:
    workflow_v1 = RichPetriWorkflow(
        workflow_id="dispatch-flow",
        version="v1",
        places=[
            RichPetriPlace(place_id="place:start", label="place:start"),
            RichPetriPlace(place_id="place:end", label="place:end"),
        ],
        transitions=[
            RichPetriTransition(
                transition_id="transition:dispatch",
                label="dispatch",
                capability="python.transform",
            )
        ],
        arcs=[
            RichPetriArc(source_kind="place", source_id="place:start", target_kind="transition", target_id="transition:dispatch"),
            RichPetriArc(source_kind="transition", source_id="transition:dispatch", target_kind="place", target_id="place:end"),
        ],
    )
    workflow_v2 = workflow_v1.model_copy(update={"version": "v2"})

    compiled_v1 = GraphCompiler().compile(workflow_v1)
    compiled_v2 = GraphCompiler().compile(workflow_v2)

    assert compiled_v1.operators[0].fragment_id != compiled_v2.operators[0].fragment_id
    assert compiled_v1.graph_edges[0].fragment_id != compiled_v2.graph_edges[0].fragment_id
