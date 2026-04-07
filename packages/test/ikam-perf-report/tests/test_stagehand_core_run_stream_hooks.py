
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


class _FakeResponse:
    def __init__(self, *, ok: bool, status: int, payload: object) -> None:
        self.ok = ok
        self.status = status
        self._payload = payload

    def json(self) -> object:
        return self._payload


class _FakeRequest:
    def __init__(self, *, post_response: _FakeResponse, get_response: _FakeResponse) -> None:
        self._post_response = post_response
        self._get_response = get_response
        self.last_post: dict[str, object] | None = None
        self.last_get: dict[str, object] | None = None

    def post(self, url: str, data: dict[str, object], timeout: int) -> _FakeResponse:
        self.last_post = {"url": url, "data": data, "timeout": timeout}
        return self._post_response

    def get(self, url: str, params: dict[str, object], timeout: int) -> _FakeResponse:
        self.last_get = {"url": url, "params": params, "timeout": timeout}
        return self._get_response


class _FakePage:
    def __init__(self, request: _FakeRequest) -> None:
        self.request = request


class _FakeLocator:
    def __init__(self, click_log: list[str], name: str) -> None:
        self._click_log = click_log
        self._name = name

    @property
    def first(self) -> "_FakeLocator":
        return self

    def click(self) -> None:
        self._click_log.append(self._name)


class _FakeControlsRequest:
    def __init__(self) -> None:
        self._runs_calls = 0
        self._stream_calls = 0
        self.control_posts: list[dict[str, object]] = []

    def get(self, url: str, params: dict[str, object] | None = None, timeout: int = 60_000) -> _FakeResponse:
        assert timeout == 60_000
        if url.endswith("/benchmarks/cases"):
            return _FakeResponse(
                ok=True,
                status=200,
                payload={"cases": [{"case_id": "s-local-retail-v01", "domain": "retail", "size_tier": "s"}]},
            )
        if url.endswith("/benchmarks/runs"):
            self._runs_calls += 1
            return _FakeResponse(
                ok=True,
                status=200,
                payload=[
                    {
                        "run_id": "run-ctrl-1",
                        "case_id": "s-local-retail-v01",
                        "evaluation": {
                            "details": {
                                "debug_pipeline": {
                                    "pipeline_id": "compression-rerender/v1",
                                    "pipeline_run_id": "pipe-ctrl-1",
                                }
                            }
                        },
                    }
                ],
            )
        if "/debug-stream" in url:
            self._stream_calls += 1
            if self._stream_calls == 1:
                payload = {
                    "status": "ok",
                    "control_availability": {"can_resume": True, "can_next_step": True},
                    "events": [{"step_id": "step-prepare", "step_name": "prepare_case", "attempt_index": 1}],
                }
            elif self._stream_calls == 2:
                payload = {
                    "status": "ok",
                    "events": [{"step_id": "step-prepare", "step_name": "prepare_case", "attempt_index": 1}],
                }
            else:
                payload = {
                    "status": "ok",
                    "events": [
                        {"step_id": "step-prepare", "step_name": "prepare_case", "attempt_index": 1},
                        {"step_id": "step-decompose", "step_name": "map", "attempt_index": 1},
                    ],
                }
            return _FakeResponse(ok=True, status=200, payload=payload)
        raise AssertionError(f"Unexpected GET url: {url}")

    def post(self, url: str, data: dict[str, object], timeout: int = 60_000) -> _FakeResponse:
        assert timeout == 60_000
        if "/control" not in url:
            raise AssertionError(f"Unexpected POST url: {url}")
        self.control_posts.append({"url": url, "data": data})
        command_id = str(data.get("command_id", ""))
        action = str(data.get("action", ""))
        next_posts = 0
        for post in self.control_posts:
            data_payload = post.get("data")
            if not isinstance(data_payload, dict):
                continue
            if str(data_payload.get("command_id", "")).startswith("cmd-next"):
                next_posts += 1
        if action == "next_step" and command_id.startswith("cmd-next") and next_posts > 1:
            return _FakeResponse(ok=True, status=200, payload={"status": "duplicate"})
        return _FakeResponse(ok=True, status=200, payload={"status": "ok"})


class _FakeControlsPage:
    def __init__(self, request: _FakeControlsRequest) -> None:
        self.request = request
        self.click_log: list[str] = []

    def get_by_label(self, _pattern) -> _FakeLocator:
        return _FakeLocator(self.click_log, "case-checkbox")

    def get_by_role(self, role: str, name: str, exact: bool = True) -> _FakeLocator:
        assert role == "button"
        assert name == "Run Cases"
        assert exact is True
        return _FakeLocator(self.click_log, "run-button")


def test_core_run_stream_seed_requests_seed_endpoint_and_returns_identifiers() -> None:
    request = _FakeRequest(
        post_response=_FakeResponse(
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
        ),
        get_response=_FakeResponse(ok=True, status=200, payload={}),
    )
    page = _FakePage(request)

    result = _MODULE._scenario_seed_core_run_stream(page, "http://localhost:8040")

    assert result == {
        "run_id": "seed-run-1",
        "pipeline_id": "compression-rerender/v1",
        "pipeline_run_id": "pipe-seed-run-1",
    }
    assert request.last_post == {
        "url": "http://localhost:8040/benchmarks/test/seed-scenario",
        "data": {"scenario_key": "core_stream_baseline"},
        "timeout": 60_000,
    }


def test_core_run_stream_act_fetches_debug_stream_and_selects_first_step() -> None:
    request = _FakeRequest(
        post_response=_FakeResponse(ok=True, status=200, payload={}),
        get_response=_FakeResponse(
            ok=True,
            status=200,
            payload={
                "status": "ok",
                "events": [
                    {"step_id": "step-prepare", "step_name": "prepare_case", "attempt_index": 1},
                    {"step_id": "step-decompose", "step_name": "map", "attempt_index": 1},
                ],
            },
        ),
    )
    page = _FakePage(request)

    result = _MODULE._scenario_act_core_run_stream(
        page,
        "http://localhost:8040",
        {
            "run_id": "seed-run-1",
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-seed-run-1",
        },
    )

    assert request.last_get == {
        "url": "http://localhost:8040/benchmarks/runs/seed-run-1/debug-stream",
        "params": {
            "pipeline_id": "compression-rerender/v1",
            "pipeline_run_id": "pipe-seed-run-1",
        },
        "timeout": 60_000,
    }
    assert result["selected_step_id"] == "step-prepare"
    assert len(result["events"]) == 2
    assert result["debug_stream"]["status"] == "ok"


def test_core_run_stream_act_fails_fast_when_seed_run_id_missing() -> None:
    request = _FakeRequest(
        post_response=_FakeResponse(ok=True, status=200, payload={}),
        get_response=_FakeResponse(ok=True, status=200, payload={"status": "ok", "events": []}),
    )
    page = _FakePage(request)

    with pytest.raises(AssertionError, match="core_stream_missing_seed_run_id"):
        _MODULE._scenario_act_core_run_stream(page, "http://localhost:8040", {"pipeline_run_id": "pipe-x"})


def test_controls_modes_seed_and_act_use_run_button_and_manual_stepthrough() -> None:
    request = _FakeControlsRequest()
    page = _FakeControlsPage(request)

    seed = _MODULE._scenario_seed_controls_modes(page, "http://localhost:8040")
    result = _MODULE._scenario_act_controls_modes(page, "http://localhost:8040", seed)

    assert seed == {"case_id": "s-local-retail-v01"}
    assert page.click_log == ["case-checkbox", "run-button"]
    assert result["availability_before"] == {"can_resume": True, "can_next_step": True}
    assert len(result["before_events"]) == 1
    assert len(result["after_events"]) == 2
    statuses = [
        str(item["status"])
        for item in result["control_responses"]
        if isinstance(item, dict) and "status" in item
    ]
    assert "ok" in statuses
    assert "duplicate" in statuses
