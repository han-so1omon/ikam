from __future__ import annotations

import json
from typing import Any, Callable


class PostgresTracePromotionOutboxSink:
    def __init__(self, connection_scope_fn: Any, writer: Callable[[dict[str, Any]], str | None] | None = None) -> None:
        self._connection_scope = connection_scope_fn
        self._writer = writer

    def record(self, plan: dict[str, Any]) -> None:
        fragment_id = self._writer(dict(plan)) if self._writer is not None else None
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    INSERT INTO workflow_trace_promotion_outbox (
                        workflow_id,
                        run_id,
                        payload,
                        committed_trace_fragment_id,
                        created_at
                    )
                    VALUES (%s, %s, %s::jsonb, %s, NOW())
                    """,
                    (
                        plan["workflow_id"],
                        plan.get("run_id"),
                        json.dumps(plan),
                        fragment_id,
                    ),
                )
            cx.commit()

    def list_records(self, *, workflow_id: str) -> list[dict[str, Any]]:
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    SELECT payload, committed_trace_fragment_id
                    FROM workflow_trace_promotion_outbox
                    WHERE workflow_id = %s
                    ORDER BY outbox_id ASC
                    """,
                    (workflow_id,),
                )
                rows = cur.fetchall()
        records: list[dict[str, Any]] = []
        for row in rows:
            payload = row[0] if isinstance(row, tuple) else row["payload"]
            fragment_id = row[1] if isinstance(row, tuple) else row["committed_trace_fragment_id"]
            if isinstance(payload, str):
                payload = json.loads(payload)
            record = dict(payload)
            if fragment_id is not None:
                record["committed_trace_fragment_id"] = fragment_id
            records.append(record)
        return records

    def claim_pending(self, *, limit: int, lease_owner: str, lease_expires_at: str) -> list[dict[str, Any]]:
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    WITH claimable AS (
                        SELECT outbox_id
                        FROM workflow_trace_promotion_outbox
                        WHERE processed_at IS NULL
                          AND (lease_expires_at IS NULL OR lease_expires_at <= NOW())
                        ORDER BY outbox_id ASC
                        FOR UPDATE SKIP LOCKED
                        LIMIT %s
                    )
                    UPDATE workflow_trace_promotion_outbox AS outbox
                    SET lease_owner = %s, lease_expires_at = %s::timestamptz
                    FROM claimable
                    WHERE outbox.outbox_id = claimable.outbox_id
                    RETURNING outbox.outbox_id, outbox.payload, outbox.committed_trace_fragment_id, outbox.lease_owner
                    """,
                    (limit, lease_owner, lease_expires_at),
                )
                rows = cur.fetchall()
            cx.commit()
        claimed: list[dict[str, Any]] = []
        for row in rows:
            payload = row[1] if isinstance(row, tuple) else row["payload"]
            fragment_id = row[2] if isinstance(row, tuple) else row["committed_trace_fragment_id"]
            outbox_id = row[0] if isinstance(row, tuple) else row["outbox_id"]
            current_lease_owner = row[3] if isinstance(row, tuple) else row["lease_owner"]
            if isinstance(payload, str):
                payload = json.loads(payload)
            record = dict(payload)
            record["outbox_id"] = outbox_id
            record["lease_owner"] = current_lease_owner
            if fragment_id is not None:
                record["committed_trace_fragment_id"] = fragment_id
            claimed.append(record)
        return claimed

    def ack(self, outbox_id: int, *, lease_owner: str) -> None:
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    UPDATE workflow_trace_promotion_outbox
                    SET processed_at = NOW()
                    WHERE outbox_id = %s AND lease_owner = %s
                    """,
                    (outbox_id, lease_owner),
                )
            cx.commit()

    def renew_lease(self, outbox_id: int, *, lease_owner: str, lease_expires_at: str) -> None:
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    UPDATE workflow_trace_promotion_outbox
                    SET lease_expires_at = %s::timestamptz
                    WHERE outbox_id = %s AND lease_owner = %s AND processed_at IS NULL
                    """,
                    (lease_expires_at, outbox_id, lease_owner),
                )
            cx.commit()

    def release_lease(self, outbox_id: int, *, lease_owner: str) -> None:
        with self._connection_scope() as cx:
            with cx.cursor() as cur:
                cur.execute(
                    """
                    UPDATE workflow_trace_promotion_outbox
                    SET lease_owner = NULL, lease_expires_at = NULL
                    WHERE outbox_id = %s AND lease_owner = %s AND processed_at IS NULL
                    """,
                    (outbox_id, lease_owner),
                )
            cx.commit()
