from __future__ import annotations

from dataclasses import dataclass
from types import SimpleNamespace
from typing import Any

from ikam.forja.cas import cas_fragment
from ikam.fragments import RELATION_MIME


@dataclass
class DecompositionDirective:
    source: bytes
    mime_type: str
    artifact_id: str


@dataclass
class DecompositionResult:
    structural: list[Any]
    root_fragments: list[Any]
    canonical: Any | None = None
    source: bytes = b""


class _MapCompatDecomposer:
    def __init__(self, mime_type: str):
        self._mime_type = mime_type

    def decompose(self, directive: DecompositionDirective) -> DecompositionResult:
        resolved_mime = directive.mime_type or self._mime_type
        text_value = None
        try:
            text_value = directive.source.decode("utf-8")
        except UnicodeDecodeError:
            text_value = None

        leaf_payload = {
            "bytes_b64": __import__("base64").b64encode(directive.source).decode("utf-8"),
            "text": text_value or "",
        }
        leaf_frag = cas_fragment(leaf_payload, resolved_mime)
        leaves: list[Any] = [
            SimpleNamespace(
                id=getattr(leaf_frag, "cas_id", None),
                cas_id=getattr(leaf_frag, "cas_id", None),
                mime_type=resolved_mime,
                value=leaf_payload,
                level=1,
                type="text" if text_value is not None else "binary",
            )
        ]

        root_payload = {
            "kind": "contains",
            "artifact_id": directive.artifact_id,
            "radicals": [leaf.cas_id for leaf in leaves if getattr(leaf, "cas_id", None)],
        }
        root = cas_fragment(root_payload, RELATION_MIME)
        root_fragment = SimpleNamespace(
            id=getattr(root, "cas_id", None),
            cas_id=getattr(root, "cas_id", None),
            mime_type=RELATION_MIME,
            value=root_payload,
            level=0,
            type="relation",
        )

        structural = [root_fragment, *leaves]
        root_fragments = list(structural)
        canonical = cas_fragment(
            {"bytes_b64": __import__("base64").b64encode(directive.source).decode("utf-8")},
            resolved_mime,
        )
        if not hasattr(canonical, "value"):
            canonical = SimpleNamespace(
                cas_id=getattr(canonical, "cas_id", None),
                mime_type=resolved_mime,
                value={"bytes_b64": __import__("base64").b64encode(directive.source).decode("utf-8")},
                level=0,
                type="canonical",
            )
        return DecompositionResult(
            source=directive.source,
            structural=structural,
            root_fragments=root_fragments,
            canonical=canonical,
        )


def register_defaults() -> None:
    return None


def get_decomposer(mime_type: str) -> _MapCompatDecomposer:
    return _MapCompatDecomposer(mime_type)
