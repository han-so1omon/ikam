from pathlib import Path

import pytest


def test_perf_report_docs_linked():
    repo_root = Path(__file__).resolve().parents[4]
    docs_index = repo_root / "docs/INDEX.md"
    if not docs_index.exists():
        pytest.skip("docs index not mounted in this test environment")
    text = docs_index.read_text()
    assert "ikam-perf-report.md" in text
