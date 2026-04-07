from __future__ import annotations

import json
import os
import sys
import time
import urllib.error
import urllib.request

from modelado.core.execution_scope import DefaultExecutionScope


def _request_json(method: str, url: str, *, timeout_s: float = 60.0) -> dict:
    req = urllib.request.Request(url, method=method)
    req.add_header("Accept", "application/json")
    with urllib.request.urlopen(req, timeout=timeout_s) as resp:
        body = resp.read().decode("utf-8")
    return json.loads(body)


def _post_json(url: str, body: dict, *, timeout_s: float = 60.0) -> dict:
    data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, method="POST")
    req.add_header("Content-Type", "application/json")
    req.add_header("Accept", "application/json")
    with urllib.request.urlopen(req, timeout=timeout_s) as resp:
        body_str = resp.read().decode("utf-8")
    return json.loads(body_str)


def _wait_for_health(base_url: str, *, deadline_s: float = 60.0) -> None:
    started = time.time()
    last_error = None
    while time.time() - started < deadline_s:
        try:
            payload = _request_json("GET", f"{base_url}/health", timeout_s=5.0)
            if payload.get("status") == "ok":
                return
        except Exception as exc:
            last_error = exc
        time.sleep(0.5)
    raise RuntimeError(f"API not healthy after {deadline_s}s (last_error={last_error})")


def main() -> int:
    base_url = os.getenv("IKAM_PERF_REPORT_BASE_URL", "http://localhost:8040").rstrip("/")

    # Step 1: wait for healthy API
    _wait_for_health(base_url)

    # Step 2: POST /benchmarks/run — start run for the smoke case
    run_url = f"{base_url}/benchmarks/run?case_ids=s-construction-v01"
    run = _post_json(run_url, {})
    run_payload = run["runs"][0]
    run_id = run_payload["run_id"]

    debug_state = run_payload.get("debug_state", {})
    execution_state = debug_state.get("execution_state")
    current_step_name = debug_state.get("current_step_name")
    pipeline_id = debug_state.get("pipeline_id", "compression-rerender/v1")
    pipeline_run_id = debug_state.get("pipeline_run_id", run_id)

    if execution_state != "paused":
        raise AssertionError(
            f"expected execution_state='paused', got {execution_state!r}"
        )
    if current_step_name != "prepare_case":
        raise AssertionError(
            f"expected current_step_name='prepare_case', got {current_step_name!r}"
        )

    # Step 3: advance pipeline through the remaining steps after prepare_case
    dynamic_steps = DefaultExecutionScope().get_dynamic_execution_steps()
    control_url = f"{base_url}/benchmarks/runs/{run_id}/control"
    for i in range(len(dynamic_steps) - 1):
        command_id = f"cmd-{i}-{run_id[:8]}"
        _post_json(
            control_url,
            {
                "command_id": command_id,
                "action": "next_step",
                "pipeline_id": pipeline_id,
                "pipeline_run_id": pipeline_run_id,
            },
        )

    # Step 4: GET debug-stream and verify events in dynamic order, all succeeded
    stream_url = (
        f"{base_url}/benchmarks/runs/{run_id}/debug-stream"
        f"?pipeline_id={pipeline_id}&pipeline_run_id={pipeline_run_id}"
    )
    stream = _request_json("GET", stream_url)
    events_raw = stream.get("events") if isinstance(stream, dict) else stream
    events = events_raw if isinstance(events_raw, list) else []

    if len(events) != len(dynamic_steps):
        raise AssertionError(
            f"expected exactly {len(dynamic_steps)} debug events, got {len(events)}"
        )

    for idx, event in enumerate(events):
        expected = dynamic_steps[idx]
        step_name = event.get("step_name")
        if step_name != expected:
            raise AssertionError(
                f"event[{idx}] step_name={step_name!r}, expected {expected!r}"
            )

    # Step 5: GET debug-state and verify run completed
    debug_state_resp = _request_json(
        "GET", f"{base_url}/benchmarks/runs/{run_id}/debug-state"
    )
    final_state = debug_state_resp.get("execution_state")
    if final_state not in ("completed", "succeeded"):
        raise AssertionError(
            f"expected execution_state in ('completed', 'succeeded'), got {final_state!r}"
        )

    print("OK")
    print(
        json.dumps(
            {
                "run_id": run_id,
                "pipeline_id": pipeline_id,
                "pipeline_run_id": pipeline_run_id,
                "final_execution_state": final_state,
                "steps_verified": dynamic_steps,
            },
            indent=2,
            sort_keys=True,
        )
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except urllib.error.HTTPError as exc:
        sys.stderr.write(f"HTTPError: {exc.code} {exc.reason}\n")
        sys.stderr.write(exc.read().decode("utf-8") + "\n")
        raise
