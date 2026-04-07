"""Wave 6 — Within-Case Reuse Metrics Tests.

Validates that the report generator defaults to within-case metrics only,
cross-case reuse is excluded from the primary report, and merge mode
emits a separate appendix section.
"""

from __future__ import annotations

import sys
from pathlib import Path
from types import ModuleType

import pytest

# Add scripts/analysis to path for report module import
_SCRIPTS_ROOT = Path(__file__).resolve().parents[4] / "scripts" / "analysis"

if not _SCRIPTS_ROOT.exists():
    pytest.skip("Scripts directory not mounted in this environment", allow_module_level=True)

sys.path.insert(0, str(_SCRIPTS_ROOT))


from fragment_reuse_report import (
    CaseStats,
    FragmentRecord,
    generate_report,
)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

def _make_records(case_id: str, cas_ids: list[str]) -> list[FragmentRecord]:
    """Build minimal FragmentRecords for a case."""
    return [
        FragmentRecord(
            cas_id=cid,
            case_id=case_id,
            file_name=f"file-{i}.md",
            level=0,
            frag_type="text",
            content_bytes=100,
            is_domain=True,
        )
        for i, cid in enumerate(cas_ids)
    ]


def _make_stats(case_id: str, records: list[FragmentRecord]) -> CaseStats:
    """Compute CaseStats from records."""
    from collections import Counter

    unique = set(r.cas_id for r in records)
    stats = CaseStats(
        case_id=case_id,
        domain="test",
        size_tier="s",
        asset_count=len(records),
        total_fragments=len(records),
        unique_cas_ids=len(unique),
        duplicate_fragments=len(records) - len(unique),
        raw_bytes=sum(r.content_bytes for r in records),
        fragment_bytes=sum(r.content_bytes for r in records),
        deduped_bytes=len(unique) * 100,
    )
    for r in records:
        stats.by_type[r.frag_type] += 1
        stats.by_level[r.level] += 1
    return stats


# Two cases with overlapping CAS IDs to test cross-case behavior
CASE_A_CAS = ["aaa", "bbb", "ccc", "aaa"]  # within-case dup: aaa
CASE_B_CAS = ["aaa", "ddd", "eee"]  # "aaa" also in case A → cross-case


@pytest.fixture
def two_case_data():
    records_a = _make_records("case-a", CASE_A_CAS)
    records_b = _make_records("case-b", CASE_B_CAS)
    stats_a = _make_stats("case-a", records_a)
    stats_b = _make_stats("case-b", records_b)
    return (
        [stats_a, stats_b],
        records_a + records_b,
    )


# ---------------------------------------------------------------------------
# Tests: primary metrics are per-case only
# ---------------------------------------------------------------------------

class TestWithinCasePrimaryMetrics:
    """Primary report metrics must be within-case only."""

    def test_executive_summary_shows_within_case_dedup(self, two_case_data):
        stats_list, all_records = two_case_data
        report = generate_report(
            case_stats=stats_list,
            all_records=all_records,
            cross_case={},
            cross_domain={},
            total_time_ms=100.0,
        )
        # Executive summary must include within-case deduplication metric
        assert "Within-Case" in report or "within-case" in report.lower()

    def test_per_case_table_shows_within_case_savings(self, two_case_data):
        stats_list, all_records = two_case_data
        report = generate_report(
            case_stats=stats_list,
            all_records=all_records,
            cross_case={},
            cross_domain={},
            total_time_ms=100.0,
        )
        # Per-case table header must exist
        assert "Per-Case" in report
        # Each case must appear
        assert "case-a" in report
        assert "case-b" in report


# ---------------------------------------------------------------------------
# Tests: cross-case reuse absent from primary report
# ---------------------------------------------------------------------------

class TestCrossCaseAbsentFromPrimary:
    """Cross-case reuse must NOT appear in the primary (default) report."""

    def test_no_cross_case_section_in_default_mode(self, two_case_data):
        stats_list, all_records = two_case_data
        # Pass empty cross-case data (default mode = no merge)
        report = generate_report(
            case_stats=stats_list,
            all_records=all_records,
            cross_case={},
            cross_domain={},
            total_time_ms=100.0,
        )
        # Cross-case section must NOT appear in default report
        assert "Cross-Case Fragment Reuse" not in report
        assert "Cross-Domain Fragment Reuse" not in report

    def test_no_global_dedup_in_executive_summary(self, two_case_data):
        stats_list, all_records = two_case_data
        report = generate_report(
            case_stats=stats_list,
            all_records=all_records,
            cross_case={},
            cross_domain={},
            total_time_ms=100.0,
        )
        # Global deduplication across cases must NOT be in executive summary
        assert "Global deduplicated storage" not in report
        assert "Global storage reduction" not in report
        assert "Cross-case shared CAS IDs" not in report
        assert "Cross-domain shared CAS IDs" not in report


# ---------------------------------------------------------------------------
# Tests: merge mode emits appendix
# ---------------------------------------------------------------------------

class TestMergeModeAppendix:
    """When cross-case data is provided (merge mode), show it as an appendix."""

    def test_cross_case_appendix_present_when_merge_data(self, two_case_data):
        stats_list, all_records = two_case_data
        cross_case = {"aaa": ["case-a", "case-b"]}
        cross_domain = {}
        report = generate_report(
            case_stats=stats_list,
            all_records=all_records,
            cross_case=cross_case,
            cross_domain=cross_domain,
            total_time_ms=100.0,
        )
        # Merge appendix must exist
        assert "Appendix" in report or "Cross-Case" in report

    def test_appendix_clearly_separated(self, two_case_data):
        stats_list, all_records = two_case_data
        cross_case = {"aaa": ["case-a", "case-b"]}
        cross_domain = {}
        report = generate_report(
            case_stats=stats_list,
            all_records=all_records,
            cross_case=cross_case,
            cross_domain=cross_domain,
            total_time_ms=100.0,
        )
        # The merge appendix must be after the primary metrics
        primary_end = report.find("CAS Axiom Validation")
        appendix_start = report.find("Cross-Case")
        if primary_end >= 0 and appendix_start >= 0:
            assert appendix_start > primary_end, (
                "Cross-case appendix must appear after primary metrics"
            )


# ---------------------------------------------------------------------------
# Tests: planning artifact required (fail closed)
# ---------------------------------------------------------------------------

class TestPlanningPreStep:
    """Semantic breakdown planning must be present per case."""

    def test_case_stats_has_planning_field(self):
        """CaseStats must track whether planning artifact was present."""
        stats = CaseStats(
            case_id="test-case",
            domain="test",
            size_tier="s",
        )
        assert hasattr(stats, "has_planning_artifact"), (
            "CaseStats must have has_planning_artifact field"
        )

    def test_planning_artifact_defaults_false(self):
        """Planning artifact defaults to False (fail closed)."""
        stats = CaseStats(
            case_id="test-case",
            domain="test",
            size_tier="s",
        )
        assert stats.has_planning_artifact is False
