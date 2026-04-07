from __future__ import annotations

from typing import Any

from interacciones.schemas import TracePersistenceMode, TracePersistencePolicy


def should_promote_trace(policy: TracePersistencePolicy, *, event: dict[str, Any], is_final: bool) -> bool:
    mode = policy.mode
    event_type = str(event.get("event_type", ""))
    if mode == TracePersistenceMode.NONE:
        return False
    if mode == TracePersistenceMode.ON_FAILURE:
        return event_type == "execution.failed"
    if mode == TracePersistenceMode.ON_APPROVAL:
        return event_type.startswith("approval.")
    if mode == TracePersistenceMode.PER_STEP:
        return True
    if mode == TracePersistenceMode.FINAL_ONLY:
        return is_final
    if mode == TracePersistenceMode.BATCH:
        return False
    return False


def build_trace_promotion_plan(
    policy: TracePersistencePolicy,
    *,
    hot_events: list[dict[str, Any]],
    event: dict[str, Any],
    is_final: bool,
) -> dict[str, Any] | None:
    if not should_promote_trace(policy, event=event, is_final=is_final):
        return None
    return {
        "workflow_id": event.get("workflow_id"),
        "run_id": event.get("run_id"),
        "target_ref": event.get("ref"),
        "policy_mode": policy.mode.value,
        "trigger_event_type": event.get("event_type"),
        "trace_ids": [str(item["trace_id"]) for item in hot_events if "trace_id" in item],
        "event_count": len(hot_events),
    }
