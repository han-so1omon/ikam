"""Tests for EvaluationReport, DeltaReport, and compare()."""
from __future__ import annotations

from ikam.oraculo.reports import (
    DeltaReport,
    EvaluationReport,
    compare,
)


def test_evaluation_report_passed_requires_all_quality_dimensions_true():
    report = EvaluationReport.make_stub(
        entities_passed=True,
        predicates_passed=True,
        exploration_passed=True,
        query_passed=False,
    )
    assert report.passed is False


def test_evaluation_report_passed_all_true():
    report = EvaluationReport.make_stub(
        entities_passed=True,
        predicates_passed=True,
        exploration_passed=True,
        query_passed=True,
    )
    assert report.passed is True


def test_compare_produces_delta_report():
    baseline = EvaluationReport.make_stub(
        entities_passed=True,
        predicates_passed=True,
        exploration_passed=True,
        query_passed=True,
    )
    current = EvaluationReport.make_stub(
        entities_passed=True,
        predicates_passed=True,
        exploration_passed=True,
        query_passed=True,
    )
    delta = compare(baseline, current)
    assert isinstance(delta, DeltaReport)
    assert delta.entity_delta == 0.0
    assert isinstance(delta.improvements, list)
    assert isinstance(delta.regressions, list)


def test_evaluation_report_render_returns_string():
    report = EvaluationReport.make_stub(
        entities_passed=True,
        predicates_passed=True,
        exploration_passed=True,
        query_passed=True,
    )
    rendered = report.render()
    assert isinstance(rendered, str)
    assert "compression" in rendered.lower() or "entity" in rendered.lower()
