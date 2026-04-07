from __future__ import annotations

import pytest

from modelado.operators import core as operator_core
from modelado.operators.registry import create_default_operator_registry
from modelado.registry import OperatorRegistryAdapter


class _InMemoryManager:
    def __init__(self) -> None:
        self.entries: dict[tuple[str, str], object] = {}

    def append_put(self, _cx: object, namespace: str, key: str, value: object, *, base_version: int | None = None) -> None:
        del base_version
        self.entries[(namespace, key)] = value

    def get(self, _cx: object, namespace: str, key: str) -> object | None:
        return self.entries.get((namespace, key))

    def list_keys(self, _cx: object, namespace: str) -> list[str]:
        return sorted(key for entry_namespace, key in self.entries if entry_namespace == namespace)


class MockOperator:
    pass


class StatefulMockOperator:
    def __init__(self) -> None:
        self.flag = "initial"


def test_operator_registry_adapter_round_trips_operator_descriptor_metadata(
) -> None:
    manager = _InMemoryManager()
    registry = OperatorRegistryAdapter(object(), manager, "operators.plan")

    assert hasattr(operator_core, "OperatorDescriptor")

    descriptor = operator_core.OperatorDescriptor(
        operator=MockOperator(),
        operator_ref="modelado/operators/mock",
        capabilities=("capability:alpha", "capability:beta"),
        selection_policy={"executor": "preferred"},
    )

    registry.register("mock", descriptor)

    entry = registry.get("mock")

    assert isinstance(entry, operator_core.OperatorDescriptor)
    assert isinstance(entry.operator, MockOperator)
    assert entry.operator_ref == "modelado/operators/mock"
    assert entry.capabilities == ("capability:alpha", "capability:beta")
    assert entry.selection_policy == {"executor": "preferred"}


def test_default_operator_registry_accepts_descriptor_overrides(
    monkeypatch,
) -> None:
    assert hasattr(operator_core, "OperatorDescriptor")

    override = operator_core.OperatorDescriptor(
        operator=MockOperator(),
        operator_ref="modelado/operators/mock-override",
        capabilities=("capability:override",),
        selection_policy="policy/mock",
    )

    created: list[_FakeRegistry] = []

    class _FakeRegistry:
        def __init__(self, _cx: object, _manager: object, namespace: str) -> None:
            self.namespace = namespace
            self.entries: dict[str, object] = {}
            created.append(self)

        def list_keys(self) -> list[str]:
            return sorted(self.entries)

        def register(self, key: str, entry: object) -> None:
            self.entries[key] = entry

        def get(self, key: str) -> object | None:
            return self.entries.get(key)

    monkeypatch.setattr("modelado.operators.registry.OperatorRegistryAdapter", _FakeRegistry)

    registry = create_default_operator_registry(object(), object(), namespace="operators.default.test", overrides={"modelado/operators/identify": override})

    override_entry = registry.get("modelado/operators/identify")
    default_entry = registry.get("modelado/operators/lift")

    assert isinstance(override_entry, operator_core.OperatorDescriptor)
    assert isinstance(override_entry.operator, MockOperator)
    assert override_entry.operator_ref == "modelado/operators/mock-override"
    assert override_entry.capabilities == ("capability:override",)
    assert override_entry.selection_policy == "policy/mock"
    assert default_entry is not None
    assert default_entry.__class__.__name__ == "LiftOperator"


def test_operator_registry_adapter_round_trips_descriptor_operator_instance_state() -> None:
    manager = _InMemoryManager()
    registry = OperatorRegistryAdapter(object(), manager, "operators.plan")

    operator = StatefulMockOperator()
    operator.flag = "persisted"
    descriptor = operator_core.OperatorDescriptor(
        operator=operator,
        operator_ref="modelado/operators/stateful",
        capabilities=("capability:stateful",),
        selection_policy={"executor": "preferred"},
    )

    registry.register("stateful", descriptor)

    entry = registry.get("stateful")

    assert isinstance(entry, operator_core.OperatorDescriptor)
    assert isinstance(entry.operator, StatefulMockOperator)
    assert entry.operator.flag == "persisted"


def test_operator_descriptor_rejects_non_json_selection_policy() -> None:
    with pytest.raises(ValueError, match="JSON-serializable"):
        operator_core.OperatorDescriptor(
            operator=MockOperator(),
            operator_ref="modelado/operators/bad-policy",
            selection_policy={"bad": {1, 2, 3}},
        )
