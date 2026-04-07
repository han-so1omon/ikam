"""Query evaluator — fact coverage and quality scoring via judge."""
from __future__ import annotations

from dataclasses import dataclass, field

from ikam.oraculo.graph_state import GraphState
from ikam.oraculo.judge import JudgeProtocol, JudgeQuery
from ikam.oraculo.reports import QueryReport
from ikam.oraculo.spec import OracleSpec


@dataclass
class QueryResult:
    """Result of evaluating a single benchmark query response."""

    query: str
    facts_found: list[str] = field(default_factory=list)
    fact_coverage: float = 0.0
    quality_score: float = 0.0
    answer_text: str = ""
    evidence_fragment_ids: list[str] = field(default_factory=list)


class QueryEvaluator:
    """Asks the judge to answer queries against graph context, then scores fact coverage."""

    def evaluate(self, graph_state: GraphState, spec: OracleSpec, judge: JudgeProtocol) -> QueryReport:
        results: list[QueryResult] = []

        for bq in spec.benchmark_queries:
            traversal = graph_state.traverse(bq.query, judge)
            fragment_texts = [str(f.value) for f in traversal.fragments if f.value]
            context_str = "\n".join(fragment_texts)

            # Ask judge to answer the query
            judgment = judge.judge(JudgeQuery(
                question=bq.query,
                context={"graph_context": context_str, "required_facts": bq.required_facts},
            ))

            # Calculate fact coverage from judge's facts_found
            facts_found = judgment.facts_found or []
            required = bq.required_facts
            if required:
                covered = sum(
                    1 for req in required
                    if any(req.lower() in f.lower() for f in facts_found)
                )
                fact_coverage = covered / len(required)
            else:
                fact_coverage = 1.0

            # Quality score derived from judge score (scaled to 0-10)
            quality_score = judgment.score * 10.0
            answer_text = str(judgment.reasoning or "").strip()
            fragment_ids = [str(fragment.cas_id or "") for fragment in traversal.fragments if getattr(fragment, "cas_id", None)]

            results.append(QueryResult(
                query=bq.query,
                facts_found=facts_found,
                fact_coverage=fact_coverage,
                quality_score=quality_score,
                answer_text=answer_text,
                evidence_fragment_ids=fragment_ids,
            ))

        mean_fact_coverage = (
            sum(r.fact_coverage for r in results) / len(results)
            if results else 0.0
        )
        mean_quality_score = (
            sum(r.quality_score for r in results) / len(results)
            if results else 0.0
        )

        passed = mean_fact_coverage >= 0.8 and mean_quality_score >= 7.0

        return QueryReport(
            results=results,
            mean_fact_coverage=mean_fact_coverage,
            mean_quality_score=mean_quality_score,
            passed=passed,
        )
