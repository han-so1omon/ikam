from __future__ import annotations

from typing import Any
import psycopg

from modelado.registry import OperatorRegistryAdapter, RegistryManager, get_shared_registry_manager

from .composition import ComposeOperator
from .chunking import ChunkOperator
from .claims import ClaimsOperator
from .commit import CommitOperator
from .entities_and_relationships import EntitiesAndRelationshipsOperator
from .identify import IdentifyOperator
from .load_documents import LoadDocumentsOperator
from .lifting import LiftOperator
from .mapping import MapDNAOperator
from .monadic import ResolveOperator, VerifyOperator
from .semantic import EmbedOperator, NormalizeOperator, SearchOperator
from .repair import RepairOperator


DEFAULT_OPERATOR_NAMESPACE = "operators.default"


def create_default_operator_registry(
    cx: psycopg.Connection[Any],
    manager: RegistryManager | None = None,
    *,
    namespace: str = DEFAULT_OPERATOR_NAMESPACE,
    overrides: dict[str, object] | None = None,
) -> OperatorRegistryAdapter[object]:
    registry = OperatorRegistryAdapter(
        cx,
        manager or get_shared_registry_manager(),
        namespace,
    )

    default_entries = {
        "modelado/operators/noop": ResolveOperator(),
        "modelado/operators/map_dna": MapDNAOperator(),
        "modelado/operators/identify": IdentifyOperator(),
        "modelado/operators/load_documents": LoadDocumentsOperator(),
        "modelado/operators/chunking": ChunkOperator(),
        "modelado/operators/entities_and_relationships": EntitiesAndRelationshipsOperator(),
        "modelado/operators/claims": ClaimsOperator(),
        "modelado/operators/lift": LiftOperator(),
        "modelado/operators/verify": VerifyOperator(),
        "modelado/operators/repair": RepairOperator(),
        "modelado/operators/embed": EmbedOperator(),
        "modelado/operators/search": SearchOperator(),
        "modelado/operators/normalize": NormalizeOperator(),
        "modelado/operators/compose": ComposeOperator(),
        "modelado/operators/commit": CommitOperator(),
    }
    override_keys = set(overrides.keys()) if overrides else set()
    if overrides:
        default_entries.update(overrides)

    existing_keys = set(registry.list_keys())

    for key, operator in default_entries.items():
        if key in override_keys or key not in existing_keys:
            registry.register(key, operator)

    return registry
