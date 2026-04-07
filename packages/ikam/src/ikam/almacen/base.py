from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass
from enum import Enum, auto
from typing import Any, Dict, Iterable, Mapping, Optional, Protocol


class Capability(Enum):
    # Core operations
    PUT = auto()
    GET = auto()
    DELETE = auto()
    LIST = auto()

    # Optional: versioning and metadata
    VERSIONS = auto()
    METADATA_QUERY = auto()

    # Optional: content-addressable support (hash-based addressing)
    CAS = auto()


@dataclass(frozen=True)
class FragmentKey:
    """Stable identifier for a fragment payload.

    Prefer content-addressable keys (e.g., blake3 hash) for deduplication.
    """

    key: str  # opaque identifier; recommended format: "blake3:hexhash" or "uuid:..."
    kind: str  # semantic kind, e.g., "text", "image/png", "chart-spec", "patch/json"


@dataclass
class FragmentRecord:
    key: FragmentKey
    payload: bytes
    metadata: Dict[str, Any]


class StorageBackend(ABC):
    """Abstract storage backend.

    Backends are engine-specific implementations (e.g., PostgreSQL, S3/MinIO,
    filesystem). They should not encode any tier-specific logic. Register
    instances with BackendRegistry and wire them via TieredStorage.
    """

    name: str  # logical name (e.g., "postgresql", "s3", "fs")

    @property
    @abstractmethod
    def capabilities(self) -> frozenset[Capability]:
        """Capabilities supported by this backend."""

    @abstractmethod
    def put(self, record: FragmentRecord) -> FragmentKey:
        """Store or upsert a fragment payload; return the effective key."""

    @abstractmethod
    def get(self, key: FragmentKey) -> Optional[FragmentRecord]:
        """Fetch a fragment by key. Returns None if not found."""

    @abstractmethod
    def delete(self, key: FragmentKey) -> bool:
        """Delete a fragment by key. Returns True if deleted, False if missing."""

    @abstractmethod
    def list(self, prefix: Optional[str] = None) -> Iterable[FragmentKey]:
        """Iterate keys, optionally filtered by a string prefix."""

    # Optional hooks
    def describe(self) -> Mapping[str, Any]:
        return {
            "name": getattr(self, "name", self.__class__.__name__.lower()),
            "capabilities": [c.name for c in self.capabilities],
        }
