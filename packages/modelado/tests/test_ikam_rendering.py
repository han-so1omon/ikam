"""Tests for IKAM artifact rendering adapter.

Validates:
1. Lossless reconstruction from fragments
2. Provenance event emission with required metadata
3. Renderer invocation (mock/stub)
4. Variation ID generation and determinism
5. Error handling for invalid inputs
"""

from __future__ import annotations

import base64
import json
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Optional
from unittest.mock import Mock, patch

import pytest

from modelado.ikam_rendering import (
    ExternalRenderer,
    RenderError,
    RenderRequest,
    RenderResult,
    RendererType,
    reconstruct_artifact_bytes,
    render_artifact,
)
from ikam.fragments import BindingGroup, Fragment, RELATION_MIME, Relation, SlotBinding
from ikam.graph import Artifact


# Mock renderer for testing

class MockRenderer:
    """Mock renderer for testing."""
    
    def __init__(self, output: bytes = b"rendered", version: str = "mock-1.0", mime: str = "application/mock"):
        self._output = output
        self._version = version
        self._mime = mime
    
    def render(self, input_bytes: bytes, seed: Optional[int] = None) -> bytes:
        # Simple deterministic transform: prepend seed if provided
        if seed is not None:
            return f"seed={seed}|".encode("utf-8") + input_bytes
        return self._output
    
    def get_version(self) -> str:
        return self._version
    
    def get_output_mime_type(self) -> str:
        return self._mime


# Fixtures

@pytest.fixture
def sample_artifact() -> Artifact:
    """Sample artifact for testing."""
    return Artifact(
        id="art-123",
        kind="document",
        title="Sample Document",
        root_fragment_id="frag-1",
    )


@pytest.fixture
def sample_fragments(sample_artifact: Artifact) -> list[Any]:
    """Sample fragments for reconstruction."""
    return [
        SimpleNamespace(id="frag-1", artifact_id=sample_artifact.id, level=0, content="First paragraph."),
        SimpleNamespace(id="frag-2", artifact_id=sample_artifact.id, level=0, content="Second paragraph."),
    ]


@pytest.fixture
def sample_v3_document_fragments() -> list[Fragment]:
    """V3 fragments containing a root relation and canonical bytes payload."""
    canonical_cas_id = "cas-doc-1"
    relation = Relation(
        predicate="reconstructs",
        binding_groups=[
            BindingGroup(
                invocation_id="inv-1",
                slots=[SlotBinding(slot="canonical", fragment_id=canonical_cas_id)],
            )
        ],
    )

    return [
        Fragment(
            fragment_id="frag-root",
            mime_type=RELATION_MIME,
            value=relation.model_dump(),
        ),
        Fragment(
            cas_id=canonical_cas_id,
            mime_type="application/octet-stream",
            value={"bytes_b64": base64.b64encode(b"Hello V3 rendering.").decode("ascii")},
        ),
    ]


@pytest.fixture
def sample_v3_file_fragments() -> list[Fragment]:
    """V3 fragments for a binary/file artifact with canonical bytes payload."""
    canonical_cas_id = "cas-file-1"
    relation = Relation(
        predicate="reconstructs",
        binding_groups=[
            BindingGroup(
                invocation_id="inv-file-1",
                slots=[SlotBinding(slot="canonical", fragment_id=canonical_cas_id)],
            )
        ],
    )

    return [
        Fragment(
            fragment_id="frag-file-root",
            mime_type=RELATION_MIME,
            value=relation.model_dump(),
        ),
        Fragment(
            cas_id=canonical_cas_id,
            mime_type="application/octet-stream",
            value={"bytes_b64": base64.b64encode(b"\x00IKAM\xff").decode("ascii")},
        ),
    ]


# Tests

@pytest.mark.skip(reason="Legacy document reconstruction path is retired")
def test_reconstruct_artifact_bytes_document(sample_artifact: Artifact, sample_fragments: list[Any]):
    """Test lossless reconstruction for document artifacts."""
    reconstructed = reconstruct_artifact_bytes(sample_artifact, sample_fragments)
    
    assert isinstance(reconstructed, bytes)
    decoded = reconstructed.decode("utf-8")
    assert "First paragraph." in decoded
    assert "Second paragraph." in decoded


def test_reconstruct_artifact_bytes_empty():
    """Test reconstruction with no fragments returns empty bytes."""
    artifact = Artifact(id="art-empty", kind="document", title="Empty")
    reconstructed = reconstruct_artifact_bytes(artifact, [])
    assert reconstructed == b""


def test_reconstruct_artifact_bytes_document_accepts_v3_fragments(
    sample_artifact: Artifact,
    sample_v3_document_fragments: list[Fragment],
):
    """Document reconstruction uses the actual V3 fragment shape."""
    reconstructed = reconstruct_artifact_bytes(sample_artifact, sample_v3_document_fragments)

    assert reconstructed == b"Hello V3 rendering."


def test_reconstruct_artifact_bytes_file_accepts_v3_fragments(
    sample_v3_file_fragments: list[Fragment],
):
    """File reconstruction should use the V3 binary path, not generic payload joining."""
    artifact = Artifact(id="art-file-123", kind="file", title="Binary File", root_fragment_id="frag-file-root")

    reconstructed = reconstruct_artifact_bytes(artifact, sample_v3_file_fragments)

    assert reconstructed == b"\x00IKAM\xff"


def test_reconstruct_artifact_bytes_mismatched_artifact(sample_artifact: Artifact):
    """Test reconstruction fails for fragments from different artifact."""
    other_artifact = Artifact(id="art-other", kind="document", title="Other", root_fragment_id="frag-wrong")
    fragments = [
        SimpleNamespace(id="frag-wrong", artifact_id="art-wrong", level=0, content="Wrong artifact."),
    ]
    
    with pytest.raises(RenderError, match="Fragments belong to different artifacts"):
        reconstruct_artifact_bytes(other_artifact, fragments)


@pytest.mark.skip(reason="Legacy document reconstruction path is retired")
def test_render_artifact_passthrough(sample_artifact: Artifact, sample_fragments: list[Any]):
    """Test passthrough rendering (no external renderer)."""
    request = RenderRequest(
        artifact=sample_artifact,
        fragments=sample_fragments,
        renderer_type=RendererType.PASSTHROUGH,
        seed=42,
        variation_id="var-test",
    )
    
    result = render_artifact(request)
    
    assert result.output_bytes == reconstruct_artifact_bytes(sample_artifact, sample_fragments)
    assert result.mime_type == "application/octet-stream"
    assert result.renderer_version == "passthrough-1.0"
    assert result.seed == 42
    assert result.variation_id == "var-test"
    assert result.provenance["artifact_id"] == sample_artifact.id
    assert result.provenance["fragment_count"] == 2


@pytest.mark.skip(reason="Legacy document reconstruction path is retired")
def test_render_artifact_with_mock_renderer(sample_artifact: Artifact, sample_fragments: list[Any]):
    """Test rendering with external mock renderer."""
    mock_renderer = MockRenderer(output=b"mocked output", version="mock-2.0", mime="text/mock")
    
    request = RenderRequest(
        artifact=sample_artifact,
        fragments=sample_fragments,
        renderer_type=RendererType.PASSTHROUGH,  # Ignored when renderer provided
        seed=123,
    )
    
    result = render_artifact(request, renderer=mock_renderer)
    
    # MockRenderer prepends seed when provided, then uses reconstructed input
    assert result.output_bytes.startswith(b"seed=123|")
    assert result.mime_type == "text/mock"
    assert result.renderer_version == "mock-2.0"
    assert result.seed == 123
    assert result.provenance["renderer_version"] == "mock-2.0"


@pytest.mark.skip(reason="Legacy document reconstruction path is retired")
def test_render_artifact_deterministic_seed(sample_artifact: Artifact, sample_fragments: list[Any]):
    """Test deterministic rendering with seed."""
    mock_renderer = MockRenderer()
    
    request1 = RenderRequest(
        artifact=sample_artifact,
        fragments=sample_fragments,
        renderer_type=RendererType.PASSTHROUGH,
        seed=999,
    )
    
    request2 = RenderRequest(
        artifact=sample_artifact,
        fragments=sample_fragments,
        renderer_type=RendererType.PASSTHROUGH,
        seed=999,
    )
    
    result1 = render_artifact(request1, renderer=mock_renderer)
    result2 = render_artifact(request2, renderer=mock_renderer)
    
    # Same seed → same output (deterministic)
    assert result1.output_bytes == result2.output_bytes
    assert result1.variation_id == result2.variation_id


@pytest.mark.skip(reason="Legacy document reconstruction path is retired")
def test_render_artifact_variation_id_generated(sample_artifact: Artifact, sample_fragments: list[Any]):
    """Test variation ID auto-generation when not provided."""
    request = RenderRequest(
        artifact=sample_artifact,
        fragments=sample_fragments,
        renderer_type=RendererType.PASSTHROUGH,
        seed=42,
        renderer_version="test-1.0",
    )
    
    result = render_artifact(request)
    
    assert result.variation_id is not None
    assert result.variation_id.startswith("var-")
    assert len(result.variation_id) == 20  # "var-" + 16 hex chars


@pytest.mark.skip(reason="Legacy document reconstruction path is retired")
def test_render_artifact_provenance_metadata(sample_artifact: Artifact, sample_fragments: list[Any]):
    """Test provenance metadata completeness."""
    request = RenderRequest(
        artifact=sample_artifact,
        fragments=sample_fragments,
        renderer_type=RendererType.PASSTHROUGH,
        seed=111,
        variation_id="var-custom",
    )
    
    result = render_artifact(request)
    
    prov = result.provenance
    assert prov["artifact_id"] == sample_artifact.id
    assert prov["renderer_type"] == RendererType.PASSTHROUGH.value
    assert prov["renderer_version"] == "passthrough-1.0"
    assert prov["seed"] == 111
    assert prov["variation_id"] == "var-custom"
    assert prov["fragment_count"] == 2
    assert prov["output_size"] == len(result.output_bytes)


def test_render_artifact_reconstruction_error(sample_artifact: Artifact):
    """Test error handling when reconstruction fails."""
    bad_fragment = SimpleNamespace(id="frag-bad", artifact_id="art-wrong", level=0, content="Bad")
    
    request = RenderRequest(
        artifact=sample_artifact,
        fragments=[bad_fragment],
        renderer_type=RendererType.PASSTHROUGH,
    )
    
    with pytest.raises(RenderError, match="Reconstruction failed"):
        render_artifact(request)


def test_render_artifact_passthrough_accepts_v3_fragments(
    sample_artifact: Artifact,
    sample_v3_document_fragments: list[Fragment],
):
    """Passthrough rendering does not require legacy fragment fields."""
    request = RenderRequest(
        artifact=sample_artifact,
        fragments=sample_v3_document_fragments,
        renderer_type=RendererType.PASSTHROUGH,
        variation_id="var-v3",
    )

    result = render_artifact(request)

    assert result.output_bytes == b"Hello V3 rendering."
    assert result.mime_type == "application/octet-stream"
    assert result.variation_id == "var-v3"
    assert result.provenance["fragment_count"] == 2


@pytest.mark.skip(reason="Legacy document reconstruction path is retired")
def test_render_artifact_renderer_error(sample_artifact: Artifact, sample_fragments: list[Any]):
    """Test error handling when external renderer fails."""
    failing_renderer = Mock(spec=ExternalRenderer)
    failing_renderer.render.side_effect = Exception("Renderer crashed")
    
    request = RenderRequest(
        artifact=sample_artifact,
        fragments=sample_fragments,
        renderer_type=RendererType.MARKDOWN_HTML,
    )
    
    with pytest.raises(RenderError, match="External rendering failed"):
        render_artifact(request, renderer=failing_renderer)


def test_markdown_html_renderer():
    """Test built-in Markdown→HTML renderer (integration test)."""
    pytest.importorskip("markdown", reason="markdown library required for this test")
    
    from modelado.ikam_rendering import MarkdownHtmlRenderer
    
    renderer = MarkdownHtmlRenderer()
    md_input = b"# Heading\n\nParagraph with **bold**."
    html_output = renderer.render(md_input)
    
    assert b"<h1>Heading</h1>" in html_output
    assert b"<strong>bold</strong>" in html_output
    assert renderer.get_output_mime_type() == "text/html"
    assert "markdown" in renderer.get_version()


@pytest.mark.skip(reason="Requires pdflatex binary; run manually in environments with LaTeX installed")
def test_latex_pdf_renderer():
    """Test built-in LaTeX→PDF renderer (manual integration test)."""
    from modelado.ikam_rendering import LatexPdfRenderer
    
    renderer = LatexPdfRenderer()
    latex_input = rb"""
\documentclass{article}
\begin{document}
Hello, LaTeX!
\end{document}
"""
    pdf_output = renderer.render(latex_input)
    
    # Basic PDF validation: check for PDF magic bytes
    assert pdf_output.startswith(b"%PDF")
    assert renderer.get_output_mime_type() == "application/pdf"
    assert "pdflatex" in renderer.get_version().lower()
