"""Unit tests for decision trace models and helpers."""

import json

import pytest
from pydantic import ValidationError

from interacciones.schemas import (
    DecisionTrace,
    DecisionTraceCandidate,
    DecisionTraceQuery,
    coerce_decision_trace,
    decision_trace_to_json,
)


def _sample_trace_dict() -> dict:
    return {
        "version": "v1",
        "kind": "agent_routing",
        "matched_at_ms": 1700000000123,
        "query": {"domain": "model", "action": "plan", "tags": ["x"], "require_healthy": True},
        "candidates": [
            {
                "agent_id": "econ-modeler",
                "score": 0.9,
                "reasons": ["capability_match"],
                "status": "healthy",
                "in_flight": 1,
            }
        ],
        "selected_agent_id": "econ-modeler",
    }


def test_coerce_decision_trace_from_dict() -> None:
    trace = coerce_decision_trace(_sample_trace_dict())
    assert isinstance(trace, DecisionTrace)
    assert trace.kind == "agent_routing"
    assert trace.selected_agent_id == "econ-modeler"


def test_coerce_decision_trace_from_json_string() -> None:
    raw = json.dumps(_sample_trace_dict())
    trace = coerce_decision_trace(raw)
    assert trace is not None
    assert trace.query.domain == "model"


def test_decision_trace_to_json_compact() -> None:
    compact = decision_trace_to_json(_sample_trace_dict())
    assert compact is not None
    assert " " not in compact
    # round-trip
    restored = coerce_decision_trace(compact)
    assert restored is not None
    assert restored.candidates[0].agent_id == "econ-modeler"


def test_decision_trace_models_reject_unknown_fields() -> None:
    with pytest.raises(ValidationError):
        DecisionTraceCandidate.model_validate({
            "agent_id": "econ-modeler",
            "score": 1.0,
            "reasons": [],
            "status": "healthy",
            "in_flight": 0,
            "extra": "nope",
        })

    with pytest.raises(ValidationError):
        DecisionTraceQuery.model_validate({
            "domain": None,
            "action": None,
            "tags": [],
            "require_healthy": True,
            "extra": "nope",
        })

    bad = _sample_trace_dict()
    bad["extra"] = "nope"
    with pytest.raises(ValidationError):
        DecisionTrace.model_validate(bad)
