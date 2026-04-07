from __future__ import annotations

from interacciones.schemas.execution import ExecutionRequest, ResolutionMode
from interacciones.schemas.executors import ExecutorDeclaration


class WorkflowExecutorResolver:
    def __init__(self, declarations: list[ExecutorDeclaration]) -> None:
        self._declarations = declarations

    def resolve(self, request: ExecutionRequest) -> ExecutorDeclaration:
        if request.resolution_mode is ResolutionMode.DIRECT_EXECUTOR_REF:
            if request.direct_executor_ref is None:
                raise ValueError("direct_executor_ref is required for direct executor resolution")
            for declaration in self._declarations:
                if declaration.executor_id == request.direct_executor_ref:
                    return declaration
            raise ValueError(f"direct executor '{request.direct_executor_ref}' is not declared")

        for declaration in self._declarations:
            if request.capability in declaration.capabilities:
                return declaration
        raise ValueError(f"no executor declaration supports capability '{request.capability}'")
