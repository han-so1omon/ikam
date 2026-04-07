from __future__ import annotations

from typing import Any, Callable, Protocol


class TracePromotionSink(Protocol):
    def record(self, plan: dict[str, Any]) -> None: ...


class InMemoryTracePromotionSink:
    def __init__(self) -> None:
        self.plans: list[dict[str, Any]] = []

    def record(self, plan: dict[str, Any]) -> None:
        self.plans.append(dict(plan))


class IkamTracePromotionSink:
    def __init__(self, *, writer: Callable[[dict[str, Any]], str | None]) -> None:
        self._writer = writer
        self.records: list[dict[str, Any]] = []

    def record(self, plan: dict[str, Any]) -> None:
        fragment_id = self._writer(dict(plan))
        record = dict(plan)
        if fragment_id is not None:
            record["committed_trace_fragment_id"] = fragment_id
        self.records.append(record)
