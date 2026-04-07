"""Tests for oracle spec generation from idea text via judge."""
from __future__ import annotations

from ikam.oraculo.judge import JudgeQuery, Judgment


class StubGeneratorJudge:
    """Returns structured JSON-like responses for generation queries."""

    def judge(self, query: JudgeQuery) -> Judgment:
        task = query.context.get("task", "")
        if task == "entities":
            return Judgment(
                score=1.0,
                reasoning="found entities",
                facts_found=["Maya Chen", "Bramble & Bitters"],
                metadata={
                    "entities": [
                        {"name": "Maya Chen", "aliases": ["Maya"], "entity_type": "person", "source_hint": "idea text"},
                        {"name": "Bramble & Bitters", "aliases": ["B&B"], "entity_type": "organization", "source_hint": "idea text"},
                    ]
                },
            )
        elif task == "predicates":
            return Judgment(
                score=1.0,
                reasoning="found predicates",
                metadata={
                    "predicates": [
                        {
                            "label": "Maya founded B&B",
                            "chain": [{"source": "Maya Chen", "target": "Bramble & Bitters", "relation_type": "founded-by", "evidence_hint": "idea text"}],
                            "inference_type": "direct",
                        }
                    ]
                },
            )
        elif task == "contradictions":
            return Judgment(
                score=1.0,
                reasoning="found contradictions",
                metadata={
                    "contradictions": [
                        {"field": "revenue", "conflicting_values": ["$340K", "$410K"], "artifacts_involved": ["pitch_deck.pptx", "financials.xlsx"]}
                    ]
                },
            )
        elif task == "benchmark_queries":
            return Judgment(
                score=1.0,
                reasoning="generated queries",
                metadata={
                    "benchmark_queries": [
                        {"query": "Who founded Bramble & Bitters?", "required_facts": ["Maya Chen"], "relevant_artifacts": ["idea.md"]}
                    ]
                },
            )
        return Judgment(score=0.5, reasoning="unknown query type")


def test_generate_oracle_spec_builds_structured_spec_from_judge():
    from ikam.oraculo.generator import generate_oracle_spec

    idea = "Bramble & Bitters sells subscription boxes. Founded by Maya Chen."
    spec = generate_oracle_spec(idea_text=idea, case_id="s-local-retail-v01", judge=StubGeneratorJudge())
    assert spec.case_id == "s-local-retail-v01"
    assert isinstance(spec.entities, list)
    assert len(spec.entities) >= 1
    assert any(e.name == "Maya Chen" for e in spec.entities)


def test_generate_oracle_spec_includes_predicates():
    from ikam.oraculo.generator import generate_oracle_spec

    idea = "Maya Chen founded Bramble & Bitters."
    spec = generate_oracle_spec(idea_text=idea, case_id="test", judge=StubGeneratorJudge())
    assert len(spec.predicates) >= 1


def test_generate_oracle_spec_includes_contradictions():
    from ikam.oraculo.generator import generate_oracle_spec

    idea = "Revenue figures conflict between documents."
    spec = generate_oracle_spec(idea_text=idea, case_id="test", judge=StubGeneratorJudge())
    assert len(spec.contradictions) >= 1


def test_generate_oracle_spec_includes_benchmark_queries():
    from ikam.oraculo.generator import generate_oracle_spec

    idea = "Maya Chen founded Bramble & Bitters."
    spec = generate_oracle_spec(idea_text=idea, case_id="test", judge=StubGeneratorJudge())
    assert len(spec.benchmark_queries) >= 1
