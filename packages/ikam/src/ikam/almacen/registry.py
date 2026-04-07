from __future__ import annotations

from dataclasses import dataclass
from typing import Dict, Optional

from .base import StorageBackend


@dataclass
class BackendRegistry:
    """Simple in-memory registry for storage backends.

    Engines register themselves with a logical name. TieredStorage can then
    resolve the engine for each tier via configuration.
    """

    _backends: Dict[str, StorageBackend]

    def __init__(self) -> None:
        self._backends = {}

    def register(self, backend: StorageBackend) -> None:
        if backend.name in self._backends:
            raise ValueError(f"Backend '{backend.name}' already registered")
        self._backends[backend.name] = backend

    def get(self, name: str) -> StorageBackend:
        try:
            return self._backends[name]
        except KeyError as e:
            raise KeyError(f"Backend '{name}' is not registered") from e

    def list(self) -> Dict[str, str]:
        return {name: b.__class__.__name__ for name, b in self._backends.items()}
