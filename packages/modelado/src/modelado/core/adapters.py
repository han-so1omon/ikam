"""Adapter contracts for modeling-related integrations.

These protocols allow packages such as ``narraciones_modeling`` to rely on
injected persistence, telemetry, and metadata functionality without importing
``narraciones_base_api`` directly. Services (base API, workers, MCP servers)
are expected to register concrete adapters during startup.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Dict, Iterable, Optional, Protocol

"""Adapter contracts for modeling-related integrations.

This module previously imported the telemetry adapter from
``narraciones.adapters.telemetry`` at module import time. After the package
consolidation refactor, importing telemetry eagerly created a circular import:

modelado.core.adapters -> narraciones.adapters.telemetry -> narraciones (__init__)
-> narraciones.models.story_model -> narraciones.adapters.modeling ->
modelado.core.adapters (partially initialized)

Because the import happened before the adapter protocol classes (e.g.
``EconomicPersistenceAdapter``) were defined, attempting to import them from the
partially initialized module failed. We fix this by deferring the telemetry
import until after the protocol/class definitions are executed, guaranteeing the
names exist when ``narraciones.adapters.modeling`` re-imports this module.
"""

from typing import TYPE_CHECKING

if TYPE_CHECKING:  # Only for type checking; avoids runtime circular import.
    from narraciones.adapters.telemetry import TelemetryAdapter  # pragma: no cover
else:
    # Define a minimal Protocol placeholder to satisfy the dataclass annotation
    # before we perform the real import later. We intentionally avoid importing
    # the real module here to break the cycle.
    from typing import Protocol  # noqa: WPS433 (local import for narrow scope)

    class TelemetryAdapter(Protocol):  # type: ignore[no-redef]
        def log_event(
            self,
            category: str,
            action: str,
            status: str,
            *,
            request_id: Optional[str] = None,  # type: ignore[name-defined]
            latency_ms: Optional[int] = None,  # type: ignore[name-defined]
            extra: Optional[Dict[str, Any]] = None,  # type: ignore[name-defined]
        ) -> None: ...


class PendingChangesAdapter(Protocol):
    """Interactions with the pending changes queue."""

    def create_change(
        self,
        scope: str,
        *,
        title: Optional[str] = None,
        summary: Optional[str] = None,
        payload: Optional[Dict[str, Any]] = None,
        documents: Optional[Dict[str, Any]] = None,
        meta: Optional[Dict[str, Any]] = None,
    ) -> Dict[str, Any]: ...

    def list_changes(self, scope: str) -> Iterable[Any]: ...

    def get_change(self, change_id: int) -> Optional[Any]: ...

    def mark_applied(self, change_id: int) -> Any: ...

    def mark_discarded(self, change_id: int) -> Any: ...


class ProjectMetaAdapter(Protocol):
    """Expose read/write access to project metadata records."""

    def get_project_meta(self) -> Dict[str, Any]: ...

    def update_project_meta(self, **fields: Any) -> None: ...


class EconomicPersistenceAdapter(Protocol):
    """Persistence hooks required by the economic modeling module."""

    def get_model_items(self) -> Iterable[Any]: ...

    def upsert_model_items(self, rows: Iterable[Any]) -> None: ...

    def derive_model_offerings(self) -> Iterable[Any]: ...

    def list_offerings(self) -> Iterable[Any]: ...

    def replace_offerings(self, rows: Iterable[Any]) -> None: ...

    def append_offering(self, row: Dict[str, Any]) -> None: ...

    def delete_offering(self, item_name: str) -> bool: ...

    def clear_offerings(self) -> int: ...

    def list_variables(self) -> Iterable[Any]: ...

    def upsert_variables(self, rows: Iterable[Any]) -> None: ...

    def delete_variable(self, key: str) -> None: ...

    def list_attribute_definitions(self, entity_kind: Optional[str] = None) -> Iterable[Any]: ...

    def upsert_attribute_definitions(self, rows: Iterable[Any]) -> None: ...

    def delete_attribute_definition(self, key: str) -> None: ...

    def list_formulas(self) -> Iterable[Any]: ...

    def upsert_formulas(self, rows: Iterable[Any]) -> None: ...

    def delete_formula(self, name: str) -> None: ...

    def list_relationships(self, kind: Optional[str] = None) -> Iterable[Any]: ...

    def replace_relationships(self, rows: Iterable[Any]) -> None: ...

    def upsert_relationship(self, row: Dict[str, Any]) -> int: ...

    def delete_relationship(self, rel_id: int) -> None: ...

    def clear_sheet_plan(self) -> None: ...

    def load_sheet_plan(self, sheet: Optional[str] = None) -> Optional[Dict[str, Any]]: ...

    def persist_sheet_plan(self, sheet: str, payload: Dict[str, Any]) -> None: ...


class StoryPersistenceAdapter(Protocol):
    """Persistence hooks used by the story modeling module."""

    def list_slides(self) -> Iterable[Dict[str, Any]]: ...

    def replace_slides(self, slides: Iterable[Dict[str, Any]]) -> None: ...

    def delete_slide(self, index: int) -> bool: ...

    def clear_slides(self) -> int: ...

    def clear_slides_plan(self) -> None: ...

    def persist_slides_plan(self, plan: Dict[str, Any]) -> None: ...

    def load_slides_plan(self) -> Optional[Dict[str, Any]]: ...

    def fetch_brand_palette(self) -> Dict[str, Any]: ...

    def update_brand_fields(self, fields: Dict[str, Any]) -> None: ...


@dataclass
class ModelingAdapters:
    """Container for all modeling-related adapters."""

    telemetry: TelemetryAdapter
    pending_changes: PendingChangesAdapter
    project_meta: ProjectMetaAdapter
    economic_persistence: EconomicPersistenceAdapter
    story_persistence: StoryPersistenceAdapter


class _MissingPendingChanges:
    def create_change(self, scope: str, *, title: Optional[str] = None, summary: Optional[str] = None, payload: Optional[Dict[str, Any]] = None, documents: Optional[Dict[str, Any]] = None, meta: Optional[Dict[str, Any]] = None) -> Dict[str, Any]:  # noqa: D401
        raise RuntimeError("PendingChangesAdapter not registered")

    def list_changes(self, scope: str) -> Iterable[Dict[str, Any]]:  # noqa: D401
        raise RuntimeError("PendingChangesAdapter not registered")

    def get_change(self, change_id: int) -> Optional[Dict[str, Any]]:  # noqa: D401
        raise RuntimeError("PendingChangesAdapter not registered")

    def mark_applied(self, change_id: int) -> Dict[str, Any]:  # noqa: D401
        raise RuntimeError("PendingChangesAdapter not registered")

    def mark_discarded(self, change_id: int) -> Dict[str, Any]:  # noqa: D401
        raise RuntimeError("PendingChangesAdapter not registered")


class _MissingProjectMeta:
    def get_project_meta(self) -> Dict[str, Any]:  # noqa: D401
        raise RuntimeError("ProjectMetaAdapter not registered")

    def update_project_meta(self, **fields: Any) -> None:  # noqa: D401
        raise RuntimeError("ProjectMetaAdapter not registered")


class _MissingEconomicPersistence:
    def __getattr__(self, name: str) -> Any:  # noqa: D401
        raise RuntimeError("EconomicPersistenceAdapter not registered")


class _MissingStoryPersistence:
    def __getattr__(self, name: str) -> Any:  # noqa: D401
        raise RuntimeError("StoryPersistenceAdapter not registered")


def _resolve_telemetry():
    """Import and return the global telemetry adapter.

    Done lazily to avoid circular imports during module initialisation.
    Falls back to a no-op adapter if the real module is unavailable.
    """

    try:  # Local import to defer dependency.
        from narraciones.adapters.telemetry import get_telemetry_adapter  # noqa: WPS433

        return get_telemetry_adapter()
    except Exception:  # noqa: BLE001 - safest fallback in early boot
        class _Noop:  # pragma: no cover - only exercised in misconfigured boots
            def log_event(self, *args: Any, **kwargs: Any) -> None:  # noqa: D401
                return None

        return _Noop()


_default_adapters = ModelingAdapters(
    telemetry=_resolve_telemetry(),
    pending_changes=_MissingPendingChanges(),
    project_meta=_MissingProjectMeta(),
    economic_persistence=_MissingEconomicPersistence(),
    story_persistence=_MissingStoryPersistence(),
)

_current_adapters: ModelingAdapters = _default_adapters


def register_modeling_adapters(adapters: ModelingAdapters) -> None:
    """Install concrete modeling adapters for the current process."""

    global _current_adapters
    _current_adapters = adapters


def get_modeling_adapters() -> ModelingAdapters:
    """Return the registered modeling adapters, or defaults if none provided."""

    return _current_adapters


__all__ = [
    "ModelingAdapters",
    "EconomicPersistenceAdapter",
    "StoryPersistenceAdapter",
    "PendingChangesAdapter",
    "ProjectMetaAdapter",
    "TelemetryAdapter",
    "get_modeling_adapters",
    "register_modeling_adapters",
]
