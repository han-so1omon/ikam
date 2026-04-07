from __future__ import annotations

import psycopg
import pytest
import uuid

from modelado.registry import RegistryConflictError, RegistryEvent, RegistryManager
from modelado.registry.projection import RegistryProjection


def _namespace(prefix: str) -> str:
    return f"{prefix}.{uuid.uuid4().hex}"


def test_append_put_appends_event_and_updates_projection(db_connection: psycopg.Connection) -> None:
    manager = RegistryManager()
    namespace = _namespace("reasoning.search_strategies")

    event = manager.append_put(
        db_connection,
        namespace=namespace,
        key="default",
        value="bfs",
    )

    assert isinstance(event, RegistryEvent)
    assert event.op == "put"
    assert event.version == 1
    assert manager.current_version(db_connection, namespace) == 1
    assert manager.get(db_connection, namespace, "default") == "bfs"


def test_amend_conflict_raises_on_stale_base_version(db_connection: psycopg.Connection) -> None:
    manager = RegistryManager()
    namespace = _namespace("reasoning.search_strategies")
    manager.append_put(db_connection, namespace, "default", "bfs")

    with pytest.raises(RegistryConflictError):
        manager.append_put(
            db_connection,
            namespace=namespace,
            key="default",
            value="dfs",
            base_version=0,
        )


def test_namespace_lookup_isolated(db_connection: psycopg.Connection) -> None:
    manager = RegistryManager()
    reasoning_namespace = _namespace("reasoning.search_strategies")
    operator_namespace = _namespace("operators.plan")
    manager.append_put(db_connection, reasoning_namespace, "default", "bfs")
    manager.append_put(db_connection, operator_namespace, "default", "plan_operator")

    assert manager.get(db_connection, reasoning_namespace, "default") == "bfs"
    assert manager.get(db_connection, operator_namespace, "default") == "plan_operator"
    assert manager.list_keys(db_connection, reasoning_namespace) == ["default"]
    assert manager.list_keys(db_connection, operator_namespace) == ["default"]


def test_registry_projection_persists_to_canonical_main_ref(monkeypatch: pytest.MonkeyPatch) -> None:
    manager = RegistryManager()
    captured: dict[str, object] = {}

    def fake_store_fragment(cx, fragment, *, project_id, ref, operation_id):
        captured["project_id"] = project_id
        captured["ref"] = ref
        captured["operation_id"] = operation_id

    monkeypatch.setattr("modelado.registry.manager.store_fragment", fake_store_fragment)

    manager._save_projection(
        object(),
        RegistryProjection(namespace="reasoning.search_strategies", version=1, entries={"default": "bfs"}),
    )

    assert captured == {
        "project_id": "global_registry",
        "ref": "refs/heads/main",
        "operation_id": "registry_update",
    }
