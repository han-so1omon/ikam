from __future__ import annotations

from pathlib import Path


def test_perf_report_declares_psycopg_pool_runtime_dependency() -> None:
    pyproject = Path(__file__).resolve().parents[1] / "pyproject.toml"
    text = pyproject.read_text(encoding="utf-8")
    assert "psycopg-pool" in text
