from __future__ import annotations

from modelado.plans.events import (
    build_plan_event,
    get_effective_lease,
    plan_event_id,
    reduce_plan_events,
)


def test_plan_event_id_is_deterministic_for_same_record() -> None:
    base = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        kind="plan_created",
        ts=123,
        message="hello",
        payload={"a": 1, "b": [2, 3]},
    )
    again = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        kind="plan_created",
        ts=123,
        message="hello",
        payload={"b": [2, 3], "a": 1},
    )

    assert base["eventId"] == again["eventId"]
    assert plan_event_id(base) == base["eventId"]


def test_amendment_event_kinds_are_supported_and_deterministic() -> None:
    proposed = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_amendment_proposed",
        ts=123,
        message="propose",
        payload={"amendment_id": "a1", "ops": [{"op": "replace", "path": "/x", "value": 1}]},
    )
    proposed_reordered = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_amendment_proposed",
        ts=123,
        message="propose",
        payload={"ops": [{"path": "/x", "value": 1, "op": "replace"}], "amendment_id": "a1"},
    )

    assert proposed["eventId"] == proposed_reordered["eventId"]

    accepted = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_amendment_accepted",
        ts=124,
        payload={"amendment_id": "a1"},
    )
    assert accepted["kind"] == "plan_amendment_accepted"


def test_reduce_plan_events_is_idempotent_with_duplicates() -> None:
    ev = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_section_claimed",
        ts=1000,
        lease_owner="agent-a",
        lease_token="t1",
        lease_expires_at=2000,
    )

    state1 = reduce_plan_events([ev, ev])
    state2 = reduce_plan_events([ev, ev, ev])

    lease1 = get_effective_lease(state1, plan_artifact_id="plan-uuid", fragment_id="frag-1", as_of_ms=1500)
    lease2 = get_effective_lease(state2, plan_artifact_id="plan-uuid", fragment_id="frag-1", as_of_ms=1500)

    assert lease1 is not None
    assert lease2 is not None
    assert lease1 == lease2


def test_lease_cannot_be_stolen_before_expiry() -> None:
    a_claim = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_section_claimed",
        ts=1000,
        lease_owner="agent-a",
        lease_token="t1",
        lease_expires_at=2000,
    )
    b_claim_early = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_section_claimed",
        ts=1500,
        lease_owner="agent-b",
        lease_token="t2",
        lease_expires_at=2500,
    )
    b_claim_after = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_section_claimed",
        ts=2100,
        lease_owner="agent-b",
        lease_token="t2",
        lease_expires_at=2600,
    )

    state_mid = reduce_plan_events([b_claim_early, a_claim])
    lease_mid = get_effective_lease(state_mid, plan_artifact_id="plan-uuid", fragment_id="frag-1", as_of_ms=1600)
    assert lease_mid is not None
    assert lease_mid.owner == "agent-a"

    state_late = reduce_plan_events([b_claim_early, a_claim, b_claim_after])
    lease_late = get_effective_lease(state_late, plan_artifact_id="plan-uuid", fragment_id="frag-1", as_of_ms=2200)
    assert lease_late is not None
    assert lease_late.owner == "agent-b"


def test_release_requires_matching_token() -> None:
    claim = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_section_claimed",
        ts=1000,
        lease_owner="agent-a",
        lease_token="t1",
        lease_expires_at=2000,
    )
    bad_release = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_section_released",
        ts=1200,
        lease_token="wrong",
    )
    good_release = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_section_released",
        ts=1300,
        lease_token="t1",
    )

    state = reduce_plan_events([claim, bad_release, good_release])
    lease = get_effective_lease(state, plan_artifact_id="plan-uuid", fragment_id="frag-1", as_of_ms=1500)
    assert lease is None


def test_lease_expires_when_as_of_exceeds_expires_at() -> None:
    claim = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_section_claimed",
        ts=1000,
        lease_owner="agent-a",
        lease_token="t1",
        lease_expires_at=1200,
    )

    state = reduce_plan_events([claim])
    assert get_effective_lease(state, plan_artifact_id="plan-uuid", fragment_id="frag-1", as_of_ms=1100) is not None
    assert get_effective_lease(state, plan_artifact_id="plan-uuid", fragment_id="frag-1", as_of_ms=1200) is None
    assert get_effective_lease(state, plan_artifact_id="plan-uuid", fragment_id="frag-1", as_of_ms=1300) is None


def test_competing_claims_same_ts_tie_breaks_by_event_id() -> None:
    a_claim = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_section_claimed",
        ts=1000,
        lease_owner="agent-a",
        lease_token="t1",
        lease_expires_at=2000,
    )
    b_claim = build_plan_event(
        project_id="proj",
        scope_id="scope",
        plan_artifact_id="plan-uuid",
        fragment_id="frag-1",
        kind="plan_section_claimed",
        ts=1000,
        lease_owner="agent-b",
        lease_token="t2",
        lease_expires_at=2000,
    )

    state = reduce_plan_events([a_claim, b_claim])
    lease = get_effective_lease(state, plan_artifact_id="plan-uuid", fragment_id="frag-1", as_of_ms=1500)
    assert lease is not None

    expected_owner = "agent-a" if a_claim["eventId"] >= b_claim["eventId"] else "agent-b"
    assert lease.owner == expected_owner
