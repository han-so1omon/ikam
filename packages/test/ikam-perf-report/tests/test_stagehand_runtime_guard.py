
from __future__ import annotations
import pytest
pytest.importorskip('playwright')

import importlib.util
from pathlib import Path

import pytest


_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "stagehand_perf_report.py"
_SPEC = importlib.util.spec_from_file_location("stagehand_perf_report", _SCRIPT)
if _SPEC is None or _SPEC.loader is None:
    raise RuntimeError(f"Unable to load stagehand script: {_SCRIPT}")
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)


def test_runtime_guard_rejects_python_3_14_plus() -> None:
    with pytest.raises(RuntimeError):
        _MODULE.validate_stagehand_runtime((3, 14, 1), "darwin")


def test_runtime_guard_allows_python_3_12() -> None:
    _MODULE.validate_stagehand_runtime((3, 12, 9), "darwin")
