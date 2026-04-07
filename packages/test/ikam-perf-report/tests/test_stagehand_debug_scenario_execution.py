
from __future__ import annotations
import pytest
pytest.importorskip('playwright')

import importlib.util
from pathlib import Path


_SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "stagehand_perf_report.py"
_SPEC = importlib.util.spec_from_file_location("stagehand_perf_report", _SCRIPT)
if _SPEC is None or _SPEC.loader is None:
    raise RuntimeError(f"Unable to load stagehand script: {_SCRIPT}")
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)


class _FakeResponse:
    def __init__(self, *, ok: bool, status: int, payload: dict[str, object]) -> None:
        self.ok = ok
        self.status = status
        self._payload = payload

    def json(self) -> dict[str, object]:
        return self._payload


class _FakeRequest:
    def post(self, _url: str, data: dict[str, object], timeout: int) -> _FakeResponse:
        assert data == {"scenario_key": "core_stream_baseline"}
        assert timeout == 60_000
        return _FakeResponse(
            ok=True,
            status=200,
            payload={
                "status": "ok",
                "run": {
                    "run_id": "seed-run-1",
                    "pipeline_id": "compression-rerender/v1",
                    "pipeline_run_id": "pipe-seed-run-1",
                },
            },
        )

    def get(self, _url: str, params: dict[str, object], timeout: int) -> _FakeResponse:
        assert params == {
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-seed-run-1",
        }
        assert timeout == 60_000
        return _FakeResponse(
            ok=True,
            status=200,
            payload={
                "status": "ok",
                "events": [
                    {
                        "step_id": "step-prepare",
                        "step_name": "prepare_case",
                        "attempt_index": 1,
                        "status": "succeeded",
                    }
                ],
            },
        )


class _FakePage:
    def __init__(self) -> None:
        self.request = _FakeRequest()


def test_execute_debug_validation_scenario_core_run_stream(tmp_path: Path) -> None:
    screenshot_path = tmp_path / "ui.png"
    video_path = tmp_path / "video.mp4"
    screenshot_path.write_bytes(b"png")
    video_path.write_bytes(b"video")

    result = _MODULE.execute_debug_validation_scenario(
        scenario_key="core_run_stream",
        page=_FakePage(),
        api_url="http://localhost:8040",
        out_dir=tmp_path,
        screenshot_path=screenshot_path,
        video_path=video_path,
        console_lines=["info: test"],
        network_events=[{"url": "/benchmarks/runs/seed-run-1/debug-stream", "status": 200}],
    )

    assert result["status"] == "pass"
    assert result["scenario_key"] == "core_run_stream"


def test_maybe_run_debug_validation_scenario_returns_none_without_key(tmp_path: Path) -> None:
    result = _MODULE.maybe_run_debug_validation_scenario(
        scenario_key="",
        page=_FakePage(),
        api_url="http://localhost:8040",
        out_dir=tmp_path,
        screenshot_path=tmp_path / "ui.png",
        video_path=tmp_path / "video.mp4",
        console_lines=[],
        network_events=[],
    )

    assert result is None
