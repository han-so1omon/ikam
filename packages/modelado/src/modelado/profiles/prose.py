"""Prose Profile — Sequence of Propositions + Lexical Deltas.

This profile specializes StructuredDataIR for representing narrative prose
while maintaining 100% byte-fidelity through phrasing patches.
Satisfies Architecture Decision D18.
"""

from __future__ import annotations

import io
import re
import zipfile
from typing import Any, Dict, List, Literal, Optional, Union
from xml.sax.saxutils import escape

from pydantic import BaseModel, ConfigDict, Field

from ikam.ir import StructuredDataIR
from modelado.profiles import PROSE_BACKBONE_V1, PHRASING_DELTA_V1


class ProseBackbone(BaseModel):
    """
    Represents the ordered logical sequence of a prose unit (e.g., a paragraph).
    """

    model_config = ConfigDict(extra="forbid", populate_by_name=True)

    schema_id: Literal["modelado/prose-backbone@1"] = Field(
        default="modelado/prose-backbone@1",
        alias="schema",
        description="Prose backbone schema id",
    )

    proposition_ids: List[str] = Field(
        ..., description="Ordered list of fragment IDs pointing to MIME_PROPOSITION fragments"
    )

    metadata: Dict[str, Any] = Field(
        default_factory=dict, description="Arbitrary structural context (role, heading_level, etc.)"
    )


class TextPatchOp(BaseModel):
    """A single character-level text patch operation."""

    model_config = ConfigDict(extra="forbid")

    op: Literal["insert", "delete"] = Field(..., description="Operation type")
    at: int = Field(..., description="Character offset for the operation")
    text: Optional[str] = Field(None, description="Text to insert (required for insert)")
    length: Optional[int] = Field(
        None, description="Number of characters to delete (required for delete)"
    )


class PhrasingDelta(BaseModel):
    """
    Companion artifact for ProseBackbone to handle lexical variance for exact byte reproduction.
    """

    model_config = ConfigDict(extra="forbid", populate_by_name=True)

    schema_id: Literal["modelado/phrasing-delta@1"] = Field(
        default="modelado/phrasing-delta@1",
        alias="schema",
        description="Phrasing delta schema id",
    )

    target_id: str = Field(
        ..., description="ID of the ProseBackbone fragment this delta applies to"
    )

    ops: List[TextPatchOp] = Field(..., description="Ordered character-level patch operations")

    metadata: Dict[str, Any] = Field(default_factory=dict)


def create_prose_backbone(
    artifact_id: str,
    proposition_ids: List[str],
    scope_id: Optional[str] = None,
    provenance_id: Optional[str] = None,
) -> StructuredDataIR:
    """Create a StructuredDataIR instance with the ProseBackbone profile."""
    backbone = ProseBackbone(proposition_ids=proposition_ids)
    return StructuredDataIR(
        artifact_id=artifact_id,
        scope_id=scope_id,
        provenance_id=provenance_id,
        profile=PROSE_BACKBONE_V1,
        data=backbone.model_dump(mode="json", by_alias=True),
    )


def create_phrasing_delta(
    artifact_id: str,
    target_id: str,
    ops: List[TextPatchOp],
    scope_id: Optional[str] = None,
    provenance_id: Optional[str] = None,
) -> StructuredDataIR:
    """Create a StructuredDataIR instance with the PhrasingDelta profile."""
    delta = PhrasingDelta(target_id=target_id, ops=ops)
    return StructuredDataIR(
        artifact_id=artifact_id,
        scope_id=scope_id,
        provenance_id=provenance_id,
        profile=PHRASING_DELTA_V1,
        data=delta.model_dump(mode="json", by_alias=True),
    )


def render_docx_from_paragraph_text(template_docx_bytes: bytes, paragraph_text: str) -> bytes:
    """Render DOCX bytes by replacing the paragraph text in word/document.xml.

    This utility is intentionally minimal for Stage 1 fidelity gates.
    It preserves the original ZIP member order and compression settings where possible.
    """
    zin = zipfile.ZipFile(io.BytesIO(template_docx_bytes), mode="r")
    out_buffer = io.BytesIO()
    with zipfile.ZipFile(out_buffer, mode="w") as zout:
        for item in zin.infolist():
            data = zin.read(item.filename)
            if item.filename == "word/document.xml":
                xml_text = data.decode("utf-8")
                replaced, count = re.subn(
                    r"(<w:t[^>]*>)(.*?)(</w:t>)",
                    lambda m: f"{m.group(1)}{escape(paragraph_text)}{m.group(3)}",
                    xml_text,
                    count=1,
                    flags=re.DOTALL,
                )
                if count == 0:
                    raise ValueError("DOCX render failed: no <w:t> text node found")
                data = replaced.encode("utf-8")
            zout.writestr(item, data)
    zin.close()
    return out_buffer.getvalue()
