from __future__ import annotations

from typing import Any, Callable

from service import SharedQueueExecutor


class PythonExecutorService(SharedQueueExecutor):
    def __init__(
        self,
        *,
        executor_id: str,
        publisher: Any,
        handlers: dict[str, Callable[[dict[str, Any]], dict[str, Any]]],
    ) -> None:
        super().__init__(
            executor_id=executor_id,
            executor_kind="python-executor",
            publisher=publisher,
            handlers=handlers,
        )
