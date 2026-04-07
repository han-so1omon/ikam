"""Exploration evaluator — retrieval recall via graph traversal."""
from __future__ import annotations

from dataclasses import dataclass, field

from ikam.oraculo.graph_state import GraphState
from ikam.oraculo.judge import JudgeProtocol, JudgeQuery
from ikam.oraculo.reports import ExplorationReport
from ikam.oraculo.spec import OracleSpec


@dataclass
class RetrievalResult:
    """Result of a single benchmark query's retrieval evaluation."""

    query: str
    fragments_retrieved: int = 0
    relevance_score: float = 0.0


class ExplorationEvaluator:
    """Uses GraphState.traverse() + judge for relevance scoring."""

    def evaluate(self, graph_state: GraphState, spec: OracleSpec, judge: JudgeProtocol) -> ExplorationReport:
        results: list[RetrievalResult] = []

        for bq in spec.benchmark_queries:
            traversal = graph_state.traverse(bq.query, judge)
            fragment_texts = [str(f.value) for f in traversal.fragments if f.value]

            if fragment_texts:
                judgment = judge.judge(JudgeQuery(
                    question=f"How relevant are these fragments to: {bq.query}",
                    context={"fragments": fragment_texts, "relevant_artifacts": bq.relevant_artifacts},
                ))
                relevance = judgment.score
            else:
                relevance = 0.0

            results.append(RetrievalResult(
                query=bq.query,
                fragments_retrieved=len(traversal.fragments),
                relevance_score=relevance,
            ))

        mean_recall = (
            sum(r.relevance_score for r in results) / len(results)
            if results else 0.0
        )

        passed = mean_recall >= 0.5

        return ExplorationReport(
            results=results,
            mean_recall=mean_recall,
            passed=passed,
        )
