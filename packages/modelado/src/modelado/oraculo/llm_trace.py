"""LLM request/response tracing helpers for debugging."""
from __future__ import annotations

import json
import os
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def _truncate(value: Any, *, max_chars: int = 800) -> Any:
    if isinstance(value, str):
        return value if len(value) <= max_chars else value[: max_chars - 3] + "..."
    if isinstance(value, list):
        return [_truncate(item, max_chars=max_chars) for item in value[:20]]
    if isinstance(value, dict):
        out: dict[str, Any] = {}
        for index, (key, item) in enumerate(value.items()):
            if index >= 30:
                break
            out[str(key)] = _truncate(item, max_chars=max_chars)
        return out
    return value


def _trace_file_path() -> Path:
    raw = os.getenv("IKAM_LLM_TRACE_FILE", "/tmp/ikam-llm-trace.jsonl").strip()
    return Path(raw)


def emit_llm_trace(
    *,
    provider: str,
    operation: str,
    model: str,
    phase: str,
    request_payload: dict[str, Any] | None = None,
    response_payload: dict[str, Any] | None = None,
    metadata: dict[str, Any] | None = None,
) -> None:
    record = {
        "timestamp": datetime.now(timezone.utc).isoformat(),
        "provider": provider,
        "operation": operation,
        "model": model,
        "phase": phase,
        "thread_id": os.getenv("IKAM_TRACE_THREAD_ID", ""),
        "artifact_id": os.getenv("IKAM_TRACE_ARTIFACT_ID", ""),
        "request": _truncate(request_payload or {}),
        "response": _truncate(response_payload or {}),
        "metadata": _truncate(metadata or {}),
    }
    path = _trace_file_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(record, ensure_ascii=True) + "\n")
