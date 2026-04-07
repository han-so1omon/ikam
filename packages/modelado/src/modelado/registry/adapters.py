from __future__ import annotations

import importlib
import json
from typing import Any, Generic, TypeVar, Optional
import psycopg

from .manager import RegistryManager
from modelado.operators.core import OperatorDescriptor

T = TypeVar("T")

_SHARED_MANAGER = RegistryManager()

def get_shared_registry_manager() -> RegistryManager:
    return _SHARED_MANAGER

def _serialize_callable(entry: Any) -> dict[str, Any]:
    if isinstance(entry, OperatorDescriptor):
        return {
            "type": "operator_descriptor",
            "operator": _serialize_callable(entry.operator),
            "operator_ref": entry.operator_ref,
            "capabilities": list(entry.capabilities),
            "selection_policy": entry.selection_policy,
        }
    if hasattr(entry, "__class__") and type(entry) is not type and type(entry) is not type(lambda: None):
        # Instance of a class
        module_name = entry.__class__.__module__
        class_name = entry.__class__.__name__
        state = getattr(entry, "__dict__", None)
        if state is not None:
            try:
                json.dumps(state)
            except TypeError as exc:
                raise ValueError(f"Cannot serialize instance state for {module_name}.{class_name}") from exc
        return {"type": "class_instance_ref", "path": f"{module_name}.{class_name}", "state": state}
    elif hasattr(entry, "__module__") and hasattr(entry, "__name__"):
        # Function or Class
        return {"type": "callable_ref", "path": f"{entry.__module__}.{entry.__name__}"}
    raise ValueError(f"Cannot serialize entry of type {type(entry)}: {entry}")

def _deserialize_callable(data: Any) -> Any:
    if not isinstance(data, dict):
        return data
    ref_type = data.get("type")
    if ref_type == "operator_descriptor":
        return OperatorDescriptor(
            operator=_deserialize_callable(data.get("operator")),
            operator_ref=data["operator_ref"],
            capabilities=tuple(data.get("capabilities", ())),
            selection_policy=data.get("selection_policy"),
        )
    path = data.get("path")
    if ref_type in ("class_instance_ref", "callable_ref") and path:
        module_name, obj_name = path.rsplit(".", 1)
        mod = importlib.import_module(module_name)
        obj = getattr(mod, obj_name)
        if ref_type == "class_instance_ref":
            instance = obj()
            state = data.get("state")
            if isinstance(state, dict) and hasattr(instance, "__dict__"):
                instance.__dict__.update(state)
            return instance
        return obj
    return data


class _RegistryAdapter(Generic[T]):
    def __init__(self, cx: psycopg.Connection[Any], manager: RegistryManager, namespace: str):
        self._cx = cx
        self._manager = manager
        self._namespace = namespace

    def register(self, key: str, entry: T, *, base_version: int | None = None) -> None:
        serialized = _serialize_callable(entry)
        self._manager.append_put(self._cx, self._namespace, key, serialized, base_version=base_version)

    def get(self, key: str, default: Any = None) -> T | None:
        val = self._manager.get(self._cx, self._namespace, key)
        if val is None:
            return default
        return _deserialize_callable(val)

    def list_keys(self) -> list[str]:
        return self._manager.list_keys(self._cx, self._namespace)

    def clear(self) -> None:
        keys = self.list_keys()
        for key in keys:
            self._manager.append_delete(self._cx, self._namespace, key)

    def __contains__(self, key: str) -> bool:
        return self.get(key) is not None

    def __getitem__(self, key: str) -> T:
        value = self.get(key)
        if value is None:
            raise KeyError(key)
        return value

class ReasoningRegistryAdapter(_RegistryAdapter[T]):
    pass

class OperatorRegistryAdapter(_RegistryAdapter[T]):
    pass
