from __future__ import annotations

from typing import Protocol

from ikam.forja.boundary_planner_models import BoundaryPlan


class PlanValidationError(ValueError):
    pass


class BoundaryPlanner(Protocol):
    def plan_text(self, *, text: str, mime_type: str, artifact_id: str) -> BoundaryPlan: ...


def validate_plan_for_text(*, text: str, plan: BoundaryPlan) -> BoundaryPlan:
    if not plan.spans:
        raise PlanValidationError("boundary plan must include at least one span")

    sorted_spans = sorted(plan.spans, key=lambda s: (s.start, s.end))
    if sorted_spans != plan.spans:
        raise PlanValidationError("boundary spans must be in stable sorted order")

    text_len = len(text)
    cursor = 0
    for span in plan.spans:
        if span.start < 0 or span.end < 0:
            raise PlanValidationError("boundary spans must be non-negative")
        if span.start >= span.end:
            raise PlanValidationError("boundary span start must be < end")
        if span.start != cursor:
            raise PlanValidationError("boundary spans must fully cover text without gaps")
        if span.end > text_len:
            raise PlanValidationError("boundary span exceeds text length")
        cursor = span.end

    if cursor != text_len:
        raise PlanValidationError("boundary spans must end at text length")

    return plan


def slice_plan_text(*, text: str, plan: BoundaryPlan) -> list[str]:
    validated = validate_plan_for_text(text=text, plan=plan)
    return [text[span.start:span.end] for span in validated.spans]
