from __future__ import annotations

from dataclasses import dataclass
from hashlib import sha1
from time import perf_counter
from typing import Any, Dict, List, Tuple
from uuid import uuid4

from ikam.forja.cas import cas_fragment
from ikam.fragments import RELATION_MIME


@dataclass
class IkamFlowResult:
    fragments: List[Any]
    duration_ms: int
    reconstructed: str


def decompose_and_reconstruct(text: str, artifact_id: str) -> IkamFlowResult:
    start = perf_counter()
    fragment = cas_fragment({"text": text, "artifact_id": artifact_id}, "text/markdown")
    reconstructed = text
    elapsed_ms = max(1, int((perf_counter() - start) * 1000))
    return IkamFlowResult(fragments=[fragment], duration_ms=elapsed_ms, reconstructed=reconstructed)


def _frag_id(fragment: Any) -> str:
    """Return a stable identifier for a fragment (V3 domain or storage)."""
    # V3 domain Fragment uses cas_id; storage Fragment uses id
    return getattr(fragment, "cas_id", None) or getattr(fragment, "id", None) or f"anon-{uuid4().hex[:8]}"


def _frag_type_from_mime(mime_type: str | None) -> str:
    """Derive a short type label from MIME type."""
    if mime_type == RELATION_MIME:
        return "relation"
    if mime_type and mime_type.startswith("text/"):
        return "text"
    return "binary"


def _frag_label(fragment: Any, frag_id: str) -> str:
    """Derive a human-readable label for search/explainability."""
    value = getattr(fragment, "value", None)
    if isinstance(value, str):
        text = value.strip()
        return text[:120] if text else frag_id
    if isinstance(value, dict):
        for key in ("summary", "title", "text", "content", "name"):
            raw = value.get(key)
            if isinstance(raw, str) and raw.strip():
                return raw.strip()[:120]
    return frag_id


def _artifact_node_id(label: str) -> str:
    digest = sha1(label.encode("utf-8")).hexdigest()[:12]
    return f"artifact:{digest}"


def fragments_to_graph(
    fragments: List[Any],
    labels_by_fragment_id: Dict[str, str] | None = None,
    filenames_by_label: Dict[str, str] | None = None,
) -> Tuple[List[Dict[str, Any]], List[Dict[str, Any]], List[Dict[str, Any]]]:
    """Build graph nodes, edges, and manifests from V3 fragments.

    V3 fragments carry only ``cas_id``, ``value``, and ``mime_type``.
    Hierarchy is encoded in relation fragments (``RELATION_MIME``).

    Emits:
    - One fragment node per non-relation fragment.
    - Edges extracted from relation fragment values.
    - One manifest containing all content fragments in order.

    Returns ``(nodes, edges, manifests)``.
    """
    nodes: List[Dict[str, Any]] = []
    edges: List[Dict[str, Any]] = []
    content_entries: List[Dict[str, Any]] = []
    artifact_id_by_label: Dict[str, str] = {}

    for position, fragment in enumerate(fragments):
        frag_id = _frag_id(fragment)
        mime = getattr(fragment, "mime_type", None) or "application/octet-stream"

        if mime == RELATION_MIME:
            # Relation fragments encode parent→child edges in their value.
            rel = fragment.value if isinstance(fragment.value, dict) else {}
            source = rel.get("source", frag_id)
            target = rel.get("target", frag_id)
            edges.append(
                {
                    "id": f"edge-{uuid4().hex}",
                    "source": source,
                    "target": target,
                    "label": rel.get("predicate", "related"),
                }
            )
            continue

        ftype = _frag_type_from_mime(mime)
        label = None
        if labels_by_fragment_id:
            label = labels_by_fragment_id.get(frag_id)
        if not label:
            label = _frag_label(fragment, frag_id)

        artifact_label = label if labels_by_fragment_id and frag_id in labels_by_fragment_id else "Imported Artifact"
        artifact_id = artifact_id_by_label.get(artifact_label)
        if artifact_id is None:
            artifact_id = _artifact_node_id(artifact_label)
            artifact_id_by_label[artifact_label] = artifact_id
            artifact_node: Dict[str, Any] = {
                "id": artifact_id,
                "label": artifact_label,
                "level": 0,
                "type": "artifact",
                "parent_id": None,
            }
            if filenames_by_label and artifact_label in filenames_by_label:
                artifact_node["file_name"] = filenames_by_label[artifact_label]
            nodes.append(artifact_node)

        nodes.append(
            {
                "id": frag_id,
                "label": label,
                "level": 0,
                "type": ftype,
                "parent_id": artifact_id,
            }
        )
        edges.append(
            {
                "id": f"edge-{uuid4().hex}",
                "source": artifact_id,
                "target": frag_id,
                "label": "artifact-root",
                "kind": "artifact-root",
            }
        )
        content_entries.append(
            {
                "fragmentId": frag_id,
                "parentFragmentId": artifact_id,
                "level": 0,
                "type": ftype,
            }
        )

    manifests: List[Dict[str, Any]] = []
    if content_entries:
        manifests.append(
            {
                "schemaVersion": 1,
                "artifactId": "default",
                "kind": "document",
                "fragments": content_entries,
            }
        )

    return nodes, edges, manifests
