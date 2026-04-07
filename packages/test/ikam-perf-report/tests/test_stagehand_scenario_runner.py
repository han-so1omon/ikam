
from __future__ import annotations
import pytest
pytest.importorskip('playwright')

import importlib.util
import json
from pathlib import Path

import pytest


_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "stagehand_perf_report.py"
_SPEC = importlib.util.spec_from_file_location("stagehand_perf_report", _SCRIPT)
if _SPEC is None or _SPEC.loader is None:
    raise RuntimeError(f"Unable to load stagehand script: {_SCRIPT}")
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)


def _touch(path: Path, text: str = "x") -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_run_scenario_validation_writes_bundle_and_passes(tmp_path: Path) -> None:
    video = tmp_path / "video.mp4"
    screenshot = tmp_path / "shot.png"
    _touch(video, "video")
    _touch(screenshot, "shot")

    context = {
        "events": [{"attempt_index": 1, "step_name": "prepare_case", "step_id": "s1"}],
        "selected_step_id": "s1",
        "agentic_checkpoints": [
            {
                "name": "checkpoint-1",
                "prompt": "Describe stream",
                "response": "Observed: stream visible\nNot Observed: none\nUncertain: none",
            }
        ],
        "video_path": video,
        "screenshot_path": screenshot,
        "console_lines": ["info: ok"],
        "network_events": [{"url": "/benchmarks/runs", "status": 200}],
        "debug_stream": {"status": "ok", "events": [{"step_name": "prepare_case"}]},
        "control_responses": [],
    }

    result = _MODULE.run_scenario_validation(
        scenario_key="core_run_stream",
        context=context,
        out_dir=tmp_path,
    )
    assert result["status"] == "pass"

    verdict = json.loads(Path(result["bundle"]["files"]["verdict"]).read_text(encoding="utf-8"))
    assert verdict["status"] == "pass"


def test_run_scenario_validation_blocks_agentic_mismatch(tmp_path: Path) -> None:
    video = tmp_path / "video.mp4"
    screenshot = tmp_path / "shot.png"
    _touch(video, "video")
    _touch(screenshot, "shot")

    context = {
        "events": [
            {"attempt_index": 1, "step_name": "verify", "step_id": "v1", "status": "failed", "retry_parent_step_id": None},
            {"attempt_index": 2, "step_name": "map", "step_id": "d2", "retry_parent_step_id": "v1"},
        ],
        "agentic_checkpoints": [
            {
                "name": "checkpoint-1",
                "prompt": "Did retry happen?",
                "response": "Observed: none\nNot Observed: retry boundary\nUncertain: none",
            }
        ],
        "video_path": video,
        "screenshot_path": screenshot,
        "console_lines": [],
        "network_events": [],
        "debug_stream": {"status": "ok", "events": []},
        "control_responses": [],
    }

    with pytest.raises(AssertionError):
        _MODULE.run_scenario_validation(
            scenario_key="deterministic_retry",
            context=context,
            out_dir=tmp_path,
        )
