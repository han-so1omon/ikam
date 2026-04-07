"""OracleSpec — ground truth contract for graph evaluation.

Defines the expected entities, predicates, contradictions, and benchmark
queries that an IKAM knowledge graph should contain after ingesting a
case's artifacts.  Serializes to/from JSON for fixture storage.
"""
from __future__ import annotations

import json
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any


@dataclass
class ExpectedEntity:
    """An entity the oracle expects the graph to contain."""

    name: str
    aliases: list[str]
    entity_type: str
    source_hint: str


@dataclass
class Predicate:
    """A single hop in a predicate chain."""

    source: str
    target: str
    relation_type: str
    evidence_hint: str


@dataclass
class ExpectedPredicate:
    """A predicate chain the oracle expects the graph to support."""

    label: str
    chain: list[Predicate]
    inference_type: str  # "direct" | "transitive" | "inductive" | "deductive"
    confidence_note: str | None = None


@dataclass
class ExpectedContradiction:
    """A contradiction the oracle expects to be detectable."""

    field: str
    conflicting_values: list[str]
    artifacts_involved: list[str]
    resolution_hint: str | None = None


@dataclass
class BenchmarkQuery:
    """A natural-language query with ground-truth checklist."""

    query: str
    required_facts: list[str]
    relevant_artifacts: list[str]
    expected_contradictions: list[str] | None = None


@dataclass
class OracleSpec:
    """Complete ground truth specification for a benchmark case."""

    case_id: str
    domain: str
    entities: list[ExpectedEntity]
    predicates: list[ExpectedPredicate]
    contradictions: list[ExpectedContradiction]
    benchmark_queries: list[BenchmarkQuery]

    # ------------------------------------------------------------------
    # JSON serialization
    # ------------------------------------------------------------------

    def to_json(self, path: str) -> None:
        """Write spec to a JSON file."""
        data = asdict(self)
        Path(path).write_text(json.dumps(data, indent=2, ensure_ascii=False))

    @classmethod
    def from_json(cls, path: str) -> OracleSpec:
        """Load spec from a JSON file."""
        raw: dict[str, Any] = json.loads(Path(path).read_text())
        return cls(
            case_id=raw["case_id"],
            domain=raw["domain"],
            entities=[ExpectedEntity(**e) for e in raw.get("entities", [])],
            predicates=[
                ExpectedPredicate(
                    label=p["label"],
                    chain=[Predicate(**c) for c in p.get("chain", [])],
                    inference_type=p["inference_type"],
                    confidence_note=p.get("confidence_note"),
                )
                for p in raw.get("predicates", [])
            ],
            contradictions=[
                ExpectedContradiction(**c) for c in raw.get("contradictions", [])
            ],
            benchmark_queries=[
                BenchmarkQuery(**q) for q in raw.get("benchmark_queries", [])
            ],
        )


__all__ = [
    "BenchmarkQuery",
    "ExpectedContradiction",
    "ExpectedEntity",
    "ExpectedPredicate",
    "OracleSpec",
    "Predicate",
]
