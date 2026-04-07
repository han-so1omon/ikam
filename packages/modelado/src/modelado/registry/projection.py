from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from .events import RegistryEvent


@dataclass
class RegistryProjection:
    namespace: str
    version: int = 0
    entries: dict[str, Any] = field(default_factory=dict)

    def apply(self, event: RegistryEvent) -> None:
        if event.namespace != self.namespace:
            return
        if event.version != self.version + 1:
            raise ValueError(
                f"event version out of sequence for namespace {self.namespace}: "
                f"expected {self.version + 1}, got {event.version}"
            )
        if event.op == "put":
            self.entries[event.key] = event.value
        elif event.op == "delete":
            self.entries.pop(event.key, None)
        else:
            raise ValueError(f"unsupported registry op: {event.op}")
        self.version = event.version
