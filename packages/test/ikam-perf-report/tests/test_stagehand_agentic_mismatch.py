
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


def test_agentic_mismatch_blocks_when_contradiction_detected(tmp_path: Path) -> None:
    deterministic = {"retry_boundary": True}
    checkpoints = [
        {
            "name": "retry-observation",
            "prompt": "Describe whether retry boundary is visible",
            "response": "Observed: none\nNot Observed: retry boundary\nUncertain: none",
        }
    ]

    with pytest.raises(AssertionError):
        _MODULE.evaluate_agentic_checkpoints(
            deterministic=deterministic,
            checkpoints=checkpoints,
            out_dir=tmp_path,
            scenario_key="deterministic_retry",
        )


def test_agentic_outputs_are_persisted_per_checkpoint(tmp_path: Path) -> None:
    deterministic = {"retry_boundary": False}
    checkpoints = [
        {
            "name": "core-observation",
            "prompt": "Describe visible stream",
            "response": "Observed: stream visible\nNot Observed: none\nUncertain: none",
        }
    ]

    result = _MODULE.evaluate_agentic_checkpoints(
        deterministic=deterministic,
        checkpoints=checkpoints,
        out_dir=tmp_path,
        scenario_key="core_run_stream",
    )
    assert result["status"] == "ok"

    files = list(tmp_path.glob("core_run_stream-agentic-checkpoint-*.md"))
    assert files, "expected raw agentic checkpoint artifacts"
    text = files[0].read_text(encoding="utf-8")
    assert "Describe visible stream" in text
    assert "Observed:" in text
