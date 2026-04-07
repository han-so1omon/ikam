"""Engine-agnostic storage abstraction for IKAM fragments.

This subpackage defines:
- StorageBackend: minimal abstract interface for storage engines
- BackendRegistry: pluggable backend discovery/registration
- PostgresBackend: PostgreSQL implementation with CAS support
- chunking: Document chunking utilities for fragmentation

Note: This is a small, dependency-free interface layer. Concrete engines
(PostgreSQL, S3/MinIO, Filesystem, SQLite, etc.) can live in sibling
modules/packages and register themselves via BackendRegistry.
"""

from .base import StorageBackend, FragmentKey, FragmentRecord, Capability
from .registry import BackendRegistry
from .artifact_store import ArtifactStore  # noqa: F401
from .provenance_store import GroundedArtifact, ProvenanceGraphStore  # noqa: F401
from .provenance_backend import ProvenanceBackend  # noqa: F401

# Optional imports with graceful degradation
try:
    from .postgres import PostgresBackend
    _has_postgres = True
except ImportError:
    _has_postgres = False
    PostgresBackend = None  # type: ignore

try:
    from .chunking import EmbeddingProvider
    _has_chunking = True
except ImportError:
    _has_chunking = False
    EmbeddingProvider = None  # type: ignore

try:
    from .metrics import record_storage_observation, validate_delta_monotonicity
    _has_metrics = True
except ImportError:
    _has_metrics = False
    record_storage_observation = None  # type: ignore
    validate_delta_monotonicity = None  # type: ignore

__all__ = [
    "StorageBackend",
    "FragmentKey",
    "FragmentRecord",
    "Capability",
    "BackendRegistry",
    "EmbeddingProvider",
    "ProvenanceBackend",
    "PostgresBackend",
    "record_storage_observation",
    "validate_delta_monotonicity",
]
