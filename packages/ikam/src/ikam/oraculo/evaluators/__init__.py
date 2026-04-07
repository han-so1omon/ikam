"""Oraculo evaluators — six quality dimension evaluators."""
from __future__ import annotations

from .compression import CompressionEvaluator
from .entities import EntityEvaluator
from .predicates import PredicateEvaluator
from .exploration import ExplorationEvaluator
from .editing import EditingEvaluator
from .query import QueryEvaluator

__all__ = [
    "CompressionEvaluator",
    "EditingEvaluator",
    "EntityEvaluator",
    "ExplorationEvaluator",
    "PredicateEvaluator",
    "QueryEvaluator",
]
