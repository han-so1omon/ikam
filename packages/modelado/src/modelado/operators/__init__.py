"""Modelado Operators — Monadic and Composite Agents.

This package defines the core operators (Layer 1) for the Petri Net execution engine.
Each operator satisfies the Monadic Kernel (M24) or Composite Agent (M21) contract.
"""

from .core import (
    Operator,
    OperatorDescriptor,
    OperatorEnv,
    OperatorParams,
    ProvenanceRecord,
    params_hash,
    record_provenance,
)
from .monadic import (
    ApplyOperator,
    EvalOperator,
    JoinOperator,
    MapOperator,
    ResolveOperator,
    VerifyOperator,
)
from .lifting import LiftOperator
from .repair import RepairOperator
from .mapping import MapDNAOperator
from .identify import IdentifyOperator
from .chunking import ChunkOperator
from .claims import ClaimsOperator
from .entities_and_relationships import EntitiesAndRelationshipsOperator
from .load_documents import LoadDocumentsOperator
from .semantic import EmbedOperator, SearchOperator, NormalizeOperator
from .composition import ComposeOperator
from .commit import CommitOperator


def create_default_operator_registry(*args, **kwargs):
    from .registry import create_default_operator_registry as _create_default_operator_registry

    return _create_default_operator_registry(*args, **kwargs)

__all__ = [
    "Operator",
    "OperatorDescriptor",
    "OperatorEnv",
    "OperatorParams",
    "ProvenanceRecord",
    "params_hash",
    "record_provenance",
    "ApplyOperator",
    "EvalOperator",
    "JoinOperator",
    "MapOperator",
    "ResolveOperator",
    "VerifyOperator",
    "LiftOperator",
    "RepairOperator",
    "MapDNAOperator",
    "IdentifyOperator",
    "ChunkOperator",
    "ClaimsOperator",
    "EntitiesAndRelationshipsOperator",
    "LoadDocumentsOperator",
    "EmbedOperator",
    "SearchOperator",
    "NormalizeOperator",
    "ComposeOperator",
    "CommitOperator",
    "create_default_operator_registry",
]
