"""Tests for OracleSpec data model — serialization and schema contracts."""
from __future__ import annotations

import json
from pathlib import Path

from ikam.oraculo.spec import (
    BenchmarkQuery,
    ExpectedContradiction,
    ExpectedEntity,
    ExpectedPredicate,
    OracleSpec,
    Predicate,
)


def _make_spec() -> OracleSpec:
    return OracleSpec(
        case_id="s-local-retail-v01",
        domain="retail",
        entities=[
            ExpectedEntity(
                name="Maya",
                aliases=["Maya Chen"],
                entity_type="person",
                source_hint="idea.md",
            ),
        ],
        predicates=[
            ExpectedPredicate(
                label="Maya founded B&B",
                chain=[
                    Predicate(
                        source="Maya",
                        target="Bramble & Bitters",
                        relation_type="founded-by",
                        evidence_hint="pitch deck",
                    ),
                ],
                inference_type="direct",
                confidence_note="strongly stated",
            ),
        ],
        contradictions=[
            ExpectedContradiction(
                field="revenue",
                conflicting_values=["$340K", "$410K"],
                artifacts_involved=["pitch_deck.pptx", "financials.xlsx"],
                resolution_hint=None,
            ),
        ],
        benchmark_queries=[
            BenchmarkQuery(
                query="What is Bramble & Bitters' annual revenue?",
                required_facts=["revenue figure"],
                relevant_artifacts=["financials.xlsx"],
                expected_contradictions=["revenue"],
            ),
        ],
    )


def test_oracle_spec_roundtrip_json(tmp_path: Path):
    spec = _make_spec()
    path = tmp_path / "oracle.json"
    spec.to_json(str(path))
    loaded = OracleSpec.from_json(str(path))
    assert loaded.case_id == spec.case_id
    assert loaded.entities[0].name == "Maya"
    assert loaded.entities[0].aliases == ["Maya Chen"]
    assert loaded.predicates[0].label == "Maya founded B&B"
    assert loaded.contradictions[0].field == "revenue"
    assert loaded.benchmark_queries[0].query.startswith("What is")


def test_oracle_spec_entities_accessible():
    spec = _make_spec()
    assert len(spec.entities) == 1
    assert spec.entities[0].entity_type == "person"


def test_oracle_spec_empty_lists():
    spec = OracleSpec(
        case_id="empty",
        domain="test",
        entities=[],
        predicates=[],
        contradictions=[],
        benchmark_queries=[],
    )
    assert spec.case_id == "empty"
    assert spec.entities == []
