"""Evaluator composer — wires all six evaluators into a single API."""
from __future__ import annotations

from ikam.oraculo.evaluators.compression import CompressionEvaluator
from ikam.oraculo.evaluators.editing import EditingEvaluator, EditingReport
from ikam.oraculo.evaluators.entities import EntityEvaluator
from ikam.oraculo.evaluators.exploration import ExplorationEvaluator
from ikam.oraculo.evaluators.predicates import PredicateEvaluator
from ikam.oraculo.evaluators.query import QueryEvaluator
from ikam.oraculo.graph_state import GraphState, MutationRecord
from ikam.oraculo.judge import JudgeProtocol
from ikam.oraculo.reports import EvaluationReport
from ikam.oraculo.spec import OracleSpec


class Evaluator:
    """Composes all six evaluators into evaluate_all() and evaluate_mutation()."""

    def __init__(self, judge: JudgeProtocol) -> None:
        self._judge = judge
        self._compression = CompressionEvaluator()
        self._entities = EntityEvaluator()
        self._predicates = PredicateEvaluator()
        self._exploration = ExplorationEvaluator()
        self._query = QueryEvaluator()
        self._editing = EditingEvaluator()

    def evaluate_all(self, graph_state: GraphState, spec: OracleSpec) -> EvaluationReport:
        """Run all quality evaluators and return aggregated report."""
        return EvaluationReport(
            compression=self._compression.evaluate(graph_state),
            entities=self._entities.evaluate(graph_state, spec, self._judge),
            predicates=self._predicates.evaluate(graph_state, spec, self._judge),
            exploration=self._exploration.evaluate(graph_state, spec, self._judge),
            query=self._query.evaluate(graph_state, spec, self._judge),
            case_id=spec.case_id,
        )

    def evaluate_mutation(
        self,
        before: GraphState,
        after: GraphState,
        mutation: MutationRecord,
    ) -> EditingReport:
        """Evaluate a single graph mutation for editing correctness."""
        return self._editing.evaluate_mutation(before, after, mutation)
