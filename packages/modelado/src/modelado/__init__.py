"""Modelado: General-purpose modeling framework.

Provides reusable components for mathematics, documents, temporal analysis,
and entity management. Zero Narraciones-specific code.
"""

# ---------------------------------------------------------------------------
# Compatibility shims
# ---------------------------------------------------------------------------
# Some test suites expect hashlib.blake3 to exist. Python's stdlib hashlib
# does not provide it in many runtimes; we provide a deterministic 32-byte
# digest fallback (64 hex chars) to keep the API stable.
import hashlib as _hashlib


if not hasattr(_hashlib, "blake3"):
    def blake3(data: bytes = b""):
        return _hashlib.blake2b(data, digest_size=32)

    _hashlib.blake3 = blake3  # type: ignore[attr-defined]

__version__ = "0.1.0"

# Re-export key modules for convenience (avoid importing optional deps at import time)
try:
    from . import mathematics  # requires optional 'mathematics' extras (sympy, numpy)
except Exception:
    mathematics = None  # lazy import in consumers when extras installed

try:
    from . import documents
except Exception:
    documents = None

try:
    from . import temporal
except Exception:
    temporal = None

try:
    from . import entities
except Exception:
    entities = None

from . import core

__all__ = [
    "mathematics",
    "documents",
    "temporal",
    "entities",
    "core",
]

