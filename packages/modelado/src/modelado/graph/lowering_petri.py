from __future__ import annotations

from interacciones.schemas import ExecutorDeclaration, RichPetriTransition
from ikam.ir.core import ExpressionIR, OpAST, OpType


def lower_rich_petri_transition(
    *,
    workflow_id: str,
    version: str,
    transition: RichPetriTransition,
    executor_declarations: list[ExecutorDeclaration] | None = None,
) -> ExpressionIR:
    if transition.resolution_mode.value == "direct_executor_ref" and executor_declarations is not None:
        declared_executor_ids = {declaration.executor_id for declaration in executor_declarations}
        if transition.direct_executor_ref not in declared_executor_ids:
            raise ValueError(f"direct executor override '{transition.direct_executor_ref}' is not declared")

    eligible_executor_ids = [
        declaration.executor_id
        for declaration in (executor_declarations or [])
        if transition.capability is not None and transition.capability in declaration.capabilities
    ]
    if transition.direct_executor_ref is not None and executor_declarations is not None:
        eligible_executor_ids = [transition.direct_executor_ref]
    return ExpressionIR(
        artifact_id=f"artifact:operator:{workflow_id}:{version}:{transition.transition_id}",
        fragment_id=f"operator:{workflow_id}:{version}:{transition.transition_id}",
        ast=OpAST(
            op_type=OpType.REF,
            params={
                "workflow_id": workflow_id,
                "workflow_version": version,
                "transition_id": transition.transition_id,
                "label": transition.label,
                "capability": transition.capability,
                "policy": transition.policy,
                "constraints": transition.constraints,
                "resolution_mode": transition.resolution_mode.value,
                "direct_executor_ref": transition.direct_executor_ref,
                "approval_hint": transition.approval_hint,
                "checkpoint_hint": transition.checkpoint_hint,
                "eligible_executor_ids": eligible_executor_ids,
            },
        ),
    )
