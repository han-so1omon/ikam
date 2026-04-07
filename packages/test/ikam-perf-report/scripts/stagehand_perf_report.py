from __future__ import annotations

import json
import os
import re
import shutil
import sys
import time
import importlib.util
from datetime import datetime, timezone
from pathlib import Path
from uuid import uuid4

from playwright.sync_api import ConsoleMessage, Page, Request, Response, sync_playwright

from ikam_perf_report.benchmarks.stagehand_validations import (
    build_search_query_candidates,
    classify_agentic_mismatch,
    parse_agentic_footer,
    validate_debug_pipeline_contract,
    resolve_evaluation_case_id,
    validate_evaluation_report,
    validate_visual_pass_signals,
    validate_wiki_document,
)
from modelado.core.execution_scope import DefaultExecutionScope
from modelado.fragment_embedder import get_shared_embedder

_ASSERT_HELPER = Path(__file__).parent / "qa" / "assert_graph_rendering.py"
_SPEC = importlib.util.spec_from_file_location("assert_graph_rendering", _ASSERT_HELPER)
if _SPEC is None or _SPEC.loader is None:
    raise RuntimeError(f"Unable to load helper module: {_ASSERT_HELPER}")
_MODULE = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(_MODULE)
assert_graph_crop_non_uniform = _MODULE.assert_graph_crop_non_uniform


def _now_slug() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def get_default_perf_api_url() -> str:
    # Keep in sync with packages/test/ikam-perf-report/docker-compose.yml default published port.
    return "http://localhost:8040"


def validate_stagehand_runtime(version_info: tuple[int, int, int], platform_name: str) -> None:
    """Fail fast on known-incompatible runtime combinations.

    We do not support fallback execution paths for Stagehand runtime incompatibilities.
    """
    major, minor, _patch = version_info
    if major == 3 and minor >= 14:
        raise RuntimeError(
            "Unsupported Python runtime for stagehand_perf_report.py: "
            f"{major}.{minor} on {platform_name}. "
            "Use Python 3.11 or 3.12 with Playwright to run Stagehand validations."
        )


def _parse_int(text: str) -> int:
    digits = "".join(ch for ch in text if ch.isdigit())
    return int(digits) if digits else 0


def _get_metric_value(page: Page, label: str) -> str:
    metric = page.locator(".metric").filter(
        has=page.locator(".metric-label", has_text=re.compile(rf"^{re.escape(label)}$"))
    )
    return metric.locator(".metric-value").first.inner_text().strip()


def _assert_heading(page: Page, text: str) -> None:
    page.get_by_role("heading", name=text, exact=True).wait_for(state="visible", timeout=30_000)


def _rect_samples(page: Page, test_id: str, count: int = 5, interval_s: float = 0.3) -> list[dict[str, float]]:
    values: list[dict[str, float]] = []
    locator = page.get_by_test_id(test_id)
    for _ in range(count):
        box = locator.bounding_box()
        if box:
            values.append(
                {
                    "x": float(box["x"]),
                    "y": float(box["y"]),
                    "width": float(box["width"]),
                    "height": float(box["height"]),
                }
            )
        time.sleep(interval_s)
    return values


def _assert_rect_stable(samples: list[dict[str, float]], max_drift_px: float = 8.0) -> None:
    if len(samples) < 2:
        raise AssertionError({"error": "insufficient_rect_samples", "count": len(samples)})

    # Ignore early layout shifts (webfont/hydration) and enforce stability on tail samples.
    tail = samples[-3:] if len(samples) >= 3 else samples
    widths = [sample["width"] for sample in tail]
    heights = [sample["height"] for sample in tail]
    if max(widths) - min(widths) > max_drift_px:
        raise AssertionError({"error": "graph_width_drift", "samples": samples, "tail": tail})
    if max(heights) - min(heights) > max_drift_px:
        raise AssertionError({"error": "graph_height_drift", "samples": samples, "tail": tail})

    # Detect continual runaway expansion: multiple consecutive growth steps.
    growth_steps = 0
    for idx in range(1, len(tail)):
        if widths[idx] - widths[idx - 1] > 3:
            growth_steps += 1
    if growth_steps >= len(tail) - 1 and len(tail) >= 3:
        raise AssertionError({"error": "graph_width_runaway_growth", "samples": samples, "tail": tail})


def _poll_graph_data(page: Page, api_url: str, graph_id: str, timeout_s: float = 40.0) -> tuple[list[dict], list[dict]]:
    started = time.time()
    last_nodes: list[dict] = []
    last_edges: list[dict] = []
    while time.time() - started < timeout_s:
        nodes_resp = page.request.get(f"{api_url}/graph/nodes", params={"graph_id": graph_id}, timeout=60_000)
        edges_resp = page.request.get(f"{api_url}/graph/edges", params={"graph_id": graph_id}, timeout=60_000)
        if not nodes_resp.ok or not edges_resp.ok:
            time.sleep(1.0)
            continue
        nodes_data = nodes_resp.json()
        edges_data = edges_resp.json()
        if isinstance(nodes_data, list):
            last_nodes = nodes_data
        if isinstance(edges_data, list):
            last_edges = edges_data
        if last_nodes and last_edges:
            return last_nodes, last_edges
        time.sleep(1.0)
    return last_nodes, last_edges


def _scenario_seed_stub(*args, **kwargs) -> dict:
    return {"status": "stub"}


def _scenario_act_stub(*args, **kwargs) -> dict:
    return {"status": "stub"}


def _scenario_assert_contract_stub(*args, **kwargs) -> dict:
    return {"status": "stub"}


def _scenario_agentic_checkpoints_stub(*args, **kwargs) -> dict:
    return {"status": "stub"}


def _scenario_collect_artifacts_stub(*args, **kwargs) -> dict:
    return {"status": "stub"}


def _scenario_agentic_checkpoints_core_run_stream(context: dict[str, object]) -> list[dict[str, str]]:
    events_raw = context.get("events")
    events: list[dict[str, object]] = []
    if isinstance(events_raw, list):
        events = [event for event in events_raw if isinstance(event, dict)]
    has_events = len(events) > 0
    selected_step_id = context.get("selected_step_id")
    selected_present = False
    if has_events and isinstance(selected_step_id, str) and selected_step_id:
        selected_present = any(isinstance(event, dict) and event.get("step_id") == selected_step_id for event in events)

    observed: list[str] = []
    not_observed: list[str] = []
    if has_events:
        observed.append("stream visible")
    else:
        not_observed.append("stream visible")
    if selected_present:
        observed.append("selected step resolved")
    else:
        not_observed.append("selected step resolved")

    response = "\n".join(
        [
            f"Observed: {', '.join(observed) if observed else 'none'}",
            f"Not Observed: {', '.join(not_observed) if not_observed else 'none'}",
            "Uncertain: none",
        ]
    )

    return [
        {
            "name": "core_run_stream_visibility",
            "prompt": "Check whether debug stream appears immediately with a selectable step.",
            "response": response,
        }
    ]


def _scenario_seed_core_run_stream(page: Page, api_url: str) -> dict[str, object]:
    response = page.request.post(
        f"{api_url}/benchmarks/test/seed-scenario",
        data={"scenario_key": "core_stream_baseline"},
        timeout=60_000,
    )
    if not response.ok:
        raise AssertionError({"error": "seed_core_run_stream_failed", "status": response.status})
    raw_payload = response.json()
    payload: dict[str, object] = raw_payload if isinstance(raw_payload, dict) else {}
    run_raw = payload.get("run")
    run_info: dict[str, object] = run_raw if isinstance(run_raw, dict) else {}
    run_id = str(run_info.get("run_id", "")).strip()
    pipeline_id = str(run_info.get("pipeline_id", "compression-rerender/v1")).strip() or "compression-rerender/v1"
    pipeline_run_id = str(run_info.get("pipeline_run_id", "")).strip()
    if not run_id:
        raise AssertionError({"error": "seed_core_run_stream_missing_run_id", "payload": payload})
    if not pipeline_run_id:
        raise AssertionError({"error": "seed_core_run_stream_missing_pipeline_run_id", "payload": payload})

    return {
        "run_id": run_id,
        "pipeline_id": pipeline_id,
        "pipeline_run_id": pipeline_run_id,
    }


def _scenario_act_core_run_stream(page: Page, api_url: str, seed_result: dict[str, object]) -> dict[str, object]:
    run_id = str(seed_result.get("run_id", ""))
    pipeline_id = str(seed_result.get("pipeline_id", "compression-rerender/v1"))
    pipeline_run_id = str(seed_result.get("pipeline_run_id", run_id))
    if not run_id:
        raise AssertionError({"error": "core_stream_missing_seed_run_id", "seed_result": seed_result})
    if not pipeline_run_id:
        raise AssertionError({"error": "core_stream_missing_seed_pipeline_run_id", "seed_result": seed_result})

    response = page.request.get(
        f"{api_url}/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": pipeline_id, "pipeline_run_id": pipeline_run_id},
        timeout=60_000,
    )
    if not response.ok:
        raise AssertionError({"error": "core_stream_fetch_failed", "status": response.status, "run_id": run_id})

    raw_payload = response.json()
    payload: dict[str, object] = raw_payload if isinstance(raw_payload, dict) else {}
    events_raw = payload.get("events")
    events: list[dict[str, object]] = []
    if isinstance(events_raw, list):
        events = [event for event in events_raw if isinstance(event, dict)]

    selected_step_id: str | None = None
    if events:
        first_step = str(events[0].get("step_id", "")).strip()
        selected_step_id = first_step or None

    return {
        "events": events,
        "selected_step_id": selected_step_id,
        "debug_stream": payload,
        "control_responses": [],
        "network_events": [],
        "console_lines": [],
    }


def _scenario_seed_controls_modes(page: Page, api_url: str) -> dict[str, object]:
    response = page.request.get(f"{api_url}/benchmarks/cases", timeout=60_000)
    if not response.ok:
        raise AssertionError({"error": "controls_seed_cases_fetch_failed", "status": response.status})
    payload = response.json()
    if not isinstance(payload, dict):
        raise AssertionError({"error": "controls_seed_cases_payload_invalid", "payload": payload})
    cases = payload.get("cases")
    if not isinstance(cases, list) or not cases:
        raise AssertionError({"error": "controls_seed_no_cases"})
    first_case = cases[0] if isinstance(cases[0], dict) else {}
    case_id = str(first_case.get("case_id", "")).strip()
    if not case_id:
        raise AssertionError({"error": "controls_seed_missing_case_id", "first_case": first_case})
    return {"case_id": case_id}


def _scenario_act_controls_modes(page: Page, api_url: str, seed_result: dict[str, object]) -> dict[str, object]:
    case_id = str(seed_result.get("case_id", "")).strip()
    if not case_id:
        raise AssertionError({"error": "controls_missing_case_id", "seed_result": seed_result})

    page.get_by_label(re.compile(re.escape(case_id), re.IGNORECASE)).first.click()
    page.get_by_role("button", name="Run Cases", exact=True).click()

    run_id = ""
    pipeline_id = "compression-rerender/v1"
    pipeline_run_id = ""
    for _ in range(30):
        runs_response = page.request.get(f"{api_url}/benchmarks/runs", timeout=60_000)
        if runs_response.ok:
            runs_payload = runs_response.json()
            if isinstance(runs_payload, list):
                matched = next(
                    (
                        run
                        for run in runs_payload
                        if isinstance(run, dict) and str(run.get("case_id", "")).strip().lower() == case_id.lower()
                    ),
                    None,
                )
                if isinstance(matched, dict):
                    run_id = str(matched.get("run_id", "")).strip()
                    debug_pipeline = ((matched.get("evaluation") or {}).get("details") or {}).get("debug_pipeline")
                    if isinstance(debug_pipeline, dict):
                        pipeline_id = str(debug_pipeline.get("pipeline_id", pipeline_id)).strip() or pipeline_id
                        pipeline_run_id = str(debug_pipeline.get("pipeline_run_id", "")).strip()
                    if run_id:
                        break
        time.sleep(1.0)

    if not run_id:
        raise AssertionError({"error": "controls_run_not_found", "case_id": case_id})
    if not pipeline_run_id:
        pipeline_run_id = run_id

    stream_before_response = page.request.get(
        f"{api_url}/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": pipeline_id, "pipeline_run_id": pipeline_run_id},
        timeout=60_000,
    )
    if not stream_before_response.ok:
        raise AssertionError({"error": "controls_stream_before_failed", "status": stream_before_response.status})
    stream_before_payload = stream_before_response.json()
    if not isinstance(stream_before_payload, dict):
        raise AssertionError({"error": "controls_stream_before_invalid_payload"})

    availability_before = stream_before_payload.get("control_availability")

    command_suffix = uuid4().hex[:8]
    control_responses: list[dict[str, object]] = []

    def _post_control(action: str, *, mode: str | None = None, command_id: str | None = None) -> dict[str, object]:
        payload: dict[str, object] = {
            "command_id": command_id or f"cmd-{action}-{command_suffix}",
            "action": action,
            "pipeline_id": pipeline_id,
            "pipeline_run_id": pipeline_run_id,
        }
        if mode:
            payload["mode"] = mode
        response = page.request.post(
            f"{api_url}/benchmarks/runs/{run_id}/control",
            data=payload,
            timeout=60_000,
        )
        if not response.ok:
            raise AssertionError({"error": "controls_action_failed", "action": action, "status": response.status})
        body = response.json()
        if not isinstance(body, dict):
            raise AssertionError({"error": "controls_action_invalid_payload", "action": action})
        control_responses.append({"action": action, "status": str(body.get("status", "")), "payload": body})
        return body

    _post_control("set_mode", mode="manual")
    _post_control("pause")
    _post_control("resume")
    _post_control("pause", command_id=f"cmd-pause-before-next-{command_suffix}")

    stream_mid_response = page.request.get(
        f"{api_url}/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": pipeline_id, "pipeline_run_id": pipeline_run_id},
        timeout=60_000,
    )
    if not stream_mid_response.ok:
        raise AssertionError({"error": "controls_stream_mid_failed", "status": stream_mid_response.status})
    stream_mid_payload = stream_mid_response.json()
    if not isinstance(stream_mid_payload, dict):
        raise AssertionError({"error": "controls_stream_mid_invalid_payload"})
    before_events_raw = stream_mid_payload.get("events")
    before_events = before_events_raw if isinstance(before_events_raw, list) else []

    next_step_command_id = f"cmd-next-{command_suffix}"
    _post_control("next_step", command_id=next_step_command_id)
    _post_control("next_step", command_id=next_step_command_id)

    stream_after_response = page.request.get(
        f"{api_url}/benchmarks/runs/{run_id}/debug-stream",
        params={"pipeline_id": pipeline_id, "pipeline_run_id": pipeline_run_id},
        timeout=60_000,
    )
    if not stream_after_response.ok:
        raise AssertionError({"error": "controls_stream_after_failed", "status": stream_after_response.status})
    stream_after_payload = stream_after_response.json()
    if not isinstance(stream_after_payload, dict):
        raise AssertionError({"error": "controls_stream_after_invalid_payload"})
    after_events_raw = stream_after_payload.get("events")
    after_events = after_events_raw if isinstance(after_events_raw, list) else []

    selected_step_id = None
    if after_events and isinstance(after_events[0], dict):
        selected_step_id = str(after_events[0].get("step_id", "")).strip() or None

    return {
        "run_id": run_id,
        "pipeline_id": pipeline_id,
        "pipeline_run_id": pipeline_run_id,
        "availability_before": availability_before,
        "before_events": before_events,
        "after_events": after_events,
        "events": after_events,
        "selected_step_id": selected_step_id,
        "debug_stream": stream_after_payload,
        "control_responses": control_responses,
        "network_events": [],
        "console_lines": [],
    }


def _scenario_agentic_checkpoints_controls_modes(context: dict[str, object]) -> list[dict[str, str]]:
    responses_raw = context.get("control_responses")
    responses = responses_raw if isinstance(responses_raw, list) else []
    actions = [str(item.get("action", "")) for item in responses if isinstance(item, dict)]
    before_events_raw = context.get("before_events")
    before_events = before_events_raw if isinstance(before_events_raw, list) else []
    after_events_raw = context.get("after_events")
    after_events = after_events_raw if isinstance(after_events_raw, list) else []
    observed: list[str] = []
    not_observed: list[str] = []
    if "next_step" in actions and len(after_events) == len(before_events) + 1:
        observed.append("manual one-step advancement")
    else:
        not_observed.append("manual one-step advancement")
    if any(isinstance(item, dict) and str(item.get("status", "")) == "duplicate" for item in responses):
        observed.append("duplicate idempotency")
    else:
        not_observed.append("duplicate idempotency")

    response = "\n".join(
        [
            f"Observed: {', '.join(observed) if observed else 'none'}",
            f"Not Observed: {', '.join(not_observed) if not_observed else 'none'}",
            "Uncertain: none",
        ]
    )
    return [
        {
            "name": "controls_modes_stepthrough",
            "prompt": "Check whether manual controls advanced exactly one step and duplicate command id was non-mutating.",
            "response": response,
        }
    ]


def _build_scenario_definition() -> dict[str, object]:
    return {
        "seed": _scenario_seed_stub,
        "act": _scenario_act_stub,
        "assert_contract": _scenario_assert_contract_stub,
        "agentic_checkpoints": _scenario_agentic_checkpoints_stub,
        "collect_artifacts": _scenario_collect_artifacts_stub,
    }


def _canonical_prefix(step_names: list[str]) -> bool:
    if not step_names:
        return True
    dynamic_steps = DefaultExecutionScope().get_dynamic_execution_steps()
    if len(step_names) > len(dynamic_steps):
        return False
    return tuple(step_names) == tuple(dynamic_steps[: len(step_names)])


def _assert_core_run_stream_contract(context: dict[str, object]) -> dict[str, object]:
    events = context.get("events")
    if not isinstance(events, list) or not events:
        raise AssertionError({"error": "missing_events"})

    dynamic_steps = DefaultExecutionScope().get_dynamic_execution_steps()
    attempts: dict[int, list[str]] = {}
    for event in events:
        if not isinstance(event, dict):
            continue
        attempt = int(event.get("attempt_index", 0) or 0)
        if attempt <= 0:
            raise AssertionError({"error": "invalid_attempt_index", "event": event})
        step_name = str(event.get("step_name", ""))
        if step_name not in dynamic_steps:
            raise AssertionError({"error": "invalid_step_name", "step_name": step_name})
        attempts.setdefault(attempt, []).append(step_name)

    if not attempts:
        raise AssertionError({"error": "no_attempt_groups"})

    for attempt, names in attempts.items():
        if not _canonical_prefix(names):
            raise AssertionError({"error": "non_canonical_step_order", "attempt": attempt, "steps": names})

    selected_step_id = context.get("selected_step_id")
    if isinstance(selected_step_id, str) and selected_step_id:
        exists = any(isinstance(event, dict) and event.get("step_id") == selected_step_id for event in events)
        if not exists:
            raise AssertionError({"error": "selected_step_missing_from_stream", "selected_step_id": selected_step_id})

    return {"status": "ok", "attempts": sorted(attempts)}


def _assert_controls_modes_contract(context: dict[str, object]) -> dict[str, object]:
    responses = context.get("control_responses")
    if not isinstance(responses, list) or not responses:
        raise AssertionError({"error": "missing_control_responses"})

    availability_before = context.get("availability_before")
    if not isinstance(availability_before, dict):
        raise AssertionError({"error": "missing_control_availability_signal"})
    if availability_before.get("can_resume") is not True:
        raise AssertionError({"error": "resume_not_enabled_by_backend_signal", "availability": availability_before})
    if availability_before.get("can_next_step") is not True:
        raise AssertionError({"error": "next_step_not_enabled_by_backend_signal", "availability": availability_before})

    actions_seen: set[str] = set()
    duplicate_seen = False
    for item in responses:
        if not isinstance(item, dict):
            continue
        action = str(item.get("action", ""))
        actions_seen.add(action)
        status = str(item.get("status", ""))
        if status not in {"ok", "duplicate"}:
            raise AssertionError({"error": "invalid_control_status", "status": status, "action": action})
        if status == "duplicate":
            duplicate_seen = True

    required_actions = {"set_mode", "pause", "resume", "next_step"}
    if not required_actions.issubset(actions_seen):
        missing = sorted(required_actions - actions_seen)
        raise AssertionError({"error": "missing_control_actions", "missing": missing})

    if not duplicate_seen:
        raise AssertionError({"error": "missing_duplicate_idempotency_evidence"})

    before_events = context.get("before_events")
    after_events = context.get("after_events")
    if isinstance(before_events, list) and isinstance(after_events, list):
        if len(after_events) - len(before_events) != 1:
            raise AssertionError(
                {
                    "error": "next_step_not_single_boundary",
                    "before": len(before_events),
                    "after": len(after_events),
                }
            )

    return {"status": "ok", "actions_seen": sorted(actions_seen)}


def _assert_deterministic_retry_contract(context: dict[str, object]) -> dict[str, object]:
    events = context.get("events")
    if not isinstance(events, list) or not events:
        raise AssertionError({"error": "missing_events"})

    retry_event = None
    for event in reversed(events):
        if isinstance(event, dict) and event.get("retry_parent_step_id"):
            retry_event = event
            break
    if not isinstance(retry_event, dict):
        raise AssertionError({"error": "missing_retry_boundary_event"})

    parent_id = str(retry_event.get("retry_parent_step_id"))
    parent = next((e for e in events if isinstance(e, dict) and e.get("step_id") == parent_id), None)
    if not isinstance(parent, dict):
        raise AssertionError({"error": "missing_retry_parent_event", "retry_parent_step_id": parent_id})
    if parent.get("step_name") != "verify":
        raise AssertionError({"error": "retry_parent_not_verify", "parent": parent})
    if parent.get("status") != "failed":
        raise AssertionError({"error": "retry_parent_not_failed", "parent": parent})

    parent_attempt = int(parent.get("attempt_index", 0) or 0)
    retry_attempt = int(retry_event.get("attempt_index", 0) or 0)
    if retry_attempt <= parent_attempt:
        raise AssertionError(
            {
                "error": "retry_attempt_not_incremented",
                "parent_attempt": parent_attempt,
                "retry_attempt": retry_attempt,
            }
        )

    if str(retry_event.get("step_name", "")) != "map":
        raise AssertionError({"error": "retry_reentry_step_mismatch", "step_name": retry_event.get("step_name")})

    return {
        "status": "ok",
        "retry_boundary": True,
        "retry_parent_step_id": parent_id,
        "retry_attempt": retry_attempt,
    }


def _assert_env_scoped_drillthrough_contract(context: dict[str, object]) -> dict[str, object]:
    fragments = context.get("fragments")
    verification_records = context.get("verification_records")
    reconstruction_programs = context.get("reconstruction_programs")
    summary = context.get("env_summary")

    if not isinstance(fragments, list) or not fragments:
        raise AssertionError({"error": "missing_scoped_fragments"})
    if not isinstance(verification_records, list) or not verification_records:
        raise AssertionError({"error": "missing_scoped_verification_records"})
    if not isinstance(reconstruction_programs, list) or not reconstruction_programs:
        raise AssertionError({"error": "missing_scoped_reconstruction_programs"})
    if not isinstance(summary, dict):
        raise AssertionError({"error": "missing_env_summary"})

    for fragment in fragments:
        if not isinstance(fragment, dict):
            raise AssertionError({"error": "invalid_fragment_payload", "fragment": fragment})
        if "value" not in fragment:
            raise AssertionError({"error": "fragment_missing_value", "fragment": fragment})
        meta = fragment.get("meta")
        if not isinstance(meta, dict):
            raise AssertionError({"error": "fragment_missing_meta", "fragment": fragment})

    verification_count = int(summary.get("verification_count", -1) or -1)
    reconstruction_count = int(summary.get("reconstruction_program_count", -1) or -1)
    if verification_count != len(verification_records):
        raise AssertionError(
            {
                "error": "verification_count_mismatch",
                "summary": verification_count,
                "records": len(verification_records),
            }
        )
    if reconstruction_count != len(reconstruction_programs):
        raise AssertionError(
            {
                "error": "reconstruction_count_mismatch",
                "summary": reconstruction_count,
                "records": len(reconstruction_programs),
            }
        )

    return {
        "status": "ok",
        "fragment_count": len(fragments),
        "verification_count": len(verification_records),
        "reconstruction_count": len(reconstruction_programs),
    }


def _assert_evaluation_alignment_contract(context: dict[str, object]) -> dict[str, object]:
    payload = context.get("evaluation_payload")
    if not isinstance(payload, dict):
        raise AssertionError({"error": "missing_evaluation_payload"})
    contract = validate_debug_pipeline_contract(payload)
    return {"status": "ok", "contract": contract}


def _assert_cross_env_isolation_contract(context: dict[str, object]) -> dict[str, object]:
    scopes = context.get("scopes")
    write_delta = context.get("write_delta")
    cross_env_read_copy = context.get("cross_env_read_copy")
    delete_without_delta_error = context.get("delete_without_delta_error")
    delete_with_delta_ok = context.get("delete_with_delta_ok")

    if not isinstance(scopes, dict):
        raise AssertionError({"error": "missing_scopes"})
    required_envs = {"dev", "staging", "committed"}
    if not required_envs.issubset(set(scopes.keys())):
        raise AssertionError({"error": "missing_environment_visibility", "scopes": scopes})

    if not isinstance(write_delta, dict):
        raise AssertionError({"error": "missing_write_delta"})
    active_env = write_delta.get("active_env")
    mutated_envs = write_delta.get("mutated_envs")
    if not isinstance(mutated_envs, list):
        raise AssertionError({"error": "invalid_mutated_envs", "write_delta": write_delta})
    if mutated_envs != [active_env]:
        raise AssertionError({"error": "write_not_isolated", "write_delta": write_delta})

    if bool(cross_env_read_copy):
        raise AssertionError({"error": "cross_env_read_caused_copy"})
    if not bool(delete_without_delta_error):
        raise AssertionError({"error": "missing_delta_intent_enforcement"})
    if not bool(delete_with_delta_ok):
        raise AssertionError({"error": "delta_intent_success_missing"})

    return {"status": "ok", "active_env": active_env}


def _build_scenario_definition_with_assert(
    assert_fn,
    *,
    seed=None,
    act=None,
    agentic_checkpoints=None,
    collect_artifacts=None,
) -> dict[str, object]:
    return {
        "seed": seed or _scenario_seed_stub,
        "act": act or _scenario_act_stub,
        "assert_contract": assert_fn,
        "agentic_checkpoints": agentic_checkpoints or _scenario_agentic_checkpoints_stub,
        "collect_artifacts": collect_artifacts or _scenario_collect_artifacts_stub,
    }


def _write_agentic_checkpoint_artifact(
    *,
    out_dir: Path,
    scenario_key: str,
    checkpoint_index: int,
    checkpoint_name: str,
    prompt: str,
    response: str,
    parsed: dict[str, list[str]],
) -> Path:
    out_dir.mkdir(parents=True, exist_ok=True)
    path = out_dir / f"{scenario_key}-agentic-checkpoint-{checkpoint_index:02d}.md"
    body = "\n".join(
        [
            f"# Agentic Checkpoint: {checkpoint_name}",
            "",
            "## Prompt",
            prompt,
            "",
            "## Response",
            response,
            "",
            "## Parsed",
            f"Observed: {', '.join(parsed.get('observed', []))}",
            f"Not Observed: {', '.join(parsed.get('not_observed', []))}",
            f"Uncertain: {', '.join(parsed.get('uncertain', []))}",
            "",
        ]
    )
    path.write_text(body, encoding="utf-8")
    return path


def evaluate_agentic_checkpoints(
    *,
    deterministic: dict[str, object],
    checkpoints: list[dict[str, str]],
    out_dir: Path,
    scenario_key: str,
) -> dict[str, object]:
    parsed_outputs: list[dict[str, object]] = []
    for index, checkpoint in enumerate(checkpoints, start=1):
        name = str(checkpoint.get("name", f"checkpoint-{index}"))
        prompt = str(checkpoint.get("prompt", ""))
        response = str(checkpoint.get("response", ""))
        parsed = parse_agentic_footer(response)
        _write_agentic_checkpoint_artifact(
            out_dir=out_dir,
            scenario_key=scenario_key,
            checkpoint_index=index,
            checkpoint_name=name,
            prompt=prompt,
            response=response,
            parsed=parsed,
        )
        classify_agentic_mismatch(deterministic=deterministic, agentic=parsed)
        parsed_outputs.append(
            {
                "name": name,
                "parsed": parsed,
            }
        )

    return {
        "status": "ok",
        "scenario_key": scenario_key,
        "checkpoints": parsed_outputs,
    }


def emit_scenario_artifact_bundle(
    *,
    base_out_dir: Path,
    scenario_key: str,
    video_path: Path,
    screenshot_path: Path,
    console_lines: list[str],
    network_events: list[dict[str, object]],
    debug_stream: dict[str, object],
    control_responses: list[dict[str, object]],
    deterministic_result: dict[str, object],
    agentic_result: dict[str, object],
    agentic_artifacts: list[Path],
) -> dict[str, object]:
    if not video_path.exists() or video_path.stat().st_size == 0:
        raise AssertionError({"error": "missing_video_artifact", "path": str(video_path)})
    if not screenshot_path.exists() or screenshot_path.stat().st_size == 0:
        raise AssertionError({"error": "missing_screenshot_artifact", "path": str(screenshot_path)})
    if not agentic_artifacts:
        raise AssertionError({"error": "missing_agentic_artifacts"})
    for artifact in agentic_artifacts:
        if not artifact.exists():
            raise AssertionError({"error": "agentic_artifact_not_found", "path": str(artifact)})

    scenario_dir = base_out_dir / scenario_key
    scenario_dir.mkdir(parents=True, exist_ok=True)

    console_file = scenario_dir / "console.log"
    network_file = scenario_dir / "network.json"
    stream_file = scenario_dir / "debug-stream.json"
    control_file = scenario_dir / "control-responses.json"
    verdict_file = scenario_dir / "verdict.json"

    console_file.write_text("\n".join(console_lines) + ("\n" if console_lines else ""), encoding="utf-8")
    network_file.write_text(json.dumps(network_events, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    stream_file.write_text(json.dumps(debug_stream, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    control_file.write_text(json.dumps(control_responses, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    deterministic_ok = str(deterministic_result.get("status", "")).lower() == "ok"
    agentic_ok = str(agentic_result.get("status", "")).lower() == "ok"
    verdict = {
        "scenario_key": scenario_key,
        "status": "pass" if deterministic_ok and agentic_ok else "fail",
        "deterministic": deterministic_result,
        "agentic": agentic_result,
        "artifacts": {
            "video": str(video_path),
            "screenshot": str(screenshot_path),
            "console": str(console_file),
            "network": str(network_file),
            "debug_stream": str(stream_file),
            "control_responses": str(control_file),
            "agentic": [str(path) for path in agentic_artifacts],
        },
    }
    verdict_file.write_text(json.dumps(verdict, indent=2, sort_keys=True) + "\n", encoding="utf-8")

    return {
        "scenario_dir": str(scenario_dir),
        "verdict": verdict,
        "files": {
            "console": str(console_file),
            "network": str(network_file),
            "debug_stream": str(stream_file),
            "control_responses": str(control_file),
            "verdict": str(verdict_file),
        },
    }


def run_scenario_validation(
    *,
    scenario_key: str,
    context: dict[str, object],
    out_dir: Path,
) -> dict[str, object]:
    scenario = STAGEHAND_SCENARIOS.get(scenario_key)
    if not isinstance(scenario, dict):
        raise AssertionError({"error": "unknown_scenario", "scenario_key": scenario_key})

    assert_contract = scenario.get("assert_contract")
    if not callable(assert_contract):
        raise AssertionError({"error": "scenario_missing_assert_contract", "scenario_key": scenario_key})

    deterministic_result = assert_contract(context)
    if not isinstance(deterministic_result, dict):
        deterministic_result = {"status": "ok", "result": deterministic_result}

    checkpoints_raw = context.get("agentic_checkpoints")
    checkpoints = checkpoints_raw if isinstance(checkpoints_raw, list) else []
    agentic_result = evaluate_agentic_checkpoints(
        deterministic=deterministic_result,
        checkpoints=checkpoints,
        out_dir=out_dir,
        scenario_key=scenario_key,
    )

    agentic_files = sorted(out_dir.glob(f"{scenario_key}-agentic-checkpoint-*.md"))

    video_path = context.get("video_path")
    screenshot_path = context.get("screenshot_path")
    if not isinstance(video_path, Path):
        raise AssertionError({"error": "missing_video_path", "scenario_key": scenario_key})
    if not isinstance(screenshot_path, Path):
        raise AssertionError({"error": "missing_screenshot_path", "scenario_key": scenario_key})

    console_lines = context.get("console_lines")
    network_events = context.get("network_events")
    debug_stream = context.get("debug_stream")
    control_responses = context.get("control_responses")

    bundle = emit_scenario_artifact_bundle(
        base_out_dir=out_dir,
        scenario_key=scenario_key,
        video_path=video_path,
        screenshot_path=screenshot_path,
        console_lines=console_lines if isinstance(console_lines, list) else [],
        network_events=network_events if isinstance(network_events, list) else [],
        debug_stream=debug_stream if isinstance(debug_stream, dict) else {},
        control_responses=control_responses if isinstance(control_responses, list) else [],
        deterministic_result=deterministic_result,
        agentic_result=agentic_result,
        agentic_artifacts=agentic_files,
    )

    verdict_obj = bundle.get("verdict") if isinstance(bundle, dict) else None
    verdict_status = "fail"
    if isinstance(verdict_obj, dict):
        verdict_status = str(verdict_obj.get("status", "fail"))

    return {
        "scenario_key": scenario_key,
        "status": verdict_status,
        "deterministic": deterministic_result,
        "agentic": agentic_result,
        "bundle": bundle,
    }


def execute_debug_validation_scenario(
    *,
    scenario_key: str,
    page: Page,
    api_url: str,
    out_dir: Path,
    screenshot_path: Path,
    video_path: Path,
    console_lines: list[str],
    network_events: list[dict[str, object]],
) -> dict[str, object]:
    context = build_debug_validation_scenario_context(
        scenario_key=scenario_key,
        page=page,
        api_url=api_url,
        console_lines=console_lines,
        network_events=network_events,
    )
    context["video_path"] = video_path
    context["screenshot_path"] = screenshot_path
    return run_scenario_validation(scenario_key=scenario_key, context=context, out_dir=out_dir)


def build_debug_validation_scenario_context(
    *,
    scenario_key: str,
    page: Page,
    api_url: str,
    console_lines: list[str],
    network_events: list[dict[str, object]],
) -> dict[str, object]:
    scenario = STAGEHAND_SCENARIOS.get(scenario_key)
    if not isinstance(scenario, dict):
        raise AssertionError({"error": "unknown_scenario", "scenario_key": scenario_key})

    seed_fn = scenario.get("seed")
    act_fn = scenario.get("act")
    agentic_fn = scenario.get("agentic_checkpoints")
    if not callable(seed_fn):
        raise AssertionError({"error": "scenario_missing_seed", "scenario_key": scenario_key})
    if not callable(act_fn):
        raise AssertionError({"error": "scenario_missing_act", "scenario_key": scenario_key})
    if not callable(agentic_fn):
        raise AssertionError({"error": "scenario_missing_agentic_checkpoints", "scenario_key": scenario_key})

    seed_result = seed_fn(page, api_url)
    if not isinstance(seed_result, dict):
        raise AssertionError({"error": "invalid_seed_result", "scenario_key": scenario_key})
    act_result = act_fn(page, api_url, seed_result)
    if not isinstance(act_result, dict):
        raise AssertionError({"error": "invalid_act_result", "scenario_key": scenario_key})

    checkpoints = agentic_fn(act_result)
    if not isinstance(checkpoints, list) or not checkpoints:
        raise AssertionError({"error": "missing_agentic_checkpoints", "scenario_key": scenario_key})

    return {
        **act_result,
        "seed_result": seed_result,
        "agentic_checkpoints": checkpoints,
        "console_lines": console_lines,
        "network_events": network_events,
    }


def maybe_run_debug_validation_scenario(
    *,
    scenario_key: str,
    page: Page,
    api_url: str,
    out_dir: Path,
    screenshot_path: Path,
    video_path: Path,
    console_lines: list[str],
    network_events: list[dict[str, object]],
) -> dict[str, object] | None:
    normalized_key = scenario_key.strip().lower()
    if not normalized_key:
        return None
    return execute_debug_validation_scenario(
        scenario_key=normalized_key,
        page=page,
        api_url=api_url,
        out_dir=out_dir,
        screenshot_path=screenshot_path,
        video_path=video_path,
        console_lines=console_lines,
        network_events=network_events,
    )


def _scenario_seed_embedding_model_identity(page: Page, api_url: str) -> dict[str, object]:
    response = page.request.post(
        f"{api_url}/benchmarks/test/seed-scenario",
        data=json.dumps({"scenario_key": "core_stream_baseline", "overrides": {"current_step_name": "prepare_case"}}),
        headers={"Content-Type": "application/json"},
        timeout=60_000,
    )
    if not response.ok:
        raise AssertionError({"error": "embedding_identity_seed_failed", "status": response.status})
    raw_payload = response.json()
    payload: dict[str, object] = raw_payload if isinstance(raw_payload, dict) else {}
    run_raw = payload.get("run")
    run_info: dict[str, object] = run_raw if isinstance(run_raw, dict) else {}
    run_id = str(run_info.get("run_id", "")).strip()
    pipeline_id = str(run_info.get("pipeline_id", "compression-rerender/v1")).strip() or "compression-rerender/v1"
    pipeline_run_id = str(run_info.get("pipeline_run_id", "")).strip()
    if not run_id:
        raise AssertionError({"error": "embedding_identity_seed_missing_run_id", "payload": payload})
    if not pipeline_run_id:
        raise AssertionError({"error": "embedding_identity_seed_missing_pipeline_run_id", "payload": payload})
    return {"run_id": run_id, "pipeline_id": pipeline_id, "pipeline_run_id": pipeline_run_id}


def _scenario_act_embedding_model_identity(page: Page, api_url: str, seed_result: dict[str, object]) -> dict[str, object]:
    run_id = str(seed_result.get("run_id", "")).strip()
    pipeline_id = str(seed_result.get("pipeline_id", "compression-rerender/v1"))
    pipeline_run_id = str(seed_result.get("pipeline_run_id", run_id))
    if not run_id:
        raise AssertionError({"error": "embedding_identity_missing_run_id", "seed_result": seed_result})
    if not pipeline_run_id:
        raise AssertionError({"error": "embedding_identity_act_missing_pipeline_run_id", "seed_result": seed_result})

    # Step 1: select the seeded run in the UI so the debug stream becomes visible.
    # The RunTable renders each run as a button whose first <span> contains run_id.
    # The UI fetches runs once on mount; reload so it picks up the newly seeded run.
    page.reload(wait_until="networkidle")
    page.get_by_role("button", name="Runs", exact=True).wait_for(state="visible", timeout=30_000)
    page.get_by_role("button", name="Runs", exact=True).click()
    page.get_by_test_id("run-debug-actions").wait_for(state="visible", timeout=30_000)

    run_row = page.locator(".run-row").filter(has=page.locator("span", has_text=run_id)).first
    for _ in range(20):
        if run_row.is_visible():
            break
        time.sleep(1.0)
    else:
        raise AssertionError({"error": "embedding_identity_run_row_not_visible", "run_id": run_id})
    run_row.click()

    # Wait for the debug stream panel to load at least one step (prepare_case was seeded).
    debug_step_list = page.get_by_test_id("debug-step-list")
    debug_step_list.wait_for(state="visible", timeout=20_000)

    # Step 2: Advance 5 steps via API and wait for the UI to reflect each one.
    # Steps: decompose, embed_decomposed, lift, embed_lifted, candidate_search
    # (prepare_case was recorded during seeding)
    STEP_NAMES = ["map", "embed_mapped", "lift", "embed_lifted", "candidate_search"]
    for i, _step_name in enumerate(STEP_NAMES):
        step_count_before = debug_step_list.locator(".debug-step-item").count()

        ctrl_response = page.request.post(
            f"{api_url}/benchmarks/runs/{run_id}/control",
            data=json.dumps({
                "command_id": f"cmd-embed-identity-{i}-{run_id[:8]}",
                "action": "next_step",
                "pipeline_id": pipeline_id,
                "pipeline_run_id": pipeline_run_id,
            }),
            headers={"Content-Type": "application/json"},
            timeout=60_000,
        )
        if not ctrl_response.ok:
            raise AssertionError({"error": f"embedding_identity_control_step_{i}_failed", "status": ctrl_response.status})

        # Wait for a new step item to appear in the debug stream (the UI polls the stream).
        for _ in range(15):
            current_count = debug_step_list.locator(".debug-step-item").count()
            if current_count > step_count_before:
                break
            time.sleep(1.0)
        # Non-fatal if count didn't increase — the UI may batch updates; continue.

    # Step 3: Click on the candidate_search step item to show its detail panel.
    candidate_search_item = (
        debug_step_list.locator(".debug-step-item")
        .filter(has=page.locator(".debug-step-title", has_text="candidate_search"))
        .first
    )
    if candidate_search_item.is_visible():
        candidate_search_item.click()
        # Wait for the detail panel to populate.
        page.get_by_test_id("debug-step-detail").wait_for(state="visible", timeout=10_000)
        time.sleep(1.5)  # pause so the video captures the detail view

    # Step 4: Fetch embedding info for contract assertion.
    info_response = page.request.get(
        f"{api_url}/benchmarks/runs/{run_id}/embedding-info",
        timeout=60_000,
    )
    if not info_response.ok:
        raise AssertionError({"error": "embedding_identity_info_fetch_failed", "status": info_response.status})

    embedding_info = info_response.json()
    return {
        "run_id": run_id,
        "embedding_info": embedding_info if isinstance(embedding_info, dict) else {},
    }


def _scenario_agentic_checkpoints_embedding_model_identity(context: dict[str, object]) -> list[dict[str, str]]:
    embedding_info = context.get("embedding_info")
    info: dict[str, object] = embedding_info if isinstance(embedding_info, dict) else {}
    model = info.get("embedding_model", "unknown")
    dims = info.get("vector_dims", 0)
    count = info.get("fragment_count", 0)
    status = info.get("status", "unknown")

    observed: list[str] = []
    not_observed: list[str] = []

    if status == "ok":
        observed.append("embedding-info status ok")
    else:
        not_observed.append(f"embedding-info status ok (got: {status})")

    if isinstance(model, str) and model and model != "unknown":
        observed.append(f"model name present ({model})")
    else:
        not_observed.append("model name present")

    if isinstance(dims, int) and dims > 0:
        observed.append(f"vector dims nonzero ({dims})")
    else:
        not_observed.append("vector dims nonzero")

    if isinstance(count, int) and count > 0:
        observed.append(f"fragment count nonzero ({count})")
    else:
        not_observed.append("fragment count nonzero")

    response = "\n".join([
        f"Observed: {', '.join(observed) if observed else 'none'}",
        f"Not Observed: {', '.join(not_observed) if not_observed else 'none'}",
        "Uncertain: none",
    ])

    return [
        {
            "name": "embedding_model_identity_check",
            "prompt": "Verify embedding model identity and vector dimensions are recorded in the DB after candidate_search.",
            "response": response,
        }
    ]


def _assert_embedding_model_identity_contract(context: dict[str, object]) -> dict[str, object]:
    embedding_info = context.get("embedding_info")
    if not isinstance(embedding_info, dict):
        raise AssertionError({"error": "missing_embedding_info"})
    status = embedding_info.get("status")
    if status == "db_unavailable":
        raise AssertionError({"error": "db_unavailable_cannot_verify_model_identity"})
    if status == "no_runtime_context":
        raise AssertionError({"error": "no_runtime_context_after_5_steps"})
    if status == "db_error":
        raise AssertionError({"error": "db_error", "detail": embedding_info.get("detail")})
    embedder = get_shared_embedder()
    expected_model = embedder.model_name
    expected_dims = embedder.dimensions
    actual_model = embedding_info.get("embedding_model")
    actual_dims = embedding_info.get("vector_dims")
    fragment_count = embedding_info.get("fragment_count", 0)
    if actual_model != expected_model:
        raise AssertionError({
            "error": "embedding_model_mismatch",
            "expected": expected_model,
            "actual": actual_model,
        })
    if actual_dims != expected_dims:
        raise AssertionError({
            "error": "vector_dims_mismatch",
            "expected": expected_dims,
            "actual": actual_dims,
        })
    if not isinstance(fragment_count, int) or fragment_count <= 0:
        raise AssertionError({
            "error": "expected_nonzero_fragment_count",
            "fragment_count": fragment_count,
        })
    return {
        "status": "ok",
        "embedding_model": actual_model,
        "vector_dims": actual_dims,
        "fragment_count": fragment_count,
    }


STAGEHAND_SCENARIOS: dict[str, dict[str, object]] = {
    "core_run_stream": _build_scenario_definition_with_assert(
        _assert_core_run_stream_contract,
        seed=_scenario_seed_core_run_stream,
        act=_scenario_act_core_run_stream,
        agentic_checkpoints=_scenario_agentic_checkpoints_core_run_stream,
    ),
    "controls_modes": _build_scenario_definition_with_assert(
        _assert_controls_modes_contract,
        seed=_scenario_seed_controls_modes,
        act=_scenario_act_controls_modes,
        agentic_checkpoints=_scenario_agentic_checkpoints_controls_modes,
    ),
    "deterministic_retry": _build_scenario_definition_with_assert(_assert_deterministic_retry_contract),
    "env_scoped_drillthrough": _build_scenario_definition_with_assert(_assert_env_scoped_drillthrough_contract),
    "evaluation_alignment": _build_scenario_definition_with_assert(_assert_evaluation_alignment_contract),
    "cross_env_isolation": _build_scenario_definition_with_assert(_assert_cross_env_isolation_contract),
    "embedding_model_identity": _build_scenario_definition_with_assert(
        _assert_embedding_model_identity_contract,
        seed=_scenario_seed_embedding_model_identity,
        act=_scenario_act_embedding_model_identity,
        agentic_checkpoints=_scenario_agentic_checkpoints_embedding_model_identity,
    ),
}


def main() -> None:
    validate_stagehand_runtime((sys.version_info.major, sys.version_info.minor, sys.version_info.micro), sys.platform)

    base_url = os.getenv("IKAM_GRAPH_VIEWER_URL", "http://localhost:5179")
    api_url = os.getenv("IKAM_PERF_API_URL", get_default_perf_api_url())
    case_id = os.getenv("IKAM_STAGEHAND_CASE", "l-construction-v01")
    mode = os.getenv("IKAM_STAGEHAND_MODE", "full").strip().lower()
    require_evaluation = os.getenv("IKAM_STAGEHAND_REQUIRE_EVALUATION", "true").strip().lower() != "false"
    evaluation_case_id = resolve_evaluation_case_id(
        active_case_id=case_id,
        explicit_case_id=os.getenv("IKAM_STAGEHAND_EVALUATION_CASE", ""),
    )
    search_candidates = build_search_query_candidates(
        primary_query=os.getenv("IKAM_STAGEHAND_SEARCH_QUERY", "revenue"),
        fallback_queries_csv=os.getenv("IKAM_STAGEHAND_SEARCH_FALLBACKS", "margin,forecast,growth,cost"),
    )
    out_dir = Path(os.getenv("IKAM_STAGEHAND_OUT", "/tmp/ikam-perf-stagehand"))
    scenario_key = os.getenv("IKAM_STAGEHAND_SCENARIO", "")
    out_dir.mkdir(parents=True, exist_ok=True)

    if shutil.which("ffmpeg") is None:
        # Recording fallback can use ffmpeg if Playwright video is unavailable.
        # Not fatal because Playwright-native recording is enabled below.
        pass

    run_slug = _now_slug()
    screenshot_path = out_dir / f"ui-{run_slug}.png"
    outputs_path = out_dir / f"outputs-{run_slug}.json"
    console_path = out_dir / f"console-{run_slug}.log"

    console_lines: list[str] = []
    request_failures: list[str] = []
    responses: list[str] = []
    legend_visible = False
    inspector_visible = False
    semantic_explorer_visible = False
    evidence_view_verified = False
    evaluation_report_verified = False
    visual_pass_verified = False
    enrichment_panel_verified = False
    stage_panel_verified = False
    enrichment_runs_count = 0
    enrichment_items_count = 0
    stage_queue_before = 0
    stage_queue_after = 0
    commit_receipts_count = 0
    post_commit_nodes = 0
    post_commit_edges = 0
    wiki_section_count = 0

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        context = browser.new_context(
            viewport={"width": 1600, "height": 950},
            record_video_dir=str(out_dir),
            record_video_size={"width": 1600, "height": 950},
        )
        page = context.new_page()

        def on_console(msg: ConsoleMessage) -> None:
            console_lines.append(f"{msg.type}: {msg.text}")

        def on_request_failed(req: Request) -> None:
            request_failures.append(
                json.dumps(
                    {
                        "url": req.url,
                        "method": req.method,
                        "resource_type": req.resource_type,
                        "failure": req.failure,
                    },
                    sort_keys=True,
                )
            )

        def on_response(resp: Response) -> None:
            if "/benchmarks/" in resp.url or "/graph/" in resp.url:
                responses.append(json.dumps({"url": resp.url, "status": resp.status}, sort_keys=True))

        page.on("console", on_console)
        page.on("requestfailed", on_request_failed)
        page.on("response", on_response)

        page.goto(base_url, wait_until="networkidle")

        if scenario_key.strip():
            page.get_by_role("button", name="Runs", exact=True).wait_for(state="visible", timeout=30_000)
            page.get_by_role("button", name="Runs", exact=True).click()
            page.get_by_test_id("run-debug-actions").wait_for(state="visible", timeout=30_000)
            page.screenshot(path=str(screenshot_path), full_page=True)
            scenario_context = build_debug_validation_scenario_context(
                scenario_key=scenario_key,
                page=page,
                api_url=api_url,
                console_lines=console_lines,
                network_events=[],
            )
            video = page.video
            context.close()
            browser.close()

            video_path = Path(video.path()) if video else None
            if not video_path or not video_path.exists() or video_path.stat().st_size == 0:
                raise AssertionError({"error": "missing_video_artifact", "video": str(video_path) if video_path else None})

            scenario_context["video_path"] = video_path
            scenario_context["screenshot_path"] = screenshot_path
            scenario_result = run_scenario_validation(
                scenario_key=scenario_key.strip().lower(),
                context=scenario_context,
                out_dir=out_dir,
            )
            print(json.dumps({"scenario": scenario_result}, indent=2, default=str))
            return

        _assert_heading(page, "IKAM Performance Report")
        page.get_by_role("button", name="Runs", exact=True).wait_for(state="visible", timeout=30_000)
        page.get_by_role("button", name="Graph", exact=True).wait_for(state="visible", timeout=30_000)
        page.get_by_role("button", name="Merge", exact=True).wait_for(state="visible", timeout=30_000)

        page.get_by_role("button", name="Runs", exact=True).click()
        case_checkbox = (
            page.locator(".case-option")
            .filter(has=page.locator("span", has_text=re.compile(rf"^{re.escape(case_id)}$")))
            .locator('input[type="checkbox"]')
            .first
        )
        case_checkbox.wait_for(state="visible", timeout=30_000)
        case_checkbox.click()

        # Keep reset enabled for deterministic baseline run.
        run_button = page.get_by_role("button", name="Run Cases", exact=True)
        run_button.click()

        runs_data = None
        active_run = None
        for _ in range(45):
            runs_data = page.evaluate(
                """
                async (apiUrl) => {
                    const res = await fetch(`${apiUrl}/benchmarks/runs`);
                    return res.json();
                }
                """,
                api_url,
            )
            if isinstance(runs_data, list):
                active_run = next(
                    (run for run in runs_data if str(run.get("case_id", "")).lower() == str(case_id).lower()),
                    None,
                )
                if active_run:
                    break
            time.sleep(2.0)
        if not active_run:
            raise AssertionError({"error": "run_not_found", "case_id": case_id})

        run_aqs = (active_run.get("answer_quality") or {}).get("aqs")
        if run_aqs is None:
            raise AssertionError({"error": "missing_answer_quality", "run_id": active_run.get("run_id")})

        evaluation_resp = page.request.post(
            f"{api_url}/evaluations/run",
            params={"case_id": evaluation_case_id},
            timeout=180_000,
        )
        if not evaluation_resp.ok:
            if require_evaluation:
                raise AssertionError(
                    {
                        "error": "evaluation_endpoint_failed",
                        "status": evaluation_resp.status,
                        "case_id": evaluation_case_id,
                    }
                )
        else:
            evaluation_payload = evaluation_resp.json() if isinstance(evaluation_resp.json(), dict) else {}
            validate_evaluation_report(evaluation_payload)
            evaluation_report_verified = True

        page.get_by_role("button", name="Graph", exact=True).click()
        graph_map = page.get_by_test_id("graph-map")
        graph_map.wait_for(state="visible", timeout=30_000)
        graph_canvas = page.locator('[data-testid="graph-map"] canvas').first
        graph_canvas.wait_for(state="attached", timeout=30_000)

        # Evidence view: execute semantic query and verify marker + inspector states.
        marker = page.get_by_test_id("graph-marker").first
        search_input = page.get_by_placeholder("Search semantic group")
        marker_visible = False
        for query in search_candidates:
            search_input.click()
            search_input.fill(query)
            search_input.press("Enter")
            try:
                marker.wait_for(state="visible", timeout=8_000)
                marker_visible = True
                break
            except Exception:
                continue
        if not marker_visible:
            raise AssertionError({"error": "graph_marker_not_found", "queries": search_candidates})
        marker.click()
        page.get_by_test_id("graph-inspector").wait_for(state="visible", timeout=30_000)
        evidence_view_verified = True
        legend_visible = page.get_by_test_id("graph-legend").count() > 0
        inspector_visible = page.get_by_test_id("graph-inspector").count() > 0
        semantic_explorer_visible = page.get_by_test_id("semantic-explorer").count() > 0
        # Graph viewer is now minimalist; legend/inspector are conditional.

        active_run_id = str(active_run.get("run_id", ""))
        active_graph_id = str(active_run.get("graph_id") or active_run.get("project_id") or "")
        nodes_metric = 0
        edges_metric = 0

        nodes_data, edges_data = _poll_graph_data(page, api_url, active_graph_id)
        nodes_metric = len(nodes_data)
        edges_metric = len(edges_data)

        if not nodes_data:
            raise AssertionError({"error": "nodes_empty", "graph_id": active_graph_id})
        if not edges_data:
            raise AssertionError({"error": "edges_empty", "graph_id": active_graph_id})

        if not active_graph_id:
            raise AssertionError({"error": "missing_graph_id", "run_id": active_run_id})

        # Enrichment workflow verification: Review/Enrichment/Stage tabs.
        review_panel = page.get_by_test_id("review-panel")
        review_panel.wait_for(state="visible", timeout=30_000)

        page.get_by_role("tab", name="Enrichment", exact=True).click()
        page.get_by_test_id("enrichment-tab").wait_for(state="visible", timeout=30_000)
        enrichment_panel_verified = True

        runs_resp = page.request.get(f"{api_url}/graph/enrichment/runs", params={"graph_id": active_graph_id}, timeout=60_000)
        items_resp = page.request.get(f"{api_url}/graph/enrichment/staged", params={"graph_id": active_graph_id}, timeout=60_000)
        if not runs_resp.ok or not items_resp.ok:
            raise AssertionError(
                {
                    "error": "enrichment_endpoints_unavailable",
                    "runs_status": runs_resp.status,
                    "items_status": items_resp.status,
                }
            )
        runs_payload = runs_resp.json() if isinstance(runs_resp.json(), dict) else {}
        items_payload = items_resp.json() if isinstance(items_resp.json(), dict) else {}
        enrichment_runs_count = len(runs_payload.get("runs", []) if isinstance(runs_payload.get("runs"), list) else [])
        enrichment_items_count = len(items_payload.get("items", []) if isinstance(items_payload.get("items"), list) else [])
        if enrichment_runs_count <= 0 or enrichment_items_count <= 0:
            raise AssertionError(
                {
                    "error": "missing_enrichment_rows",
                    "runs": enrichment_runs_count,
                    "items": enrichment_items_count,
                }
            )

        approve_button = page.get_by_test_id("enrichment-tab").get_by_role("button", name="Approve").first
        approve_button.wait_for(state="visible", timeout=30_000)
        approve_button.click()

        page.get_by_role("tab", name="Stage", exact=True).click()
        page.get_by_test_id("stage-tab").wait_for(state="visible", timeout=30_000)
        stage_panel_verified = True

        queued_resp = page.request.get(f"{api_url}/graph/enrichment/staged", params={"graph_id": active_graph_id}, timeout=60_000)
        queued_payload = queued_resp.json() if queued_resp.ok else {}
        queued_items = queued_payload.get("items", []) if isinstance(queued_payload, dict) else []
        stage_queue_before = len([item for item in queued_items if isinstance(item, dict) and item.get("status") == "queued"])
        if stage_queue_before <= 0:
            raise AssertionError({"error": "queue_not_populated_after_approve", "queued": stage_queue_before})

        commit_button = page.get_by_role("button", name=re.compile(r"^Commit Queue"))
        commit_button.wait_for(state="visible", timeout=30_000)
        commit_button.click()

        receipt_payload = None
        for _ in range(15):
            receipts_resp = page.request.get(
                f"{api_url}/graph/enrichment/receipts",
                params={"graph_id": active_graph_id},
                timeout=60_000,
            )
            staged_resp = page.request.get(
                f"{api_url}/graph/enrichment/staged",
                params={"graph_id": active_graph_id},
                timeout=60_000,
            )
            if receipts_resp.ok and staged_resp.ok:
                maybe_receipts = receipts_resp.json()
                maybe_staged = staged_resp.json()
                receipts = maybe_receipts.get("receipts", []) if isinstance(maybe_receipts, dict) else []
                staged_items = maybe_staged.get("items", []) if isinstance(maybe_staged, dict) else []
                commit_receipts_count = len(receipts) if isinstance(receipts, list) else 0
                stage_queue_after = len(
                    [item for item in staged_items if isinstance(item, dict) and item.get("status") == "queued"]
                ) if isinstance(staged_items, list) else 0
                if commit_receipts_count > 0 and stage_queue_after == 0:
                    receipt_payload = receipts[-1] if receipts else None
                    break
            time.sleep(1.0)

        if commit_receipts_count <= 0:
            raise AssertionError({"error": "missing_commit_receipt", "graph_id": active_graph_id})
        if stage_queue_after != 0:
            raise AssertionError({"error": "queue_not_drained_after_commit", "queued": stage_queue_after})

        post_nodes_data, post_edges_data = _poll_graph_data(page, api_url, active_graph_id)
        post_commit_nodes = len(post_nodes_data)
        post_commit_edges = len(post_edges_data)

        time.sleep(2.0)
        rect_samples = _rect_samples(page, "graph-map", count=3 if mode == "smoke" else 6)
        _assert_rect_stable(rect_samples)
        if rect_samples and (rect_samples[0]["width"] < 300 or rect_samples[0]["height"] < 180):
            raise AssertionError({"error": "graph_viewport_too_small", "rect": rect_samples[0]})

        camera_distance_text = graph_map.get_attribute("data-camera-distance") or ""
        try:
            camera_distance = float(camera_distance_text)
        except ValueError:
            camera_distance = 0.0
        if not (camera_distance > 0 and camera_distance < 1_000_000):
            raise AssertionError(
                {
                    "error": "invalid_camera_distance",
                    "camera_distance": camera_distance_text,
                }
            )

        page.screenshot(path=str(screenshot_path), full_page=True)
        viewport_box = graph_canvas.bounding_box() or graph_map.bounding_box() or {"x": 0, "y": 0, "width": 0, "height": 0}
        render_stats = assert_graph_crop_non_uniform(
            screenshot_path,
            int(viewport_box["x"]),
            int(viewport_box["y"]),
            int(viewport_box["width"]),
            int(viewport_box["height"]),
        )
        validate_visual_pass_signals(
            render_stats=render_stats,
            nodes_count=nodes_metric,
            edges_count=edges_metric,
        )
        visual_pass_verified = True

        # Wiki tab verification: generate doc with parser-defined sections + IKAM Breakdown contract.
        page.get_by_role("button", name="Wiki", exact=True).click()
        page.get_by_test_id("wiki-workspace").wait_for(state="visible", timeout=30_000)
        page.get_by_role("button", name="Generate Wiki", exact=True).click()
        page.get_by_test_id("ikam-breakdown").wait_for(state="visible", timeout=90_000)
        page.wait_for_selector(".panel-placeholder:has-text('Generating wiki...')", state="hidden", timeout=90_000)

        wiki_resp = page.request.get(f"{api_url}/graph/wiki", params={"graph_id": active_graph_id}, timeout=60_000)
        if not wiki_resp.ok:
            raise AssertionError({"error": "wiki_fetch_failed", "status": wiki_resp.status})
        wiki_payload = wiki_resp.json() if isinstance(wiki_resp.json(), dict) else {}
        wiki_validation = validate_wiki_document(wiki_payload)
        wiki_section_count = int(wiki_validation.get("section_count", 0))

        ikam_breakdown_title = page.get_by_test_id("ikam-breakdown").locator("h3").inner_text().strip()
        if ikam_breakdown_title.strip().lower() != "ikam breakdown":
            raise AssertionError({"error": "missing_ikam_breakdown", "title": ikam_breakdown_title})

        cards_count = page.locator(".wiki-card").count()
        expected_cards = wiki_section_count + 1
        if cards_count < expected_cards:
            raise AssertionError(
                {
                    "error": "wiki_cards_incomplete",
                    "cards_count": cards_count,
                    "expected_min": expected_cards,
                }
            )

        # Ensure provenance metadata rendered for generated sections.
        page.wait_for_function(
            """
            () => {
                const rows = Array.from(document.querySelectorAll('.wiki-card .decision-meta'));
                return rows.some((r) => {
                    const t = (r.textContent || '').toLowerCase();
                    return t.includes('model:') && t.includes('harness:');
                });
            }
            """,
            timeout=30_000,
        )

        video = page.video
        context.close()
        browser.close()

        video_path = Path(video.path()) if video else None
        if not video_path or not video_path.exists() or video_path.stat().st_size == 0:
            raise AssertionError({"error": "missing_video_artifact", "video": str(video_path) if video_path else None})

    console_path.write_text("\n".join(console_lines) + "\n")

    outputs = {
        "viewer_url": base_url,
        "api_url": api_url,
        "case_id": case_id,
        "mode": mode,
        "require_evaluation": require_evaluation,
        "evaluation_case_id": evaluation_case_id,
        "search_candidates": search_candidates,
        "run_id": active_run_id,
        "graph_id": active_graph_id,
        "metrics": {
            "nodes": nodes_metric,
            "edges": edges_metric,
            "api_nodes": len(nodes_data),
            "api_edges": len(edges_data),
            "post_commit_nodes": post_commit_nodes,
            "post_commit_edges": post_commit_edges,
            "post_commit_edge_delta": post_commit_edges - len(edges_data),
        },
        "viewport": {
            "rect_samples": rect_samples,
            "camera_distance": camera_distance,
        },
        "render": render_stats,
            "explainability": {
            "evaluation_report_verified": evaluation_report_verified,
            "legend_visible": legend_visible,
            "inspector_visible": inspector_visible,
            "semantic_explorer_visible": semantic_explorer_visible,
            "evidence_view_verified": evidence_view_verified,
            "visual_pass_verified": visual_pass_verified,
            "enrichment_panel_verified": enrichment_panel_verified,
            "stage_panel_verified": stage_panel_verified,
            "wiki_generated": True,
            "ikam_breakdown_title": ikam_breakdown_title,
            "wiki_section_count": wiki_section_count,
            },
        "enrichment": {
            "runs": enrichment_runs_count,
            "items": enrichment_items_count,
            "queued_before_commit": stage_queue_before,
            "queued_after_commit": stage_queue_after,
            "commit_receipts": commit_receipts_count,
        },
        "artifacts": {
            "screenshot": str(screenshot_path),
            "console": str(console_path),
            "video": str(video_path),
        },
        "request_failures": request_failures,
        "responses": responses,
        "status": "pass",
    }
    outputs_path.write_text(json.dumps(outputs, indent=2, sort_keys=True) + "\n")

    print(
        json.dumps(
            {
                "outputs": str(outputs_path),
                "screenshot": str(screenshot_path),
                "video": str(video_path),
                "console": str(console_path),
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
