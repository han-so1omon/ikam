"""Tests for deterministic Excel rendering.

Verifies non-deterministic mode includes time variance and deterministic mode produces stable bytes.
"""
from __future__ import annotations

import os
import time
from hashlib import blake2b

from ikam.renderers.excel import render_excel


def _hash(data: bytes) -> str:
    return blake2b(data, digest_size=16).hexdigest()


def test_excel_nondeterministic_differs(monkeypatch):
    """Without deterministic flag, two renders should differ if timestamp changes."""
    # Ensure flags disabled
    monkeypatch.delenv("IKAM_DETERMINISTIC_RENDER", raising=False)
    monkeypatch.delenv("IKAM_FROZEN_TIMESTAMP", raising=False)
    monkeypatch.delenv("IKAM_STABLE_IDS", raising=False)

    model = {
        "sheets": [
            {"name": "Financials", "cells": {"A1": 1.123456789, "B2": "Revenue"}},
        ]
    }

    b1 = render_excel(model)
    b2 = render_excel(model)

    h1 = _hash(b1)
    h2 = _hash(b2)
    assert h1 != h2, f"Expected differing hashes without deterministic mode (h1={h1}, h2={h2})"


def test_excel_deterministic_stable(monkeypatch):
    """With deterministic flags, repeated renders produce identical bytes."""
    monkeypatch.setenv("IKAM_DETERMINISTIC_RENDER", "true")
    monkeypatch.setenv("IKAM_FROZEN_TIMESTAMP", "2025-11-30T00:00:00Z")
    monkeypatch.setenv("IKAM_STABLE_IDS", "true")
    monkeypatch.setenv("IKAM_FLOAT_PRECISION", "6")

    model = {
        "sheets": [
            {"name": "Financials", "cells": {"A1": 1.123456789, "B2": "Revenue"}},
            {"name": "Operations", "cells": {"A1": 42.99999999}},
        ]
    }

    b1 = render_excel(model)
    b2 = render_excel(model)

    h1 = _hash(b1)
    h2 = _hash(b2)
    assert h1 == h2, f"Deterministic mode produced differing hashes (h1={h1}, h2={h2})"

    # Validate rounding applied (precision 6) by regenerating and inspecting one cell indirectly via hash change when precision changes
    monkeypatch.setenv("IKAM_FLOAT_PRECISION", "4")
    b3 = render_excel(model)
    h3 = _hash(b3)
    assert h3 != h1, "Changing float precision should alter the output hash in deterministic mode"


def test_excel_deterministic_sheet_order(monkeypatch):
    """Stable IDs enforce deterministic sheet ordering by name."""
    monkeypatch.setenv("IKAM_DETERMINISTIC_RENDER", "true")
    monkeypatch.setenv("IKAM_STABLE_IDS", "true")
    monkeypatch.setenv("IKAM_FROZEN_TIMESTAMP", "2025-11-30T00:00:00Z")

    model_a = {
        "sheets": [
            {"name": "Zeta", "cells": {}},
            {"name": "Alpha", "cells": {}},
        ]
    }
    model_b = {
        "sheets": [
            {"name": "Alpha", "cells": {}},
            {"name": "Zeta", "cells": {}},
        ]
    }

    h_a = _hash(render_excel(model_a))
    h_b = _hash(render_excel(model_b))
    assert h_a == h_b, "Stable IDs mode should ignore input ordering and sort by name deterministically"


def test_excel_invalid_timestamp_graceful(monkeypatch):
    """Invalid frozen timestamp should not crash and should fallback."""
    monkeypatch.setenv("IKAM_DETERMINISTIC_RENDER", "true")
    monkeypatch.setenv("IKAM_FROZEN_TIMESTAMP", "not-a-timestamp")

    model = {"sheets": [{"name": "Main", "cells": {"A1": 1}}]}
    data = render_excel(model)
    assert len(data) > 100, "Renderer should still produce output when timestamp invalid"
