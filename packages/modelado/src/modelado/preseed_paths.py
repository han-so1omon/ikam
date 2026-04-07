from __future__ import annotations

from pathlib import Path


def preseed_root() -> Path:
    return Path(__file__).resolve().parents[3] / "test" / "ikam-perf-report" / "preseed"


def preseed_compiled_dir() -> Path:
    return preseed_root() / "compiled"


def preseed_source_dirs() -> tuple[Path, ...]:
    root = preseed_root()
    return (root / "declarations", root / "workflows", root / "operators")


def preseed_graph_template_dirs() -> tuple[Path, ...]:
    root = preseed_root()
    return (root / "operators",)
