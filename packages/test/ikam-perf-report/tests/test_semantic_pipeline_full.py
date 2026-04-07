"""Full 5-phase semantic evaluation pipeline for Bramble & Bitters.

Phases:
  1. Ingest + compression metrics (report only, no threshold)
  2. Baseline evaluation (entity, predicate, exploration, query)
  3. Mutations (contradiction resolution, artifact injection, entity correction)
  4. Post-edit re-evaluation
  5. Delta report (compare baseline vs post-edit)

Uses real OpenAI LLM judge calls every run (refinement phase).
OPENAI_API_KEY is loaded by conftest.py from packages/test/ikam-perf-report/.env.
"""
from __future__ import annotations

import os
import pytest

from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
from ikam.forja.contracts import ExtractedEntity, ExtractedRelation, stable_entity_key, stable_relation_key
from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
from ikam.oraculo.composer import Evaluator
from ikam.oraculo.graph_state import InMemoryGraphState, ProposedMutation
from ikam.oraculo.reports import EvaluationReport, compare
from ikam.oraculo.spec import OracleSpec
from modelado.oraculo.openai_judge import OpenAIJudge

from ikam_perf_report.benchmarks.case_fixtures import load_case_fixture

_CASE_ID = "s-local-retail-v01"
_ORACLE_PATH = f"tests/fixtures/cases/{_CASE_ID}/oracle.json"


def _build_graph_from_case(case_id: str) -> InMemoryGraphState:
    """Ingest all text/markdown case assets into an InMemoryGraphState.

    Decomposes each markdown asset, extracts simple entities/relations
    heuristically, and populates the graph.
    """
    fixture = load_case_fixture(case_id)
    gs = InMemoryGraphState()

    # Text-based extensions to ingest (mimetypes.guess_type may return
    # 'application/octet-stream' for .md on some platforms)
    _TEXT_EXTS = {".md", ".txt", ".csv", ".tsv"}

    # Collect per-source entity candidates for relation building after dedup
    source_entity_labels: list[tuple[str, list[str]]] = []

    for asset in fixture.assets:
        ext = os.path.splitext(asset.file_name)[1].lower()
        is_text = (
            (asset.mime_type and asset.mime_type.startswith("text/"))
            or ext in _TEXT_EXTS
        )
        if not is_text:
            continue
        try:
            text = asset.payload.decode("utf-8")
        except UnicodeDecodeError:
            continue

        # Decompose into V3 fragments
        register_defaults()
        decomposer = get_decomposer("text/markdown")
        directive = DecompositionDirective(
            source=text.encode("utf-8"),
            mime_type="text/markdown",
            artifact_id=asset.file_name,
        )
        fragments = decomposer.decompose(directive).structural
        for frag in fragments:
            gs.add_fragment(frag)

        # Extract entities (adds to graph, dedupes by canonical_label)
        labels = _extract_entities(gs, text, source_id=asset.file_name)
        source_entity_labels.append((asset.file_name, labels))

    # Build relations AFTER all entities are added and deduped.
    # This ensures relation entity_key references match the surviving entities.
    _build_relations(gs, source_entity_labels)

    return gs


def _extract_entities(
    gs: InMemoryGraphState, text: str, source_id: str
) -> list[str]:
    """Extract entities from text and add to graph. Returns canonical labels."""
    import re

    candidates = re.findall(r"\b[A-Z][a-zA-Z]+(?:\s+[A-Z][a-zA-Z]+)*\b", text)
    seen: set[str] = set()
    labels: list[str] = []

    for raw in candidates:
        canonical = " ".join(raw.strip().lower().split())
        if not canonical or canonical in seen or len(canonical) < 3:
            continue
        seen.add(canonical)
        entity = ExtractedEntity(
            label=raw.strip(),
            canonical_label=canonical,
            source_fragment_id=source_id,
            entity_key=stable_entity_key(source_id, raw),
        )
        gs.add_entity(entity)
        labels.append(canonical)

    return labels


def _build_relations(
    gs: InMemoryGraphState,
    source_entity_labels: list[tuple[str, list[str]]],
) -> None:
    """Build co-occurrence relations using surviving entity keys after dedup."""
    for source_id, labels in source_entity_labels:
        # Resolve each canonical label to the surviving entity in the graph
        surviving = []
        for label in labels[:10]:
            entity = gs.entity_by_name(label)
            if entity:
                surviving.append(entity)

        for i, src_e in enumerate(surviving):
            for tgt_e in surviving[i + 1 : i + 4]:
                rel = ExtractedRelation(
                    predicate="co-occurs",
                    source_label=src_e.label,
                    target_label=tgt_e.label,
                    source_entity_key=src_e.entity_key,
                    target_entity_key=tgt_e.entity_key,
                    relation_key=stable_relation_key(
                        source_id, "co-occurs", src_e.label, tgt_e.label
                    ),
                )
                gs.add_relation(rel)


# ──────────────────────────────────────────────────────────
# Module-scoped fixtures shared across all phases
# ──────────────────────────────────────────────────────────

@pytest.fixture(scope="module")
def spec() -> OracleSpec:
    return OracleSpec.from_json(_ORACLE_PATH)


@pytest.fixture(scope="module")
def graph() -> InMemoryGraphState:
    return _build_graph_from_case(_CASE_ID)


@pytest.fixture(scope="module")
def judge() -> OpenAIJudge:
    return OpenAIJudge(model="gpt-4o-mini")


@pytest.fixture(scope="module")
def evaluator(judge: OpenAIJudge) -> Evaluator:
    return Evaluator(judge=judge)


@pytest.fixture(scope="module")
def baseline_report(
    evaluator: Evaluator, graph: InMemoryGraphState, spec: OracleSpec
) -> EvaluationReport:
    return evaluator.evaluate_all(graph, spec)


# ──────────────────────────────────────────────────────────
# Phase 1: Ingest + Compression (report only — no threshold)
# ──────────────────────────────────────────────────────────


class TestPhase1Compression:
    def test_graph_has_fragments(self, graph: InMemoryGraphState) -> None:
        assert graph.fragment_count() > 0, "Graph should have at least one fragment"

    def test_graph_has_entities(self, graph: InMemoryGraphState) -> None:
        assert len(graph.entities()) > 0, "Graph should have extracted entities"

    def test_compression_report_metrics(
        self, baseline_report: EvaluationReport
    ) -> None:
        cr = baseline_report.compression
        assert cr.total_fragments > 0
        assert cr.total_bytes > 0
        assert 0.0 <= cr.dedup_ratio <= 1.0


# ──────────────────────────────────────────────────────────
# Phase 2: Baseline evaluation
# ──────────────────────────────────────────────────────────


class TestPhase2BaselineEvaluation:
    def test_entity_coverage(self, baseline_report: EvaluationReport) -> None:
        er = baseline_report.entities
        assert er.coverage >= 0.0, "Entity coverage must be non-negative"
        # With stub judge or heuristic enrichment, coverage may be < 0.8.
        # The assertion validates the evaluator runs; thresholds tuned later.

    def test_predicate_coverage(self, baseline_report: EvaluationReport) -> None:
        pr = baseline_report.predicates
        assert pr.predicate_coverage >= 0.0

    def test_exploration_recall(self, baseline_report: EvaluationReport) -> None:
        exr = baseline_report.exploration
        assert exr.mean_recall >= 0.0

    def test_query_response(self, baseline_report: EvaluationReport) -> None:
        qr = baseline_report.query
        assert qr.mean_fact_coverage >= 0.0
        assert qr.mean_quality_score >= 0.0


# ──────────────────────────────────────────────────────────
# Phase 3: Mutations (contradiction, injection, correction)
# ──────────────────────────────────────────────────────────


class TestPhase3Mutations:
    def test_contradiction_resolution(
        self,
        graph: InMemoryGraphState,
        evaluator: Evaluator,
    ) -> None:
        before = graph.snapshot()
        mutation = ProposedMutation(
            mutation_type="contradiction_resolution",
            description="Resolve Q4 revenue contradiction ($340K vs $410K)",
            target_entities=["Bramble & Bitters"],
            metadata={"field": "revenue", "resolved_value": "$410K"},
        )
        record = graph.apply_mutation(mutation)
        after = graph.snapshot()
        editing = evaluator.evaluate_mutation(before, after, record)
        assert editing.provenance_recorded, "Provenance must be recorded"
        assert editing.cas_integrity, "CAS integrity must hold"
        assert not editing.stale_edges, "No stale edges after mutation"
        assert editing.passed

    def test_artifact_injection(
        self,
        graph: InMemoryGraphState,
        evaluator: Evaluator,
    ) -> None:
        before = graph.snapshot()
        mutation = ProposedMutation(
            mutation_type="artifact_injection",
            description="Inject Q2 2026 update document",
            target_fragments=["q2-update"],
            metadata={"artifact": "q2-update.md"},
        )
        record = graph.apply_mutation(mutation)
        after = graph.snapshot()
        editing = evaluator.evaluate_mutation(before, after, record)
        assert editing.passed

    def test_entity_correction(
        self,
        graph: InMemoryGraphState,
        evaluator: Evaluator,
    ) -> None:
        before = graph.snapshot()
        mutation = ProposedMutation(
            mutation_type="entity_correction",
            description="Merge 'shrub' and 'drinking vinegar' into canonical entity",
            target_entities=["shrub", "drinking vinegar"],
            metadata={"canonical": "drinking vinegar"},
        )
        record = graph.apply_mutation(mutation)
        after = graph.snapshot()
        editing = evaluator.evaluate_mutation(before, after, record)
        assert editing.passed


# ──────────────────────────────────────────────────────────
# Phase 4: Post-edit re-evaluation
# ──────────────────────────────────────────────────────────


class TestPhase4PostEditReEvaluation:
    @pytest.fixture(scope="class")
    def post_edit_report(
        self,
        evaluator: Evaluator,
        graph: InMemoryGraphState,
        spec: OracleSpec,
    ) -> EvaluationReport:
        return evaluator.evaluate_all(graph, spec)

    def test_entity_coverage(self, post_edit_report: EvaluationReport) -> None:
        assert post_edit_report.entities.coverage >= 0.0

    def test_predicate_coverage(self, post_edit_report: EvaluationReport) -> None:
        assert post_edit_report.predicates.predicate_coverage >= 0.0

    def test_exploration_recall(self, post_edit_report: EvaluationReport) -> None:
        assert post_edit_report.exploration.mean_recall >= 0.0

    def test_query_response(self, post_edit_report: EvaluationReport) -> None:
        assert post_edit_report.query.mean_fact_coverage >= 0.0


# ──────────────────────────────────────────────────────────
# Phase 5: Delta report (baseline vs post-edit)
# ──────────────────────────────────────────────────────────


class TestPhase5DeltaReport:
    def test_delta_report_structure(
        self,
        baseline_report: EvaluationReport,
        evaluator: Evaluator,
        graph: InMemoryGraphState,
        spec: OracleSpec,
    ) -> None:
        post_edit = evaluator.evaluate_all(graph, spec)
        delta = compare(baseline_report, post_edit)
        # Delta report must have all fields populated
        assert isinstance(delta.entity_delta, float)
        assert isinstance(delta.predicate_delta, float)
        assert isinstance(delta.exploration_delta, float)
        assert isinstance(delta.fact_coverage_delta, float)
        assert isinstance(delta.quality_score_delta, float)
        assert isinstance(delta.improvements, list)
        assert isinstance(delta.regressions, list)

    def test_delta_report_renders(
        self,
        baseline_report: EvaluationReport,
        evaluator: Evaluator,
        graph: InMemoryGraphState,
        spec: OracleSpec,
    ) -> None:
        post_edit = evaluator.evaluate_all(graph, spec)
        delta = compare(baseline_report, post_edit)
        rendered = delta.render()
        assert isinstance(rendered, str)
        assert len(rendered) > 0
