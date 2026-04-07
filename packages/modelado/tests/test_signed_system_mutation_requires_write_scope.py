import pytest

from modelado.core.execution_context import (
    ExecutionContext,
    ExecutionMode,
    ExecutionPolicyViolation,
    WriteScope,
    execution_context,
)


def test_record_signed_system_mutation_requires_write_scope() -> None:
    from modelado.ikam_graph_repository import record_signed_system_mutation_event

    with pytest.raises(ExecutionPolicyViolation):
        record_signed_system_mutation_event(  # type: ignore[arg-type]
            None,
            artifact_id="a",
            payload=None,
        )


def test_record_signed_system_mutation_denied_scope() -> None:
    from modelado.ikam_graph_repository import record_signed_system_mutation_event

    scope = WriteScope(allowed=False, project_id="p", operation="op")
    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="r1", write_scope=scope)):
        with pytest.raises(ExecutionPolicyViolation):
            record_signed_system_mutation_event(  # type: ignore[arg-type]
                None,
                artifact_id="a",
                payload=None,
            )


def test_record_signed_system_mutation_missing_required_fields() -> None:
    from modelado.ikam_graph_repository import record_signed_system_mutation_event

    scope = WriteScope(allowed=True, project_id="p", operation="op")
    with execution_context(ExecutionContext(mode=ExecutionMode.REQUEST, request_id="r1", write_scope=scope)):
        with pytest.raises(ValueError):
            record_signed_system_mutation_event(  # type: ignore[arg-type]
                None,
                artifact_id="a",
                payload=None,
            )
