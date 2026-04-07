from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Optional


@dataclass(frozen=True)
class ProjectionLag:
    project_id: str
    consumer_name: str
    latest_event_id: int
    checkpoint_event_id: int
    lag_events: int
    lag_ms: Optional[int]


def _table_exists(cx: Any, name: str) -> bool:
    row = cx.execute(
        """
        SELECT 1
          FROM information_schema.tables
         WHERE table_schema = 'public'
           AND table_name = %s
        """,
        (name,),
    ).fetchone()
    return bool(row)


def compute_projection_lag(cx: Any, *, consumer_name: str, project_id: str) -> ProjectionLag:
    """Compute projection lag between graph_edge_events and a consumer checkpoint.

    If the checkpoint table doesn't exist yet, lag is computed as if checkpoint=0.
    """

    latest_row = cx.execute(
        "SELECT COALESCE(MAX(id), 0) AS latest_id, COALESCE(MAX(t), 0) AS latest_t FROM graph_edge_events WHERE project_id = %s",
        (project_id,),
    ).fetchone()

    latest_id = int(latest_row["latest_id"] if latest_row else 0)
    latest_t = int(latest_row["latest_t"] if latest_row else 0)

    checkpoint_id = 0
    checkpoint_t = None

    if _table_exists(cx, "graph_edge_projection_checkpoints"):
        row = cx.execute(
            """
            SELECT last_event_id, updated_at
              FROM graph_edge_projection_checkpoints
                         WHERE project_id = %s AND consumer_name = %s
            """,
            (project_id, consumer_name),
        ).fetchone()
        if row:
            checkpoint_id = int(row["last_event_id"])
            checkpoint_t = int(row["updated_at"]) if row.get("updated_at") is not None else None

    lag_events = max(latest_id - checkpoint_id, 0)
    lag_ms = None
    if checkpoint_t is not None and latest_t:
        lag_ms = max(latest_t - checkpoint_t, 0)

    return ProjectionLag(
        project_id=project_id,
        consumer_name=consumer_name,
        latest_event_id=latest_id,
        checkpoint_event_id=checkpoint_id,
        lag_events=lag_events,
        lag_ms=lag_ms,
    )
