from __future__ import annotations

import os
from contextlib import contextmanager
from pathlib import Path
import sys

import pytest

try:
    import psycopg
except ImportError:  # pragma: no cover - environment dependent
    psycopg = None


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/interacciones/graph/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.graph.workflow.orchestrator import WorkflowOrchestrator
from interacciones.graph.workflow.state_store import InMemoryWorkflowStateStore
from interacciones.graph.workflow.trace_promotion_sink_postgres import PostgresTracePromotionOutboxSink
from interacciones.graph.workflow.trace_store import InMemoryWorkflowTraceStore
from interacciones.schemas import TracePersistenceMode, TracePersistencePolicy
from interacciones.schemas.execution import ExecutionFailed, ExecutionRequest
from interacciones.schemas.executors import ExecutorDeclaration


pytestmark = pytest.mark.skipif(
    psycopg is None or not os.getenv("TEST_DATABASE_URL"),
    reason="psycopg or TEST_DATABASE_URL not available",
)


def _declarations() -> list[ExecutorDeclaration]:
    return [
        ExecutorDeclaration(
            executor_id="executor://python-primary",
            executor_kind="python-executor",
            capabilities=["python.parse_artifacts"],
            policy_support=["cost_tier"],
            transport={"kind": "redpanda", "request_topic": "execution.requests"},
            runtime={"language": "python", "version": "3.11"},
            concurrency={"max_inflight": 4},
            batching={"max_batch_size": 16},
            health={"readiness_path": "/health"},
        )
    ]


@pytest.fixture
def connection_scope():
    database_url = os.environ["TEST_DATABASE_URL"]

    @contextmanager
    def _scope():
        assert psycopg is not None
        with psycopg.connect(database_url) as conn:
            yield conn

    return _scope


@pytest.fixture
def sink(connection_scope):
    schema_sql = (ROOT / "packages/interacciones/graph/schema_trace_promotion_outbox.sql").read_text(encoding="utf-8")
    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute("DROP TABLE IF EXISTS workflow_trace_promotion_outbox")
            cur.execute(schema_sql)
            cur.execute("DELETE FROM workflow_trace_promotion_outbox WHERE workflow_id LIKE 'test-wf-%'")
        cx.commit()

    yield PostgresTracePromotionOutboxSink(connection_scope)

    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute("DELETE FROM workflow_trace_promotion_outbox WHERE workflow_id LIKE 'test-wf-%'")
        cx.commit()


def test_workflow_orchestrator_flushes_trace_promotions_into_postgres_outbox(
    sink: PostgresTracePromotionOutboxSink,
) -> None:
    orchestrator = WorkflowOrchestrator(
        _declarations(),
        state_store=InMemoryWorkflowStateStore(),
        trace_store=InMemoryWorkflowTraceStore(),
        trace_policy=TracePersistencePolicy(mode=TracePersistenceMode.ON_FAILURE),
        trace_promotion_sink=sink,
    )

    orchestrator.dispatch_execution(
        ExecutionRequest(
            request_id="req-promote-db-1",
            workflow_id="test-wf-promote-1",
            step_id="dispatch-parse",
            capability="python.parse_artifacts",
            policy={"cost_tier": "standard"},
            constraints={},
            payload={"input": "hello"},
        )
    )
    orchestrator.handle_execution_failed(
        ExecutionFailed(
            request_id="req-promote-db-1",
            workflow_id="test-wf-promote-1",
            step_id="dispatch-parse",
            executor_id="executor://python-primary",
            error_code="executor_timeout",
            error_message="timed out",
            retryable=False,
        )
    )

    flushed = orchestrator.flush_trace_promotions()

    assert len(flushed) == 1
    assert sink.list_records(workflow_id="test-wf-promote-1") == flushed
