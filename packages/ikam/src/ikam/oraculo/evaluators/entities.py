"""Entity evaluator — deterministic entity coverage scoring."""
from __future__ import annotations

from dataclasses import dataclass

from ikam.oraculo.graph_state import GraphState
from ikam.oraculo.judge import JudgeProtocol
from ikam.oraculo.reports import EntityReport
from ikam.oraculo.spec import OracleSpec


@dataclass
class EntityMatch:
    """Records whether an expected entity was found in the graph."""

    expected_name: str
    found: bool
    matched_label: str | None = None


class EntityEvaluator:
    """Deterministic entity matching: exact name or alias against graph canonical labels."""

    def evaluate(self, graph_state: GraphState, spec: OracleSpec, judge: JudgeProtocol) -> EntityReport:
        graph_entities = graph_state.entities()
        canonical_labels = {e.canonical_label.lower() for e in graph_entities}
        raw_labels = {e.label.lower() for e in graph_entities}
        all_labels = canonical_labels | raw_labels

        matches: list[EntityMatch] = []
        for expected in spec.entities:
            names_to_check = [expected.name.lower()] + [a.lower() for a in expected.aliases]
            found = False
            matched = None
            for name in names_to_check:
                if name in all_labels:
                    found = True
                    matched = name
                    break
            matches.append(EntityMatch(expected_name=expected.name, found=found, matched_label=matched))

        found_count = sum(1 for m in matches if m.found)
        total = len(spec.entities)
        coverage = found_count / total if total > 0 else 0.0
        passed = coverage >= 0.8

        return EntityReport(
            matches=matches,
            coverage=coverage,
            passed=passed,
        )
