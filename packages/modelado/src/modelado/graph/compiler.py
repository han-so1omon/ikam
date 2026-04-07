from __future__ import annotations

from interacciones.schemas import ExecutorDeclaration, RichPetriWorkflow, SourceWorkflowStorageMode
from ikam.ir.core import EvidenceRef, EvidenceRole, PropositionIR, StructuredDataIR

from .ir import LoweredExecutableGraph
from .lowering_petri import lower_rich_petri_transition


class GraphCompiler:
    def compile(
        self,
        workflow: RichPetriWorkflow,
        *,
        persist_source_workflow: bool | None = None,
        executor_declarations: list[ExecutorDeclaration] | None = None,
        support_inputs: dict[str, object] | None = None,
    ) -> LoweredExecutableGraph:
        validated_workflow = RichPetriWorkflow.model_validate(workflow.model_dump(mode="python"))
        validated_declarations = [
            ExecutorDeclaration.model_validate(declaration.model_dump(mode="python"))
            if isinstance(declaration, ExecutorDeclaration)
            else ExecutorDeclaration.model_validate(declaration)
            for declaration in (executor_declarations or [])
        ]
        operators = [
            lower_rich_petri_transition(
                workflow_id=validated_workflow.workflow_id,
                version=validated_workflow.version,
                transition=transition,
                executor_declarations=validated_declarations,
            )
            for transition in validated_workflow.transitions
        ]
        executable_graph = StructuredDataIR(
            artifact_id=f"artifact:graph:{validated_workflow.workflow_id}:{validated_workflow.version}",
            fragment_id=f"graph:{validated_workflow.workflow_id}:{validated_workflow.version}",
            profile="ikam_executable_graph",
            data={
                "workflow_id": validated_workflow.workflow_id,
                "version": validated_workflow.version,
                "place_ids": [place.place_id for place in validated_workflow.places],
                "operator_fragment_ids": [operator.fragment_id for operator in operators],
                "arc_count": len(validated_workflow.arcs),
                "reconstructable_from_rich_petri": True,
                "rich_petri_snapshot": validated_workflow.model_dump(mode="json"),
                "source_workflow_storage_mode": validated_workflow.source_workflow_storage.mode.value,
                "executor_declaration_ids": [declaration.executor_id for declaration in validated_declarations],
                "support_inputs": dict(support_inputs or {}),
                "publish": [target.model_dump(mode="json") for target in validated_workflow.publish],
            },
        )
        graph_edges = [
            PropositionIR(
                artifact_id=executable_graph.artifact_id,
                fragment_id=f"graph-edge:{validated_workflow.workflow_id}:{validated_workflow.version}:{index}",
                profile="ikam_executable_graph_arc",
                statement={
                    "subject": arc.source_id,
                    "predicate": f"{arc.source_kind}_to_{arc.target_kind}",
                    "object": arc.target_id,
                },
                evidence_refs=[EvidenceRef(fragment_id=executable_graph.fragment_id or "", role=EvidenceRole.PRIMARY)],
            )
            for index, arc in enumerate(validated_workflow.arcs, start=1)
        ]

        should_persist_source = (
            persist_source_workflow
            if persist_source_workflow is not None
            else validated_workflow.source_workflow_storage.mode is not SourceWorkflowStorageMode.OMIT
        )
        source_workflow = None
        source_graph_link = None
        if should_persist_source:
            source_workflow = StructuredDataIR(
                artifact_id=f"artifact:source:{validated_workflow.workflow_id}:{validated_workflow.version}",
                fragment_id=f"source:{validated_workflow.workflow_id}:{validated_workflow.version}",
                profile="rich_petri_workflow",
                data=validated_workflow.model_dump(mode="json"),
            )
            source_graph_link = PropositionIR(
                artifact_id=source_workflow.artifact_id,
                fragment_id=f"source-link:{validated_workflow.workflow_id}:{validated_workflow.version}",
                profile="ikam_graph_derivation",
                statement={
                    "subject": source_workflow.fragment_id,
                    "predicate": "lowered_to_executable_graph",
                    "object": executable_graph.fragment_id,
                },
                evidence_refs=[EvidenceRef(fragment_id=executable_graph.fragment_id or "")],
            )

        return LoweredExecutableGraph(
            executable_graph=executable_graph,
            operators=operators,
            graph_edges=graph_edges,
            source_workflow=source_workflow,
            source_graph_link=source_graph_link,
        )
