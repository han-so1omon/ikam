from __future__ import annotations

from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Any, Literal


RegistryOp = Literal["put", "delete"]


@dataclass(frozen=True)
class RegistryEvent:
    event_id: int
    namespace: str
    key: str
    op: RegistryOp
    version: int
    timestamp: str
    base_version: int | None = None
    value: Any = None


def utc_now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()
