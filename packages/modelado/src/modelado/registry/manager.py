from __future__ import annotations

import json
from copy import deepcopy
from typing import Any, Optional

import psycopg
from psycopg.abc import Query

from .events import RegistryEvent, RegistryOp, utc_now_iso
from .projection import RegistryProjection
from modelado.ikam_graph_repository import store_fragment
from ikam.fragments import Fragment as DomainFragment
from ikam.graph import _cas_hex


class RegistryConflictError(ValueError):
    pass


class RegistryManager:
    def __init__(self) -> None:
        # In-memory events are removed. State is read/written to DB.
        self._next_event_id = 1

    def append_put(
        self,
        cx: Optional[psycopg.Connection[Any]],
        namespace: str,
        key: str,
        value: Any,
        *,
        base_version: int | None = None,
    ) -> RegistryEvent:
        if cx is None:
            raise ValueError("RegistryManager.append_put requires a valid database connection (cx)")
        return self._append_event(
            cx=cx,
            namespace=namespace,
            key=key,
            op="put",
            value=value,
            base_version=base_version,
        )

    def append_delete(
        self,
        cx: Optional[psycopg.Connection[Any]],
        namespace: str,
        key: str,
        *,
        base_version: int | None = None,
    ) -> RegistryEvent:
        if cx is None:
            raise ValueError("RegistryManager.append_delete requires a valid database connection (cx)")
        return self._append_event(
            cx=cx,
            namespace=namespace,
            key=key,
            op="delete",
            value=None,
            base_version=base_version,
        )

    def get(self, cx: Optional[psycopg.Connection[Any]], namespace: str, key: str) -> Any:
        if cx is None:
            raise ValueError("RegistryManager.get requires a valid database connection (cx)")
        projection = self._get_projection(cx, namespace)
        return projection.entries.get(key)

    def list_keys(self, cx: Optional[psycopg.Connection[Any]], namespace: str) -> list[str]:
        if cx is None:
            raise ValueError("RegistryManager.list_keys requires a valid database connection (cx)")
        projection = self._get_projection(cx, namespace)
        return sorted(projection.entries.keys())

    def current_version(self, cx: Optional[psycopg.Connection[Any]], namespace: str) -> int:
        if cx is None:
            raise ValueError("RegistryManager.current_version requires a valid database connection (cx)")
        return self._get_projection(cx, namespace).version

    def snapshot(self, cx: Optional[psycopg.Connection[Any]], namespace: str) -> RegistryProjection:
        if cx is None:
            raise ValueError("RegistryManager.snapshot requires a valid database connection (cx)")
        projection = self._get_projection(cx, namespace)
        return RegistryProjection(
            namespace=projection.namespace,
            version=projection.version,
            entries=deepcopy(projection.entries),
        )

    def _append_event(
        self,
        *,
        cx: psycopg.Connection[Any],
        namespace: str,
        key: str,
        op: RegistryOp,
        value: Any,
        base_version: int | None,
    ) -> RegistryEvent:
        if not namespace:
            raise ValueError("registry namespace is required")
        if not key:
            raise ValueError("registry key is required")

        projection = self._get_projection(cx, namespace)
        expected_base = projection.version
        if base_version is not None and base_version != expected_base:
            raise RegistryConflictError(
                f"registry conflict in namespace {namespace}: base_version={base_version}, "
                f"current_version={expected_base}"
            )

        event = RegistryEvent(
            event_id=self._next_event_id,
            namespace=namespace,
            key=key,
            op=op,
            value=value,
            version=expected_base + 1,
            base_version=base_version,
            timestamp=utc_now_iso(),
        )
        self._next_event_id += 1
        
        projection.apply(event)
        self._save_projection(cx, projection)
        return event

    def _get_projection(self, cx: psycopg.Connection[Any], namespace: str) -> RegistryProjection:
        # Load the latest fragment from ikam_fragment_store for this registry namespace
        with cx.cursor() as cur:
            cur.execute(
                """
                SELECT value
                FROM ikam_fragment_store
                WHERE value->>'profile' = 'registry'
                  AND value->>'registry_name' = %s
                ORDER BY created_at DESC, (value->>'version')::int DESC
                LIMIT 1
                """,
                (namespace,)
            )
            row = cur.fetchone()
        
        if row:
            data = row["value"] if isinstance(row, dict) else row[0]
            if data:
                entries = data.get("entries", {})
                version = data.get("version", 0)
                return RegistryProjection(namespace=namespace, version=version, entries=entries)
        
        return RegistryProjection(namespace=namespace)

    def _save_projection(self, cx: psycopg.Connection[Any], projection: RegistryProjection) -> None:
        data = {
            "profile": "registry",
            "registry_name": projection.namespace,
            "version": projection.version,
            "entries": projection.entries
        }
        
        # Serialize to JSON to compute CAS hash
        json_bytes = json.dumps(data, sort_keys=True).encode("utf-8")
        cas_id = _cas_hex(json_bytes)
        
        fragment = DomainFragment(
            cas_id=cas_id,
            value=data,
            mime_type="application/ikam-registry+json"
        )
        store_fragment(
            cx,
            fragment,
            project_id="global_registry",
            ref="refs/heads/main",
            operation_id="registry_update",
        )
