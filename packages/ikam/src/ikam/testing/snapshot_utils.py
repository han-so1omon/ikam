"""Snapshot testing utilities for deterministic export validation.

These helpers live in the `ikam` package so tests can import them without
needing to add the tests directory to `PYTHONPATH`.
"""

from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Literal


ExportFormat = Literal["json", "xlsx", "pptx"]


def compute_hash(data: bytes) -> str:
    return hashlib.blake2b(data).hexdigest()


def _project_root() -> Path:
    # This file lives at: <root>/packages/ikam/src/ikam/testing/snapshot_utils.py
    return Path(__file__).resolve().parents[5]


def load_baseline(format: ExportFormat, baseline_name: str = "baseline") -> bytes:
    baseline_path = _project_root() / "tests" / "fixtures" / "exports" / f"{baseline_name}.{format}"

    if not baseline_path.exists():
        raise FileNotFoundError(
            f"Baseline not found: {baseline_path}\n"
            "Run baseline generation script to create golden files."
        )

    return baseline_path.read_bytes()


def assert_matches_baseline(
    rendered: bytes,
    format: ExportFormat,
    baseline_name: str = "baseline",
) -> None:
    baseline = load_baseline(format, baseline_name)

    rendered_hash = compute_hash(rendered)
    baseline_hash = compute_hash(baseline)

    assert rendered_hash == baseline_hash, (
        f"Snapshot mismatch for {format} (baseline: {baseline_name})\n"
        f"Expected hash: {baseline_hash}\n"
        f"Actual hash:   {rendered_hash}\n"
        f"Rendered size: {len(rendered)} bytes\n"
        f"Baseline size: {len(baseline)} bytes\n\n"
        "This indicates non-deterministic rendering or a change in output format.\n"
        "If the change is intentional, regenerate baselines with the generation script."
    )


def save_snapshot(
    rendered: bytes,
    format: ExportFormat,
    snapshot_name: str,
) -> Path:
    snapshot_dir = _project_root() / "tests" / "fixtures" / "exports" / "snapshots"
    snapshot_dir.mkdir(parents=True, exist_ok=True)

    snapshot_path = snapshot_dir / f"{snapshot_name}.{format}"
    snapshot_path.write_bytes(rendered)

    return snapshot_path
