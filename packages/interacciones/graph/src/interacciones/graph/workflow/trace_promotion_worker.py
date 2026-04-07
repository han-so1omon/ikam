from __future__ import annotations

from typing import Any


class TracePromotionWorker:
    def __init__(self, *, outbox_sink: Any, promotion_sink: Any) -> None:
        self._outbox_sink = outbox_sink
        self._promotion_sink = promotion_sink

    def run_once(self, *, limit: int, lease_owner: str, lease_expires_at: str) -> list[dict[str, Any]]:
        claimed = self._outbox_sink.claim_pending(
            limit=limit,
            lease_owner=lease_owner,
            lease_expires_at=lease_expires_at,
        )
        for item in claimed:
            self._promotion_sink.record(item)
            self._outbox_sink.ack(int(item["outbox_id"]), lease_owner=lease_owner)
        return [dict(item) for item in claimed]
