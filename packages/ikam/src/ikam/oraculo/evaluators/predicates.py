"""Predicate evaluator — predicate chain and contradiction coverage."""
from __future__ import annotations

from dataclasses import dataclass, field

from ikam.oraculo.graph_state import GraphState
from ikam.oraculo.judge import JudgeProtocol, JudgeQuery
from ikam.oraculo.reports import PredicateReport
from ikam.oraculo.spec import OracleSpec


@dataclass
class PredicateMatch:
    """Records whether an expected predicate chain was found in the graph."""

    label: str
    chain_coverage: float = 0.0
    matched_hops: list[str] = field(default_factory=list)


@dataclass
class ContradictionMatch:
    """Records whether an expected contradiction was detected."""

    field_name: str
    detected: bool = False
    score: float = 0.0


class PredicateEvaluator:
    """Chain matching for predicates; judge-assisted contradiction detection."""

    def evaluate(self, graph_state: GraphState, spec: OracleSpec, judge: JudgeProtocol) -> PredicateReport:
        relations = graph_state.relations()

        # Build a lookup: (source_lower, target_lower, predicate_lower) -> True
        relation_set: set[tuple[str, str, str]] = set()
        for r in relations:
            relation_set.add((r.source_label.lower(), r.target_label.lower(), r.predicate.lower()))

        # Match predicate chains
        pred_matches: list[PredicateMatch] = []
        for expected_pred in spec.predicates:
            hops_found: list[str] = []
            for hop in expected_pred.chain:
                key = (hop.source.lower(), hop.target.lower(), hop.relation_type.lower())
                reverse_key = (hop.target.lower(), hop.source.lower(), hop.relation_type.lower())
                if key in relation_set or reverse_key in relation_set:
                    hops_found.append(f"{hop.source}-[{hop.relation_type}]->{hop.target}")
            chain_coverage = len(hops_found) / len(expected_pred.chain) if expected_pred.chain else 0.0
            pred_matches.append(PredicateMatch(
                label=expected_pred.label,
                chain_coverage=chain_coverage,
                matched_hops=hops_found,
            ))

        predicate_coverage = (
            sum(m.chain_coverage for m in pred_matches) / len(pred_matches)
            if pred_matches else 0.0
        )

        # Match contradictions via judge
        contradiction_matches: list[ContradictionMatch] = []
        for contradiction in spec.contradictions:
            judgment = judge.judge(JudgeQuery(
                question=f"Is there a contradiction in '{contradiction.field}' between values {contradiction.conflicting_values}?",
                context={"field": contradiction.field, "values": contradiction.conflicting_values},
            ))
            contradiction_matches.append(ContradictionMatch(
                field_name=contradiction.field,
                detected=judgment.score >= 0.5,
                score=judgment.score,
            ))

        contradiction_coverage = (
            sum(1 for c in contradiction_matches if c.detected) / len(contradiction_matches)
            if contradiction_matches else 0.0
        )

        passed = predicate_coverage >= 0.8

        return PredicateReport(
            matches=pred_matches,
            contradiction_matches=contradiction_matches,
            predicate_coverage=predicate_coverage,
            contradiction_coverage=contradiction_coverage,
            passed=passed,
        )
