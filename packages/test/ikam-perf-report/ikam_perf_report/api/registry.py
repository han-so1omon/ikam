from __future__ import annotations

import json
from typing import Any

from fastapi import APIRouter, HTTPException

from modelado.registry import get_shared_registry_manager
from ikam_perf_report.db.session import open_connection

router = APIRouter(prefix="/registry", tags=["registry"])

@router.get("")
def list_namespaces():
    try:
        with open_connection() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    SELECT DISTINCT value->>'registry_name'
                    FROM ikam_fragment_store
                    WHERE value->>'profile' = 'registry'
                    """
                )
                namespaces = [row[0] for row in cur.fetchall() if row[0]]
                return {"namespaces": namespaces}
    except Exception as exc:
        raise HTTPException(status_code=500, detail=str(exc))

@router.get("/{namespace}")
def get_registry(namespace: str):
    try:
        with open_connection() as cx:
            manager = get_shared_registry_manager()
            projection = manager.snapshot(cx, namespace)
            
            # Format entries into a list for the UI
            entries = []
            for key, value in projection.entries.items():
                entry = {"key": key}
                if isinstance(value, dict):
                    entry.update(value)
                else:
                    entry["value"] = value
                entries.append(entry)
            
            # Sort by registered_at if available
            entries.sort(key=lambda x: x.get("registered_at", ""), reverse=True)
            return {"namespace": namespace, "version": projection.version, "entries": entries}
    except Exception as exc:
        raise HTTPException(status_code=500, detail=str(exc))

@router.get("/subgraph/{head_fragment_id}")
def get_subgraph(head_fragment_id: str):
    try:
        with open_connection() as cx:
            with cx.cursor() as cur:
                # 1. Fetch head fragment
                cur.execute(
                    """
                    SELECT value
                    FROM ikam_fragment_store
                    WHERE cas_id = %s OR value->>'fragment_id' = %s
                    LIMIT 1
                    """,
                    (head_fragment_id, head_fragment_id)
                )
                row = cur.fetchone()
                if not row or not row[0]:
                    raise HTTPException(status_code=404, detail="Head fragment not found")
                
                head_value = row[0]
                data = head_value.get("data", {})
                
                # Extract child CAS IDs (we look for generic patterns but also petri-specific ones)
                child_ids = set()
                for key, val in data.items():
                    if key.endswith(("_cas_ids", "_fragment_ids")) and isinstance(val, list):
                        child_ids.update(val)
                    elif key.endswith(("_cas_id", "_fragment_id")) and isinstance(val, str):
                        child_ids.add(val)
                        
                # 2. Bulk fetch child fragments
                children = {}
                if child_ids:
                    cur.execute(
                        """
                        SELECT cas_id, value
                        FROM ikam_fragment_store
                        WHERE cas_id = ANY(%s)
                        """,
                        (list(child_ids),)
                    )
                    for child_row in cur.fetchall():
                        children[child_row[0]] = child_row[1]
                        
                # 3. Assemble response
                return {
                    "head": head_value,
                    "children": children
                }
    except HTTPException:
        raise
    except Exception as exc:
        raise HTTPException(status_code=500, detail=str(exc))
