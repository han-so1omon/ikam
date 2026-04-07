"""Tests for deterministic PowerPoint rendering.

Verifies non-deterministic mode includes variance and deterministic mode produces stable bytes.
"""
from __future__ import annotations

import os
from hashlib import blake2b

from ikam.renderers.slides import render_pptx


def _hash(data: bytes) -> str:
    return blake2b(data, digest_size=16).hexdigest()


def test_pptx_nondeterministic_differs(monkeypatch):
    """Without deterministic flag, two renders should differ (UUID injection)."""
    monkeypatch.delenv("IKAM_DETERMINISTIC_RENDER", raising=False)
    monkeypatch.delenv("IKAM_FROZEN_TIMESTAMP", raising=False)
    monkeypatch.delenv("IKAM_STABLE_IDS", raising=False)

    model = {
        "slides": [
            {"title": "Overview", "content": "Key points here"},
        ]
    }

    b1 = render_pptx(model)
    b2 = render_pptx(model)

    h1 = _hash(b1)
    h2 = _hash(b2)
    assert h1 != h2, f"Expected differing hashes without deterministic mode (h1={h1}, h2={h2})"


def test_pptx_deterministic_stable(monkeypatch):
    """With deterministic flags, repeated renders produce identical bytes."""
    monkeypatch.setenv("IKAM_DETERMINISTIC_RENDER", "true")
    monkeypatch.setenv("IKAM_FROZEN_TIMESTAMP", "2025-11-30T00:00:00Z")
    monkeypatch.setenv("IKAM_STABLE_IDS", "true")

    model = {
        "slides": [
            {"title": "Introduction", "content": "Welcome"},
            {"title": "Conclusion", "content": "Thank you"},
        ]
    }

    b1 = render_pptx(model)
    b2 = render_pptx(model)

    h1 = _hash(b1)
    h2 = _hash(b2)
    assert h1 == h2, f"Deterministic mode produced differing hashes (h1={h1}, h2={h2})"


def test_pptx_deterministic_slide_order(monkeypatch):
    """Stable IDs enforce deterministic slide ordering by title."""
    monkeypatch.setenv("IKAM_DETERMINISTIC_RENDER", "true")
    monkeypatch.setenv("IKAM_STABLE_IDS", "true")
    monkeypatch.setenv("IKAM_FROZEN_TIMESTAMP", "2025-11-30T00:00:00Z")

    model_a = {
        "slides": [
            {"title": "Zeta", "content": "Last"},
            {"title": "Alpha", "content": "First"},
        ]
    }
    model_b = {
        "slides": [
            {"title": "Alpha", "content": "First"},
            {"title": "Zeta", "content": "Last"},
        ]
    }

    h_a = _hash(render_pptx(model_a))
    h_b = _hash(render_pptx(model_b))
    assert h_a == h_b, "Stable IDs mode should ignore input ordering and sort by title deterministically"


def test_pptx_with_speaker_notes(monkeypatch):
    """Verify speaker notes are included and stable in deterministic mode."""
    monkeypatch.setenv("IKAM_DETERMINISTIC_RENDER", "true")
    monkeypatch.setenv("IKAM_FROZEN_TIMESTAMP", "2025-11-30T00:00:00Z")

    model = {
        "slides": [
            {"title": "Slide 1", "content": "Content", "notes": "Remember to emphasize this point"},
        ]
    }

    b1 = render_pptx(model)
    b2 = render_pptx(model)

    assert _hash(b1) == _hash(b2), "Speaker notes should be stable in deterministic mode"


def test_pptx_empty_slides(monkeypatch):
    """Verify empty presentation renders without errors."""
    monkeypatch.setenv("IKAM_DETERMINISTIC_RENDER", "true")

    model = {"slides": []}
    data = render_pptx(model)
    assert len(data) > 100, "Empty presentation should still produce valid PPTX bytes"


def test_pptx_invalid_timestamp_graceful(monkeypatch):
    """Invalid frozen timestamp should not crash."""
    monkeypatch.setenv("IKAM_DETERMINISTIC_RENDER", "true")
    monkeypatch.setenv("IKAM_FROZEN_TIMESTAMP", "not-a-timestamp")

    model = {"slides": [{"title": "Test", "content": "Content"}]}
    data = render_pptx(model)
    assert len(data) > 100, "Renderer should still produce output when timestamp invalid"
