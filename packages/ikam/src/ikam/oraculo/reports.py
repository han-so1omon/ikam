"""EvaluationReport, DeltaReport, and compare() — report models for oráculo.

Reports aggregate evaluator outputs into a single pass/fail judgment
and support before/after comparison via DeltaReport.
"""
from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone


# ------------------------------------------------------------------
# Stub sub-report types (lightweight, no evaluator dependency)
# ------------------------------------------------------------------

@dataclass
class CompressionReport:
    """Compression metrics (no threshold — report only)."""

    total_fragments: int = 0
    unique_fragments: int = 0
    dedup_ratio: float = 0.0
    total_bytes: int = 0
    unique_bytes: int = 0
    byte_savings: float = 0.0
    fragment_size_distribution: dict[str, int] = field(default_factory=dict)


@dataclass
class EntityReport:
    """Entity coverage results."""

    matches: list = field(default_factory=list)
    unexpected_entities: list[str] = field(default_factory=list)
    coverage: float = 0.0
    passed: bool = False


@dataclass
class PredicateReport:
    """Predicate and contradiction coverage results."""

    matches: list = field(default_factory=list)
    contradiction_matches: list = field(default_factory=list)
    predicate_coverage: float = 0.0
    contradiction_coverage: float = 0.0
    passed: bool = False


@dataclass
class ExplorationReport:
    """Exploration recall results."""

    results: list = field(default_factory=list)
    mean_recall: float = 0.0
    passed: bool = False


@dataclass
class QueryReport:
    """Query response quality results."""

    results: list = field(default_factory=list)
    mean_fact_coverage: float = 0.0
    mean_quality_score: float = 0.0
    passed: bool = False


# ------------------------------------------------------------------
# EvaluationReport
# ------------------------------------------------------------------

@dataclass
class EvaluationReport:
    """Aggregated evaluation across all quality dimensions."""

    compression: CompressionReport = field(default_factory=CompressionReport)
    entities: EntityReport = field(default_factory=EntityReport)
    predicates: PredicateReport = field(default_factory=PredicateReport)
    exploration: ExplorationReport = field(default_factory=ExplorationReport)
    query: QueryReport = field(default_factory=QueryReport)
    timestamp: datetime = field(default_factory=lambda: datetime.now(timezone.utc))
    case_id: str | None = None

    @property
    def passed(self) -> bool:
        """Overall pass: all quality dimensions must pass (compression excluded)."""
        return all([
            self.entities.passed,
            self.predicates.passed,
            self.exploration.passed,
            self.query.passed,
        ])

    @classmethod
    def make_stub(
        cls,
        *,
        entities_passed: bool = False,
        predicates_passed: bool = False,
        exploration_passed: bool = False,
        query_passed: bool = False,
        entity_coverage: float = 0.0,
        predicate_coverage: float = 0.0,
        exploration_recall: float = 0.0,
        fact_coverage: float = 0.0,
        quality_score: float = 0.0,
    ) -> EvaluationReport:
        """Create a minimal report for testing."""
        return cls(
            compression=CompressionReport(),
            entities=EntityReport(passed=entities_passed, coverage=entity_coverage),
            predicates=PredicateReport(passed=predicates_passed, predicate_coverage=predicate_coverage),
            exploration=ExplorationReport(passed=exploration_passed, mean_recall=exploration_recall),
            query=QueryReport(passed=query_passed, mean_fact_coverage=fact_coverage, mean_quality_score=quality_score),
        )

    def render(self) -> str:
        """Render a human-readable summary."""
        lines = [
            f"# Evaluation Report  (passed={self.passed})",
            "",
            "## Compression",
            f"  fragments: {self.compression.total_fragments} total, {self.compression.unique_fragments} unique",
            f"  bytes: {self.compression.total_bytes} total, {self.compression.unique_bytes} unique",
            f"  dedup_ratio: {self.compression.dedup_ratio:.2%}",
            "",
            "## Entities",
            f"  coverage: {self.entities.coverage:.2%}  passed={self.entities.passed}",
            "",
            "## Predicates",
            f"  predicate_coverage: {self.predicates.predicate_coverage:.2%}",
            f"  contradiction_coverage: {self.predicates.contradiction_coverage:.2%}",
            f"  passed={self.predicates.passed}",
            "",
            "## Exploration",
            f"  mean_recall: {self.exploration.mean_recall:.2%}  passed={self.exploration.passed}",
            "",
            "## Query",
            f"  mean_fact_coverage: {self.query.mean_fact_coverage:.2%}",
            f"  mean_quality_score: {self.query.mean_quality_score:.1f}/10",
            f"  passed={self.query.passed}",
        ]
        return "\n".join(lines)


# ------------------------------------------------------------------
# DeltaReport and compare()
# ------------------------------------------------------------------

@dataclass
class DeltaReport:
    """Delta between a baseline and current evaluation."""

    baseline: EvaluationReport
    current: EvaluationReport
    entity_delta: float = 0.0
    predicate_delta: float = 0.0
    exploration_delta: float = 0.0
    fact_coverage_delta: float = 0.0
    quality_score_delta: float = 0.0
    improvements: list[str] = field(default_factory=list)
    regressions: list[str] = field(default_factory=list)

    def render(self) -> str:
        """Render a human-readable delta summary."""
        lines = [
            "# Delta Report",
            "",
            f"  entity_delta:        {self.entity_delta:+.2%}",
            f"  predicate_delta:     {self.predicate_delta:+.2%}",
            f"  exploration_delta:   {self.exploration_delta:+.2%}",
            f"  fact_coverage_delta: {self.fact_coverage_delta:+.2%}",
            f"  quality_score_delta: {self.quality_score_delta:+.1f}",
        ]
        if self.improvements:
            lines.append("")
            lines.append("Improvements:")
            for item in self.improvements:
                lines.append(f"  + {item}")
        if self.regressions:
            lines.append("")
            lines.append("Regressions:")
            for item in self.regressions:
                lines.append(f"  - {item}")
        return "\n".join(lines)


def compare(baseline: EvaluationReport, current: EvaluationReport) -> DeltaReport:
    """Compare two evaluation reports and produce a delta."""
    entity_delta = current.entities.coverage - baseline.entities.coverage
    predicate_delta = current.predicates.predicate_coverage - baseline.predicates.predicate_coverage
    exploration_delta = current.exploration.mean_recall - baseline.exploration.mean_recall
    fact_delta = current.query.mean_fact_coverage - baseline.query.mean_fact_coverage
    quality_delta = current.query.mean_quality_score - baseline.query.mean_quality_score

    improvements: list[str] = []
    regressions: list[str] = []

    for name, delta in [
        ("entities", entity_delta),
        ("predicates", predicate_delta),
        ("exploration", exploration_delta),
        ("fact_coverage", fact_delta),
    ]:
        if delta > 0.01:
            improvements.append(f"{name}: +{delta:.2%}")
        elif delta < -0.01:
            regressions.append(f"{name}: {delta:.2%}")

    if quality_delta > 0.1:
        improvements.append(f"quality_score: +{quality_delta:.1f}")
    elif quality_delta < -0.1:
        regressions.append(f"quality_score: {quality_delta:.1f}")

    return DeltaReport(
        baseline=baseline,
        current=current,
        entity_delta=entity_delta,
        predicate_delta=predicate_delta,
        exploration_delta=exploration_delta,
        fact_coverage_delta=fact_delta,
        quality_score_delta=quality_delta,
        improvements=improvements,
        regressions=regressions,
    )


__all__ = [
    "CompressionReport",
    "DeltaReport",
    "EntityReport",
    "EvaluationReport",
    "ExplorationReport",
    "PredicateReport",
    "QueryReport",
    "compare",
]
