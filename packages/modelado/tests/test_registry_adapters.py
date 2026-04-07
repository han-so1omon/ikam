from __future__ import annotations

import psycopg
import uuid
from modelado.registry import RegistryManager
from modelado.registry.adapters import OperatorRegistryAdapter, ReasoningRegistryAdapter


def _namespace(prefix: str) -> str:
    return f"{prefix}.{uuid.uuid4().hex}"

def mock_fn(*_args, **_kwargs):
    return {"nodes": [], "edges": []}

class MockBFS:
    pass

class MockPlan:
    pass

class MockObj:
    pass

def test_reasoning_adapter_matches_legacy_registry_shape(db_connection: psycopg.Connection) -> None:
    manager = RegistryManager()
    registry = ReasoningRegistryAdapter(db_connection, manager, _namespace("reasoning.search_strategies"))

    registry.register("mock", mock_fn)

    assert "mock" in registry
    assert registry.get("mock") is mock_fn
    assert registry["mock"] is mock_fn
    assert registry.list_keys() == ["mock"]


def test_operator_adapter_uses_independent_namespace(db_connection: psycopg.Connection) -> None:
    manager = RegistryManager()
    reasoning = ReasoningRegistryAdapter(db_connection, manager, _namespace("reasoning.search_strategies"))
    operators = OperatorRegistryAdapter(db_connection, manager, _namespace("operators.plan"))

    reasoning.register("default", MockBFS)
    operators.register("default", MockPlan)

    assert reasoning.get("default") is MockBFS
    assert operators.get("default") is MockPlan


def test_reasoning_adapter_clear_removes_all_entries(db_connection: psycopg.Connection) -> None:
    manager = RegistryManager()
    registry = ReasoningRegistryAdapter(db_connection, manager, _namespace("reasoning.search_strategies"))
    
    registry.register("bfs", MockObj)
    registry.register("dfs", MockObj)

    registry.clear()

    assert registry.list_keys() == []
