
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


def test_emit_scenario_artifact_bundle_writes_required_files(tmp_path: Path) -> None:
    video = tmp_path / "video.mp4"
    screenshot = tmp_path / "shot.png"
    agentic_md = tmp_path / "agentic.md"
    video.write_bytes(b"video")
    screenshot.write_bytes(b"image")
    agentic_md.write_text("Observed: stream visible", encoding="utf-8")

    result = _MODULE.emit_scenario_artifact_bundle(
        base_out_dir=tmp_path,
        scenario_key="core_run_stream",
        video_path=video,
        screenshot_path=screenshot,
        console_lines=["info: ok"],
        network_events=[{"url": "/benchmarks/runs", "status": 200}],
        debug_stream={"status": "ok", "events": [{"step_name": "prepare_case"}]},
        control_responses=[{"action": "next_step", "status": "ok"}],
        deterministic_result={"status": "ok"},
        agentic_result={"status": "ok"},
        agentic_artifacts=[agentic_md],
    )

    out_dir = Path(result["scenario_dir"])
    assert (out_dir / "console.log").exists()
    assert (out_dir / "network.json").exists()
    assert (out_dir / "debug-stream.json").exists()
    assert (out_dir / "control-responses.json").exists()
    verdict = json.loads((out_dir / "verdict.json").read_text(encoding="utf-8"))
    assert verdict["status"] == "pass"


def test_emit_scenario_artifact_bundle_requires_agentic_artifacts(tmp_path: Path) -> None:
    video = tmp_path / "video.mp4"
    screenshot = tmp_path / "shot.png"
    video.write_bytes(b"video")
    screenshot.write_bytes(b"image")

    with pytest.raises(AssertionError):
        _MODULE.emit_scenario_artifact_bundle(
            base_out_dir=tmp_path,
            scenario_key="controls_modes",
            video_path=video,
            screenshot_path=screenshot,
            console_lines=[],
            network_events=[],
            debug_stream={},
            control_responses=[],
            deterministic_result={"status": "ok"},
            agentic_result={"status": "ok"},
            agentic_artifacts=[],
        )
