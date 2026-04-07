"""
Snapshot testing utilities for deterministic export validation.

Provides helpers to compare rendered artifacts against golden baselines.
"""

import hashlib
from pathlib import Path
from typing import Literal


ExportFormat = Literal["json", "xlsx", "pptx"]


def compute_hash(data: bytes) -> str:
    """
    Compute blake2b hash of bytes.
    
    Args:
        data: Bytes to hash
    
    Returns:
        Hexadecimal hash string
    """
    return hashlib.blake2b(data).hexdigest()


def load_baseline(format: ExportFormat, baseline_name: str = "baseline") -> bytes:
    """
    Load golden baseline file from tests/fixtures/exports/.
    
    Args:
        format: Export format (json, xlsx, pptx)
        baseline_name: Baseline file name without extension
    
    Returns:
        Baseline file bytes
    
    Raises:
        FileNotFoundError: If baseline doesn't exist
    """
    # Navigate from packages/ikam/tests/ to project root
    project_root = Path(__file__).parent.parent.parent.parent
    baseline_path = project_root / "tests" / "fixtures" / "exports" / f"{baseline_name}.{format}"
    
    if not baseline_path.exists():
        raise FileNotFoundError(
            f"Baseline not found: {baseline_path}\n"
            f"Run baseline generation script to create golden files."
        )
    
    return baseline_path.read_bytes()


def assert_matches_baseline(
    rendered: bytes,
    format: ExportFormat,
    baseline_name: str = "baseline",
) -> None:
    """
    Assert that rendered output matches golden baseline.
    
    Args:
        rendered: Rendered artifact bytes
        format: Export format (json, xlsx, pptx)
        baseline_name: Baseline file name without extension
    
    Raises:
        AssertionError: If hashes don't match
        FileNotFoundError: If baseline doesn't exist
    """
    baseline = load_baseline(format, baseline_name)
    
    rendered_hash = compute_hash(rendered)
    baseline_hash = compute_hash(baseline)
    
    assert rendered_hash == baseline_hash, (
        f"Snapshot mismatch for {format} (baseline: {baseline_name})\n"
        f"Expected hash: {baseline_hash}\n"
        f"Actual hash:   {rendered_hash}\n"
        f"Rendered size: {len(rendered)} bytes\n"
        f"Baseline size: {len(baseline)} bytes\n"
        f"\n"
        f"This indicates non-deterministic rendering or a change in output format.\n"
        f"If the change is intentional, regenerate baselines with the generation script."
    )


def save_snapshot(
    rendered: bytes,
    format: ExportFormat,
    snapshot_name: str,
) -> Path:
    """
    Save rendered output as a new snapshot for debugging.
    
    Args:
        rendered: Rendered artifact bytes
        format: Export format (json, xlsx, pptx)
        snapshot_name: Snapshot file name without extension
    
    Returns:
        Path to saved snapshot
    """
    # Navigate from packages/ikam/tests/ to project root
    project_root = Path(__file__).parent.parent.parent.parent
    snapshot_dir = project_root / "tests" / "fixtures" / "exports" / "snapshots"
    snapshot_dir.mkdir(parents=True, exist_ok=True)
    
    snapshot_path = snapshot_dir / f"{snapshot_name}.{format}"
    snapshot_path.write_bytes(rendered)
    
    return snapshot_path
