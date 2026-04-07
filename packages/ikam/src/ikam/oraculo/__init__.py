"""Oráculo — reusable graph evaluation engine for IKAM.

Layer 0 package: pure protocols, data models, and deterministic logic.
No external dependencies beyond pydantic and stdlib.
"""
from __future__ import annotations

from .spec import (
    BenchmarkQuery,
    ExpectedContradiction,
    ExpectedEntity,
    ExpectedPredicate,
    OracleSpec,
    Predicate,
)
from .judge import JudgeProtocol, JudgeQuery, Judgment
from .graph_state import GraphState, InMemoryGraphState
from .reports import EvaluationReport, DeltaReport, compare
from .graph_state import ProposedMutation, MutationRecord
from .generator import generate_oracle_spec
from .mutations import suggest_mutations

__all__ = [
    # spec
    "BenchmarkQuery",
    "ExpectedContradiction",
    "ExpectedEntity",
    "ExpectedPredicate",
    "OracleSpec",
    "Predicate",
    # judge
    "JudgeProtocol",
    "JudgeQuery",
    "Judgment",
    # graph_state
    "GraphState",
    "InMemoryGraphState",
    "ProposedMutation",
    "MutationRecord",
    # reports
    "EvaluationReport",
    "DeltaReport",
    "compare",
    # generator
    "generate_oracle_spec",
    # mutations
    "suggest_mutations",
]
