from __future__ import annotations

from typing import Any


class InMemoryWorkflowTraceStore:
    def __init__(self) -> None:
        self._events: list[dict[str, Any]] = []

    def append(self, event: dict[str, Any]) -> None:
        self._events.append(dict(event))

    def list_events(self, *, workflow_id: str, run_id: str | None = None) -> list[dict[str, Any]]:
        return [
            dict(event)
            for event in self._events
            if event.get("workflow_id") == workflow_id and (run_id is None or event.get("run_id") == run_id)
        ]
