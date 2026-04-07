from __future__ import annotations

import pytest

from ikam.forja.boundary_planner import PlanValidationError, validate_plan_for_text
from ikam.forja.boundary_planner_models import BoundaryPlan, BoundarySpan


def test_validate_plan_requires_full_coverage_and_non_overlap() -> None:
    text = "alpha beta gamma"
    plan = BoundaryPlan(
        spans=[
            BoundarySpan(start=0, end=6, label="a"),
            BoundarySpan(start=6, end=len(text), label="b"),
        ],
        provider="test",
        model="test-model",
        prompt_version="v1",
    )

    validated = validate_plan_for_text(text=text, plan=plan)
    assert [s.label for s in validated.spans] == ["a", "b"]


def test_validate_plan_rejects_overlaps() -> None:
    text = "0123456789"
    bad = BoundaryPlan(
        spans=[
            BoundarySpan(start=0, end=7, label="left"),
            BoundarySpan(start=6, end=10, label="right"),
        ],
        provider="test",
        model="test-model",
        prompt_version="v1",
    )

    with pytest.raises(PlanValidationError):
        validate_plan_for_text(text=text, plan=bad)


def test_validate_plan_rejects_incomplete_coverage() -> None:
    text = "abcdefghij"
    bad = BoundaryPlan(
        spans=[BoundarySpan(start=1, end=9, label="middle")],
        provider="test",
        model="test-model",
        prompt_version="v1",
    )

    with pytest.raises(PlanValidationError):
        validate_plan_for_text(text=text, plan=bad)
