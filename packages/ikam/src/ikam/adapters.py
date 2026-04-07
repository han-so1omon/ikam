"""
IKAM Domain↔Storage Adapters

V3 Fragment Algebra adapters. Provides:
- V3 Fragment ↔ StorageFragment conversion
- Fragment object manifest construction and normalization

Guarantees:
- Deterministic CAS IDs: same (value, mime_type) → same cas_id
- Lossless round-trip: v3_fragment_from_cas_bytes(v3_fragment_to_cas_bytes(F)).value = F.value

Version: 2.0.0 (V3 Fragment Algebra - February 2026)
"""

from __future__ import annotations

import json
from typing import Any, Dict, List, Optional, Sequence

from ikam.fragments import Fragment as V3Fragment
from ikam.graph import StoredFragment, _cas_hex


class AdapterError(Exception):
    """Base exception for adapter errors."""
    pass


class SerializationError(AdapterError):
    """Raised when content serialization fails."""
    pass


class DeserializationError(AdapterError):
    """Raised when content deserialization fails."""
    pass


# ============================================================================
# Fragment Object Manifests
# ============================================================================


def build_fragment_object_manifest(
    *,
    artifact_id: str,
    kind: str,
    fragment_ids: Optional[Sequence[str]] = None,
) -> Dict[str, Any]:
    """Build an immutable fragment object manifest for an artifact.

    The manifest is a canonical, ordered record of fragment references used to
    reconstruct artifact views without mutable meta/content tables.
    """
    entries: List[Dict[str, Any]] = []

    if fragment_ids:
        entries = [{"fragmentId": fid} for fid in fragment_ids]

    return {
        "schemaVersion": 1,
        "artifactId": artifact_id,
        "kind": kind,
        "fragments": entries,
    }


def normalize_fragment_object_entries(manifest: Dict[str, Any]) -> List[Dict[str, Any]]:
    """Return normalized manifest fragment entries.

    Supports legacy manifests containing bare fragment id strings.
    """
    raw_entries = manifest.get("fragments") if isinstance(manifest, dict) else None
    if not isinstance(raw_entries, list):
        return []
    normalized: List[Dict[str, Any]] = []
    for item in raw_entries:
        if isinstance(item, str):
            normalized.append({"fragmentId": item})
            continue
        if isinstance(item, dict):
            fragment_id = item.get("fragmentId") or item.get("fragment_id") or item.get("id")
            if not fragment_id:
                continue
            entry = dict(item)
            entry["fragmentId"] = fragment_id
            normalized.append(entry)
    return normalized


def extract_fragment_ids_from_manifest(manifest: Dict[str, Any]) -> List[str]:
    entries = normalize_fragment_object_entries(manifest)
    return [entry["fragmentId"] for entry in entries if entry.get("fragmentId")]


def serialize_fragment_object_manifest(manifest: Dict[str, Any]) -> bytes:
    try:
        payload = json.dumps(manifest, sort_keys=True, ensure_ascii=False, separators=(",", ":"))
    except Exception as exc:
        raise SerializationError(f"Failed to serialize fragment object manifest: {exc}") from exc
    return payload.encode("utf-8")


def fragment_object_id_for_manifest(manifest: Dict[str, Any]) -> str:
    return _cas_hex(serialize_fragment_object_manifest(manifest))


def empty_manifest(kind: str) -> dict:
    """Identity element ε for manifest monoid. Contract: B1."""
    return {
        "schemaVersion": 1,
        "artifactId": "",
        "kind": kind,
        "fragments": [],
    }


def compose_manifests(m1: dict, m2: dict) -> dict:
    """Binary operator ⊕ for manifest monoid. Contract: B1.

    Concatenates fragment entries from m1 and m2. Requires compatible `kind`.
    The result inherits `artifactId` from m1 (or m2 if m1 is identity).
    """
    if m1["kind"] != m2["kind"]:
        raise ValueError(
            f"Cannot compose manifests with incompatible kind: "
            f"{m1['kind']!r} vs {m2['kind']!r}"
        )
    artifact_id = m1["artifactId"] or m2["artifactId"]
    return {
        "schemaVersion": 1,
        "artifactId": artifact_id,
        "kind": m1["kind"],
        "fragments": m1["fragments"] + m2["fragments"],
    }


# ============================================================================
# V3 Fragment Adapters (MIME-based, no type-switch)
# ============================================================================


def v3_fragment_to_cas_bytes(fragment: V3Fragment) -> bytes:
    """Serialize a V3 Fragment to deterministic CAS bytes.

    Strategy: Build a canonical dict from (mime_type, value), serialize to
    sorted JSON, encode to UTF-8.  MIME type is included so identical values
    with different MIME types produce distinct CAS identities.

    For CAS-only fragments (no value), the cas_id alone is serialized.
    """
    canonical: Dict[str, Any] = {}
    if fragment.mime_type is not None:
        canonical["mime_type"] = fragment.mime_type
    if fragment.value is not None:
        canonical["value"] = fragment.value
    elif fragment.cas_id is not None:
        canonical["cas_id"] = fragment.cas_id
    stable_json = json.dumps(canonical, sort_keys=True, ensure_ascii=False)
    return stable_json.encode("utf-8")


def v3_fragment_from_cas_bytes(
    *,
    cas_id: str,
    payload: bytes,
) -> V3Fragment:
    """Reconstruct a V3 Fragment from CAS bytes produced by v3_fragment_to_cas_bytes.

    Args:
        cas_id: The CAS identity (blake3 hex of payload).
        payload: The canonical bytes.

    Returns:
        V3 Fragment with cas_id set and value/mime_type restored.
    """
    try:
        data = json.loads(payload.decode("utf-8"))
    except Exception as exc:
        raise DeserializationError(f"V3 fragment payload is not valid JSON: {exc}") from exc
    if not isinstance(data, dict):
        raise DeserializationError("V3 fragment payload must be a JSON object")
    return V3Fragment(
        cas_id=cas_id,
        value=data.get("value"),
        mime_type=data.get("mime_type"),
    )


def v3_to_storage(fragment: V3Fragment) -> StoredFragment:
    """Convert a V3 Fragment to a StorageFragment (CAS layer).

    No type-switch is required — the MIME type is carried through directly.
    """
    cas_bytes = v3_fragment_to_cas_bytes(fragment)
    cas_id = _cas_hex(cas_bytes)
    return StoredFragment(
        id=cas_id,
        bytes=cas_bytes,
        mime_type=fragment.mime_type or "application/octet-stream",
        size=len(cas_bytes),
    )


__all__ = [
    "AdapterError",
    "SerializationError",
    "DeserializationError",
    "build_fragment_object_manifest",
    "normalize_fragment_object_entries",
    "extract_fragment_ids_from_manifest",
    "serialize_fragment_object_manifest",
    "fragment_object_id_for_manifest",
    "empty_manifest",
    "compose_manifests",
    "v3_fragment_to_cas_bytes",
    "v3_fragment_from_cas_bytes",
    "v3_to_storage",
]
