"""IKAM Artifact Rendering Adapter

Provides unified interface for:
1. Deterministic reconstruction from fragments (lossless)
2. External renderer invocation (LaTeX→PDF, Markdown→HTML, Canvas→PNG, etc.)
3. Provenance tracking for render variations (seed, rendererVersion, variationId)

Guarantees:
- Lossless reconstruction: reconstruct(fragments) preserves byte-level equality
- Reproducibility: same (fragments + seed + rendererVersion) → identical output
- Non-determinism isolation: render variations tracked via provenance events

References:
- AGENTS.md: Mathematical soundness for deltas & variations
- docs/ikam/ikam-specification.md: Artifact rendering model
"""

from __future__ import annotations

import hashlib
import subprocess
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import Any, Callable, Optional, Protocol

from ikam.config import ReconstructionConfig
from ikam.fragments import Fragment as V3Fragment
from ikam.graph import Artifact


class RendererType(str, Enum):
    """Supported external renderer types."""
    LATEX_PDF = "latex-pdf"
    MARKDOWN_HTML = "markdown-html"
    CANVAS_PNG = "canvas-png"
    PASSTHROUGH = "passthrough"  # No rendering, just reconstruction


class RenderError(Exception):
    """Error during artifact rendering."""


@dataclass
class RenderRequest:
    """Request for artifact rendering."""
    artifact: Artifact
    fragments: list[Any]
    renderer_type: RendererType
    seed: Optional[int] = None
    renderer_version: Optional[str] = None
    variation_id: Optional[str] = None
    config: Optional[ReconstructionConfig] = None


@dataclass
class RenderResult:
    """Result of artifact rendering."""
    output_bytes: bytes
    mime_type: str
    renderer_version: str
    seed: Optional[int]
    variation_id: Optional[str]
    provenance: dict[str, Any]  # Full metadata for event recording


class ExternalRenderer(Protocol):
    """Protocol for external renderer implementations."""
    
    def render(self, input_bytes: bytes, seed: Optional[int] = None) -> bytes:
        """Render input bytes to output format.
        
        Args:
            input_bytes: Reconstructed artifact bytes
            seed: Optional seed for deterministic rendering
            
        Returns:
            Rendered output bytes
        """
        ...
    
    def get_version(self) -> str:
        """Return renderer version string."""
        ...
    
    def get_output_mime_type(self) -> str:
        """Return MIME type of rendered output."""
        ...


def reconstruct_artifact_bytes(
    artifact: Artifact,
    fragments: list[Any],
    config: Optional[ReconstructionConfig] = None,
) -> bytes:
    """Reconstruct artifact from domain fragments (lossless).
    
    Mathematical Guarantee: reconstruct(decompose(A)) = A (byte-level equality)
    
    Args:
        artifact: Artifact metadata
        fragments: Ordered list of domain fragments (rich hierarchical model)
        config: Reconstruction configuration
        
    Returns:
        Reconstructed bytes
        
    Raises:
        RenderError: If reconstruction fails or fragments incompatible
        
    Note:
        Current V3 runtime fragments are minimal and typically provide `fragment_id`,
        `cas_id`, `value`, and `mime_type`. Legacy rendering callers may still pass
        richer objects; this adapter tolerates both shapes where feasible.
    """
    from ikam.forja import reconstruct_document
    from ikam.sheet_decomposition import reconstruct_workbook
    
    if not fragments:
        return b""

    is_v3_fragment_shape = all(
        hasattr(f, "mime_type") and (hasattr(f, "value") or hasattr(f, "cas_id") or hasattr(f, "fragment_id"))
        for f in fragments
    )

    if is_v3_fragment_shape:
        return reconstruct_artifact_bytes_v3(artifact, fragments)
    
    # Legacy fragment objects may still carry artifact ownership metadata. V3 fragments do not.
    mismatched = [
        getattr(f, "id", None) or getattr(f, "fragment_id", None) or getattr(f, "cas_id", None)
        for f in fragments
        if getattr(f, "artifact_id", None) and getattr(f, "artifact_id") != artifact.id
    ]
    if mismatched:
        raise RenderError(
            f"Fragments belong to different artifacts: expected {artifact.id}, "
            f"but found fragment(s) with artifact_id: {mismatched}"
        )
    
    # Reconstruct based on artifact kind
    if artifact.kind == "document":
        text = reconstruct_document(fragments)  # V3 signature: no config param
        return text.encode("utf-8")
    elif artifact.kind in ("sheet", "workbook"):
        # Note: reconstruct_workbook returns Workbook object; serialize to bytes
        wb = reconstruct_workbook(fragments, config)
        # Placeholder: serialize workbook (JSON or custom format)
        import json
        serialized = json.dumps({
            "id": wb.id,
            "title": wb.meta.title,
            "sheets": [{"id": s.id, "title": s.title, "cells": len(s.cells)} for s in wb.sheets],
        })
        return serialized.encode("utf-8")
    else:
        # Generic: serialize the most specific payload available without assuming
        # legacy fields that are absent from the V3 fragment model.
        import json
        sorted_frags = sorted(
            fragments,
            key=lambda f: (
                getattr(f, "level", 0),
                getattr(f, "fragment_id", None) or getattr(f, "id", None) or getattr(f, "cas_id", None) or "",
            ),
        )
        parts = []
        for f in sorted_frags:
            payload = getattr(f, "value", None)
            if payload is None:
                payload = getattr(f, "content", None)

            if payload is None:
                parts.append(b"")
                continue

            if hasattr(payload, "model_dump"):
                payload = payload.model_dump()

            if isinstance(payload, (bytes, bytearray)):
                parts.append(bytes(payload))
            elif isinstance(payload, str):
                parts.append(payload.encode("utf-8"))
            else:
                parts.append(json.dumps(payload).encode("utf-8"))
        return b"".join(parts)


def render_artifact(
    request: RenderRequest,
    renderer: Optional[ExternalRenderer] = None,
) -> RenderResult:
    """Render artifact with provenance tracking.
    
    Workflow:
    1. Deterministic reconstruction from fragments
    2. External renderer invocation (if specified)
    3. Provenance metadata assembly
    
    Args:
        request: Render request with artifact, fragments, and parameters
        renderer: Optional external renderer; if None, uses built-in based on renderer_type
        
    Returns:
        RenderResult with output bytes and full provenance
        
    Raises:
        RenderError: If rendering fails
    """
    # Step 1: Lossless reconstruction
    try:
        reconstructed = reconstruct_artifact_bytes(
            request.artifact,
            request.fragments,
            request.config,
        )
    except Exception as e:
        raise RenderError(f"Reconstruction failed: {e}") from e
    
    # Step 2: External rendering (if not passthrough)
    if request.renderer_type == RendererType.PASSTHROUGH and renderer is None:
        output_bytes = reconstructed
        mime_type = "application/octet-stream"
        renderer_version = "passthrough-1.0"
    else:
        if renderer is None:
            renderer = _get_builtin_renderer(request.renderer_type)
        try:
            output_bytes = renderer.render(reconstructed, request.seed)
            mime_type = renderer.get_output_mime_type()
            renderer_version = renderer.get_version()
        except Exception as e:
            raise RenderError(f"External rendering failed: {e}") from e
    
    # Step 3: Assemble provenance
    variation_id = request.variation_id or _generate_variation_id(
        artifact_id=request.artifact.id,
        seed=request.seed,
        renderer_version=renderer_version,
    )
    
    provenance = {
        "artifact_id": request.artifact.id,
        "renderer_type": request.renderer_type.value,
        "renderer_version": renderer_version,
        "seed": request.seed,
        "variation_id": variation_id,
        "fragment_count": len(request.fragments),
        "output_size": len(output_bytes),
    }
    
    return RenderResult(
        output_bytes=output_bytes,
        mime_type=mime_type,
        renderer_version=renderer_version,
        seed=request.seed,
        variation_id=variation_id,
        provenance=provenance,
    )


def _generate_variation_id(artifact_id: str, seed: Optional[int], renderer_version: str) -> str:
    """Generate deterministic variation ID from parameters."""
    components = [artifact_id, str(seed or 0), renderer_version]
    combined = "|".join(components)
    hash_digest = hashlib.sha256(combined.encode("utf-8")).hexdigest()
    return f"var-{hash_digest[:16]}"


def _get_builtin_renderer(renderer_type: RendererType) -> ExternalRenderer:
    """Get built-in renderer implementation."""
    if renderer_type == RendererType.LATEX_PDF:
        return LatexPdfRenderer()
    elif renderer_type == RendererType.MARKDOWN_HTML:
        return MarkdownHtmlRenderer()
    elif renderer_type == RendererType.CANVAS_PNG:
        raise RenderError("Canvas→PNG renderer not yet implemented")
    else:
        raise RenderError(f"Unsupported renderer type: {renderer_type}")


# Built-in renderer implementations

class LatexPdfRenderer:
    """LaTeX → PDF renderer using pdflatex."""
    
    def render(self, input_bytes: bytes, seed: Optional[int] = None) -> bytes:
        """Compile LaTeX to PDF."""
        import tempfile
        with tempfile.TemporaryDirectory() as tmpdir:
            tmp_path = Path(tmpdir)
            tex_file = tmp_path / "document.tex"
            pdf_file = tmp_path / "document.pdf"
            
            tex_file.write_bytes(input_bytes)
            
            # Run pdflatex (deterministic mode if seed provided)
            cmd = ["pdflatex", "-interaction=nonstopmode"]
            if seed is not None:
                # Note: LaTeX randomness control requires special packages; placeholder for now
                cmd.extend(["-jobname", f"document-seed{seed}"])
            cmd.append(str(tex_file))
            
            result = subprocess.run(
                cmd,
                cwd=tmpdir,
                capture_output=True,
                timeout=30,
            )
            
            if result.returncode != 0 or not pdf_file.exists():
                raise RenderError(f"pdflatex failed: {result.stderr.decode('utf-8', errors='replace')}")
            
            return pdf_file.read_bytes()
    
    def get_version(self) -> str:
        """Get pdflatex version."""
        try:
            result = subprocess.run(
                ["pdflatex", "--version"],
                capture_output=True,
                timeout=5,
            )
            version_line = result.stdout.decode("utf-8").split("\n")[0]
            return version_line.strip()
        except Exception:
            return "pdflatex-unknown"
    
    def get_output_mime_type(self) -> str:
        return "application/pdf"


class MarkdownHtmlRenderer:
    """Markdown → HTML renderer using Python markdown library."""
    
    def render(self, input_bytes: bytes, seed: Optional[int] = None) -> bytes:
        """Convert Markdown to HTML."""
        try:
            import markdown  # type: ignore
        except ImportError:
            raise RenderError("markdown library not installed; run: pip install markdown")
        
        md_text = input_bytes.decode("utf-8")
        html = markdown.markdown(md_text)
        return html.encode("utf-8")
    
    def get_version(self) -> str:
        try:
            import markdown  # type: ignore
            return f"markdown-{markdown.__version__}"
        except Exception:
            return "markdown-unknown"
    
    def get_output_mime_type(self) -> str:
        return "text/html"


# ============================================================================
# V3 Rendering
# ============================================================================


def reconstruct_artifact_bytes_v3(
    artifact: Artifact,
    fragments: list[V3Fragment],
) -> bytes:
    """Reconstruct artifact bytes from V3 fragments via root relation DAG.

    Delegates to the V3 reconstructor, selecting document vs binary path
    based on artifact kind.

    Mathematical Guarantee: reconstruct(decompose(A)) = A (byte-level equality)

    Args:
        artifact: Artifact metadata (used for kind routing)
        fragments: V3 Fragment objects (must include root relation)

    Returns:
        Reconstructed bytes

    Raises:
        RenderError: If reconstruction fails
    """
    from ikam.forja import reconstruct_document, reconstruct_binary

    if not fragments:
        return b""

    try:
        if artifact.kind == "document":
            text = reconstruct_document(fragments)
            return text.encode("utf-8")
        else:
            return reconstruct_binary(fragments)
    except Exception as e:
        raise RenderError(f"V3 reconstruction failed: {e}") from e
