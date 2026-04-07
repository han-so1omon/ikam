"""Tests for oracle fixture schema validation — s-local-retail-v01."""
from __future__ import annotations

from ikam.oraculo.spec import OracleSpec


def test_s_local_retail_oracle_fixture_loads_and_has_required_sections():
    """Oracle fixture must load and contain all four required sections."""
    spec = OracleSpec.from_json("tests/fixtures/cases/s-local-retail-v01/oracle.json")
    assert spec.case_id == "s-local-retail-v01"
    assert spec.entities, "must have at least one expected entity"
    assert spec.predicates, "must have at least one expected predicate"
    assert spec.contradictions, "must have at least one expected contradiction"
    assert spec.benchmark_queries, "must have at least one benchmark query"


def test_s_local_retail_oracle_has_key_entities():
    """Oracle must include the core entities from the idea.md glossary."""
    spec = OracleSpec.from_json("tests/fixtures/cases/s-local-retail-v01/oracle.json")
    entity_names = {e.name.lower() for e in spec.entities}
    assert "maya chen" in entity_names
    assert "bramble & bitters" in entity_names or "bramble and bitters" in entity_names


def test_s_local_retail_oracle_has_four_contradictions():
    """idea.md defines 4 intentional contradictions; oracle must capture all."""
    spec = OracleSpec.from_json("tests/fixtures/cases/s-local-retail-v01/oracle.json")
    assert len(spec.contradictions) >= 4


def test_s_local_retail_oracle_has_benchmark_queries_with_facts():
    """Each benchmark query must have required_facts for checklist evaluation."""
    spec = OracleSpec.from_json("tests/fixtures/cases/s-local-retail-v01/oracle.json")
    for bq in spec.benchmark_queries:
        assert bq.query, "query must be non-empty"
        assert bq.required_facts, f"query '{bq.query}' must have required_facts"
