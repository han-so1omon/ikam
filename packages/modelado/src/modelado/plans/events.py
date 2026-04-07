from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Dict, Iterable, List, Optional, Tuple

from ikam.graph import _cas_hex


PLAN_EVENT_SCHEMA_VERSION: str = "1.0.0"

PLAN_EVENT_KINDS: set[str] = {
    "plan_created",
    "plan_section_claimed",
    "plan_section_progress",
    "plan_section_blocked",
    "plan_section_completed",
    "plan_section_released",
    "plan_amendment_proposed",
    "plan_amendment_accepted",
    "plan_amendment_rejected",
}


def canonicalize_json(value: Any) -> bytes:
    """Deterministic JSON encoding (sorted keys, compact separators)."""
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def _normalize_payload(payload: Any) -> Optional[str]:
    if payload is None:
        return None
    if isinstance(payload, str):
        return payload
    return json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)


def plan_event_id(record: Dict[str, Any]) -> str:
    """Compute a deterministic event id from record content.

    We hash the canonical JSON representation of the event record **excluding**
    the `eventId` field to avoid self-reference.
    """
    base = dict(record)
    base.pop("eventId", None)
    return _cas_hex(canonicalize_json(base))


def build_plan_event(
    *,
    project_id: str,
    scope_id: str,
    plan_artifact_id: str,
    kind: str,
    ts: int,
    fragment_id: Optional[str] = None,
    actor_id: Optional[str] = None,
    lease_owner: Optional[str] = None,
    lease_token: Optional[str] = None,
    lease_expires_at: Optional[int] = None,
    message: Optional[str] = None,
    payload: Any = None,
    schema_version: str = PLAN_EVENT_SCHEMA_VERSION,
) -> Dict[str, Any]:
    """Build a PlanEvent record matching `schemas/plan-event.avsc`.

    `eventId` is content-addressed (deterministic) for replay safety.
    """
    if kind not in PLAN_EVENT_KINDS:
        raise ValueError(f"Unknown PlanEvent kind: {kind}")
    record: Dict[str, Any] = {
        "projectId": project_id,
        "scopeId": scope_id,
        "planArtifactId": plan_artifact_id,
        "fragmentId": fragment_id,
        "ts": int(ts),
        "kind": kind,
        "actorId": actor_id,
        "leaseOwner": lease_owner,
        "leaseToken": lease_token,
        "leaseExpiresAt": lease_expires_at,
        "message": message,
        "payload": _normalize_payload(payload),
        "schemaVersion": schema_version,
    }
    record["eventId"] = plan_event_id(record)
    return record


@dataclass(frozen=True)
class PlanLease:
    plan_artifact_id: str
    fragment_id: str
    owner: str
    token: str
    expires_at_ms: int
    claimed_at_ms: int
    event_id: str


@dataclass
class PlanLeaseState:
    leases: Dict[Tuple[str, str], PlanLease] = field(default_factory=dict)
    seen_event_ids: set[str] = field(default_factory=set)


def reduce_plan_events(
    events: Iterable[Dict[str, Any]],
    *,
    assume_sorted: bool = False,
) -> PlanLeaseState:
    """Reduce an event stream into current cooperative lease state.

    Replay safety rules:
    - Duplicate events (same eventId) are ignored.
    - Events are applied in a deterministic order (ts, eventId) unless `assume_sorted`.
    - A claim cannot override an unexpired prior lease at the time of the claim.
    """

    normalized: List[Dict[str, Any]] = [e for e in events if isinstance(e, dict)]
    if not assume_sorted:
        normalized.sort(key=lambda e: (int(e.get("ts") or 0), str(e.get("eventId") or "")))

    state = PlanLeaseState()

    for event in normalized:
        event_id = str(event.get("eventId") or "")
        if not event_id:
            continue
        if event_id in state.seen_event_ids:
            continue
        state.seen_event_ids.add(event_id)

        kind = str(event.get("kind") or "")
        plan_artifact_id = str(event.get("planArtifactId") or "")
        fragment_id = event.get("fragmentId")
        ts = int(event.get("ts") or 0)

        if not plan_artifact_id or not fragment_id:
            continue

        key = (plan_artifact_id, str(fragment_id))
        current = state.leases.get(key)

        if kind == "plan_section_claimed":
            owner = event.get("leaseOwner")
            token = event.get("leaseToken")
            expires_at = event.get("leaseExpiresAt")
            if not isinstance(owner, str) or not owner:
                continue
            if not isinstance(token, str) or not token:
                continue
            try:
                expires_at_ms = int(expires_at)
            except (TypeError, ValueError):
                continue

            if (
                current is not None
                and current.expires_at_ms > ts
                and current.claimed_at_ms < ts
                and current.token != token
            ):
                # Prevent lease stealing: ignore claims made while another lease is still valid.
                # Claims at the *same* timestamp are treated as a deterministic collision and
                # resolved by the tie-break below.
                continue

            proposed = PlanLease(
                plan_artifact_id=plan_artifact_id,
                fragment_id=str(fragment_id),
                owner=owner,
                token=token,
                expires_at_ms=expires_at_ms,
                claimed_at_ms=ts,
                event_id=event_id,
            )

            if current is None:
                state.leases[key] = proposed
                continue

            # Deterministic tie-break: last-wins by (claimed_at_ms, event_id)
            if (proposed.claimed_at_ms, proposed.event_id) >= (current.claimed_at_ms, current.event_id):
                state.leases[key] = proposed
            continue

        if kind in {"plan_section_released", "plan_section_completed"}:
            token = event.get("leaseToken")
            if current is None:
                continue
            if not isinstance(token, str) or not token:
                continue
            if token != current.token:
                continue
            if ts < current.claimed_at_ms:
                continue
            state.leases.pop(key, None)
            continue

        # progress/blocked: no lease state change

    return state


def get_effective_lease(
    state: PlanLeaseState,
    *,
    plan_artifact_id: str,
    fragment_id: str,
    as_of_ms: int,
) -> Optional[PlanLease]:
    lease = state.leases.get((plan_artifact_id, fragment_id))
    if lease is None:
        return None
    if lease.claimed_at_ms > int(as_of_ms):
        return None
    if lease.expires_at_ms <= int(as_of_ms):
        return None
    return lease


__all__ = [
    "PLAN_EVENT_SCHEMA_VERSION",
    "canonicalize_json",
    "plan_event_id",
    "build_plan_event",
    "PlanLease",
    "PlanLeaseState",
    "reduce_plan_events",
    "get_effective_lease",
]
