"""Shared pytest fixtures for modelado tests."""

import os
import sys
import uuid
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlparse

import pytest
from psycopg.rows import dict_row


_TESTS_DIR = Path(__file__).resolve().parent
_REPO_ROOT = _TESTS_DIR.parents[2]
if str(_TESTS_DIR) not in sys.path:
    sys.path.insert(0, str(_TESTS_DIR))
_MODELADO_SRC = _REPO_ROOT / "packages" / "modelado" / "src"
if str(_MODELADO_SRC) not in sys.path:
    sys.path.insert(0, str(_MODELADO_SRC))


@dataclass(frozen=True)
class _PostgresqlInfo:
    host: str
    port: int
    dbname: str
    user: str
    password: str


@dataclass(frozen=True)
class _PostgresqlFixture:
    info: _PostgresqlInfo


@pytest.fixture(scope="session")
def postgresql() -> _PostgresqlFixture:
    """Compatibility fixture for tests that expect `pytest-postgresql`.

    Our integration tests run against the dockerized Postgres instance and use
    env vars already wired into test runners.
    """

    database_url = (
        os.getenv("PYTEST_DATABASE_URL")
        or os.getenv("TEST_DATABASE_URL")
        or os.getenv("DATABASE_URL")
        or "postgresql://narraciones:narraciones@localhost:5432/narraciones"
    )
    parsed = urlparse(database_url)

    return _PostgresqlFixture(
        info=_PostgresqlInfo(
            host=parsed.hostname or "localhost",
            port=int(parsed.port or 5432),
            dbname=(parsed.path or "/").lstrip("/") or "postgres",
            user=parsed.username or "postgres",
            password=parsed.password or "",
        )
    )


@pytest.fixture(scope="function")
def db_connection():
    """Database connection with dict_row factory and transaction rollback after test."""
    import psycopg
    from modelado.db import ensure_schema

    db_url = (
        os.getenv("PYTEST_DATABASE_URL")
        or os.getenv("TEST_DATABASE_URL")
        or os.getenv("DATABASE_URL")
        or "postgresql://narraciones:narraciones@localhost:5432/narraciones"
    )
    ensure_schema()
    with psycopg.connect(db_url, row_factory=dict_row) as cx:
        project_id = f"proj_test_{uuid.uuid4()}"
        cx._test_project_id = project_id  # type: ignore[attr-defined]

        now_ms = int(__import__("time").time() * 1000)
        with cx.cursor() as cur:
            cur.execute(
                """
                INSERT INTO projects (id, title, snapshot, created_at, updated_at)
                VALUES (%s, %s, %s::jsonb, %s, %s)
                ON CONFLICT (id) DO NOTHING
                """,
                (project_id, "Test Project", "{}", now_ms, now_ms),
            )
        yield cx
        cx.rollback()  # Clean up after test


@pytest.fixture(autouse=True)
def _default_execution_context():
    """Ensure modelado tests always run under an ExecutionContext.

    IKAM repo-layer write guards require an active ExecutionContext. We default
    to a non-system actor so tests only require a signed WriteScope when they
    explicitly set actor_id=None.
    """

    from modelado.core.execution_context import ExecutionContext, ExecutionMode, execution_context, get_execution_context

    if get_execution_context() is not None:
        yield
        return

    with execution_context(
        ExecutionContext(mode=ExecutionMode.REQUEST, request_id="pytest-modelado", actor_id="pytest", purpose="pytest")
    ):
        yield


# ============================================================================
# Phase 9.7 Generative Extensions Fixtures
# ============================================================================

@pytest.fixture(autouse=True, scope="function")
def reset_generative_config():
    """Reset generative configuration before each test to ensure clean state."""
    from modelado.core.config import reset_config, ClockFactory
    
    # Reset configuration
    reset_config()
    
    # Ensure test mode is disabled initially
    ClockFactory.set_test_mode(False)
    
    yield
    
    # Clean up after test
    ClockFactory.set_test_mode(False)
    reset_config()


@pytest.fixture(scope="function")
def test_clock():
    """Create a StepClock for deterministic testing.
    
    Usage:
        def test_traversal(test_clock):
            clock = test_clock
            clock.tick()
            assert clock.current_step == 1
    """
    from modelado.core.config import StepClock, ClockFactory
    
    ClockFactory.set_test_mode(True)
    clock = StepClock()
    clock.step_duration_ms = 100
    
    yield clock
    
    ClockFactory.set_test_mode(False)


@pytest.fixture(scope="function")
def sample_execution_graph():
    """Create a sample execution graph for testing.
    
    Structure:
        root
        ├── child1
        │   └── grandchild
        └── child2
    
    Properties:
        - Depth: 3
        - Max fan-out: 2
        - No cycles
    """
    from modelado.core.execution_graph import ExecutionLinkGraph
    
    graph = ExecutionLinkGraph()
    graph.add_edge("root", "child1")
    graph.add_edge("root", "child2")
    graph.add_edge("child1", "grandchild")
    
    return graph


@pytest.fixture(scope="function")
def mock_ai_client():
    """Create a mock LLM client for testing without real API calls.
    
    Returns deterministic responses based on prompts for testing.
    """
    class MockAIClient:
        def __init__(self):
            self.call_count = 0
            self.responses = {
                "Generate function": "def example(): return 42",
                "Analyze data": "# Analysis results\nTotal: 100",
                "default": "# Generated code\npass"
            }
        
        async def call_async(self, prompt: str):
            self.call_count += 1
            from modelado.core.model_call_client import ModelCallResult
            
            # Return deterministic response
            text = self.responses.get(prompt, self.responses["default"])
            
            return ModelCallResult(
                text=text,
                model_name="mock-model",
                tokens_used=100,
                cost=0.001,
                latency_ms=50.0
            )
    
    return MockAIClient()


@pytest.fixture(scope="function")
def traversal_config_override():
    """Override traversal configuration for testing.
    
    Returns a dict of environment variable overrides.
    Usage with monkeypatch:
        def test_config(traversal_config_override, monkeypatch):
            for key, value in traversal_config_override.items():
                monkeypatch.setenv(key, value)
    """
    return {
        "GENERATIVE_TRAVERSAL_ENABLE_SPIKE_DECAY": "true",
        "GENERATIVE_TRAVERSAL_SPIKE_MULTIPLIER": "2.0",
        "GENERATIVE_TRAVERSAL_DECAY_MULTIPLIER": "0.8",
        "GENERATIVE_TRAVERSAL_BASE_STEP_DURATION_MS": "100",
        "GENERATIVE_POLICY_MAX_DEPTH": "3",
        "GENERATIVE_POLICY_MAX_FANOUT": "10",
        "GENERATIVE_POLICY_ALLOW_CYCLES": "false"
    }


@pytest.fixture(scope="function")
def batch_queue_factory():
    """Factory for creating batch queues with custom capacity.
    
    Usage:
        def test_queue(batch_queue_factory):
            queue = batch_queue_factory(capacity=10)
            queue.enqueue({"id": 1})
    """
    from modelado.core.batch_queue import BatchQueue
    
    def create_queue(capacity: int = 10):
        return BatchQueue(capacity=capacity)
    
    return create_queue


@pytest.fixture(scope="function")
def provenance_tracker():
    """Create a traversal provenance tracker for testing.
    
    Usage:
        def test_provenance(provenance_tracker):
            provenance_tracker.record_step("step1", 100)
            assert provenance_tracker.to_dict()["total_duration_ms"] == 100
    """
    from modelado.core.traversal_provenance import TraversalProvenance
    
    return TraversalProvenance()


# ============================================================================
# Test Markers
# ============================================================================

def pytest_configure(config):
    """Register custom markers for Phase 9.7 tests."""
    config.addinivalue_line(
        "markers",
        "llm_integration: tests requiring real LLM API calls (use GENERATIVE_ENABLE_LLM_TESTS=1)"
    )
    config.addinivalue_line(
        "markers",
        "slow: tests that take >1 second (use -m 'not slow' to skip)"
    )
    config.addinivalue_line(
        "markers",
        "e2e: end-to-end workflow tests requiring full stack"
    )


def pytest_collection_modifyitems(config, items):
    """Skip LLM integration tests unless explicitly enabled."""
    enable_llm_tests = os.getenv("GENERATIVE_ENABLE_LLM_TESTS", "0") == "1"
    
    skip_llm = pytest.mark.skip(reason="LLM integration tests disabled (set GENERATIVE_ENABLE_LLM_TESTS=1 to enable)")
    
    for item in items:
        if "llm_integration" in item.keywords and not enable_llm_tests:
            item.add_marker(skip_llm)
