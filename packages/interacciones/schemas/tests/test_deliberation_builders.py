"""Unit tests for deliberation builder helpers."""

from interacciones.schemas import build_deliberation_system_event


def test_build_deliberation_system_event_shape() -> None:
    evt = build_deliberation_system_event(
        project_id="proj-1",
        session_id="sess-1",
        parent_id="123",
        run_id="123",
        ts=1700000000000,
        phase="plan",
        status="started",
        summary="Planning started",
        details="More details",
        evidence=[{"kind": "artifact", "ref": "plan:abc", "label": "plan"}],
    )

    assert evt["scope"] == "system"
    assert evt["type"] == "system_event"
    assert evt["content"] == "Planning started"
    assert evt["parent_id"] == "123"

    md = evt["metadata"]
    assert md["event_type"] == "deliberation"
    d = md["deliberation"]
    assert d["runId"] == "123"
    assert d["projectId"] == "proj-1"
    assert d["ts"] == 1700000000000
    assert d["phase"] == "plan"
    assert d["status"] == "started"
    assert d["summary"] == "Planning started"
    assert d["details"] == "More details"
    assert d["evidence"] == [{"kind": "artifact", "ref": "plan:abc", "label": "plan"}]
