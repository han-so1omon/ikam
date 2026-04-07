"""GraphState — backend-agnostic graph access protocol.

Defines the `GraphState` protocol that evaluation code programs against,
plus `InMemoryGraphState` for unit-testing evaluators without Postgres.
"""
from __future__ import annotations

import copy
from dataclasses import dataclass, field
from typing import Any, Protocol, runtime_checkable

from ikam.forja.contracts import ExtractedEntity, ExtractedRelation
from ikam.fragments import Fragment
from ikam.oraculo.retrieval import retrieve_relevant_fragments


# ------------------------------------------------------------------
# Forward-reference types used by the protocol but defined elsewhere
# ------------------------------------------------------------------

@dataclass
class TraversalResult:
    """Result of a graph traversal query."""

    fragments: list[Fragment] = field(default_factory=list)
    reasoning: list[str] = field(default_factory=list)


@dataclass
class MutationRecord:
    """Record of an applied mutation."""

    mutation_id: str = ""
    mutation_type: str = ""
    fragments_created: list[str] = field(default_factory=list)
    fragments_reused: list[str] = field(default_factory=list)
    edges_added: list[str] = field(default_factory=list)
    edges_removed: list[str] = field(default_factory=list)
    provenance_recorded: bool = False


@dataclass
class ProposedMutation:
    """A mutation proposed by oráculo for graph improvement."""

    mutation_type: str  # "contradiction_resolution" | "artifact_injection" | "entity_correction"
    description: str = ""
    target_entities: list[str] = field(default_factory=list)
    target_fragments: list[str] = field(default_factory=list)
    metadata: dict[str, Any] = field(default_factory=dict)


# ------------------------------------------------------------------
# Protocol
# ------------------------------------------------------------------

@runtime_checkable
class GraphState(Protocol):
    """Backend-agnostic read/write access to an IKAM knowledge graph."""

    # Fragment access
    def fragments(self) -> list[Fragment]: ...
    def fragment_by_id(self, fragment_id: str) -> Fragment | None: ...

    # Entity access
    def entities(self) -> list[ExtractedEntity]: ...
    def entity_by_name(self, name: str) -> ExtractedEntity | None: ...

    # Relation/predicate access
    def relations(self) -> list[ExtractedRelation]: ...
    def relations_for(self, entity_name: str) -> list[ExtractedRelation]: ...

    # Graph traversal
    def traverse(self, query: str, judge: Any) -> TraversalResult: ...

    # Snapshot for before/after comparison
    def snapshot(self) -> GraphState: ...

    # Mutation
    def apply_mutation(self, mutation: ProposedMutation) -> MutationRecord: ...

    # Metrics
    def total_bytes(self) -> int: ...
    def unique_bytes(self) -> int: ...
    def fragment_count(self) -> int: ...
    def unique_fragment_count(self) -> int: ...


# ------------------------------------------------------------------
# In-memory implementation (for unit testing)
# ------------------------------------------------------------------

class InMemoryGraphState:
    """Dict-backed GraphState for unit-testing oráculo evaluators."""

    def __init__(self) -> None:
        self._fragments: dict[str, Fragment] = {}
        self._entities: dict[str, ExtractedEntity] = {}
        self._relations: list[ExtractedRelation] = []
        self._fragment_total_count: int = 0
        self._fragment_total_bytes: int = 0

    # -- mutation helpers (not on protocol) --

    def add_fragment(self, fragment: Fragment) -> None:
        self._fragment_total_count += 1
        if fragment.value is not None:
            self._fragment_total_bytes += len(str(fragment.value).encode("utf-8"))
        key = fragment.cas_id or id(fragment)
        self._fragments[str(key)] = fragment

    def add_entity(self, entity: ExtractedEntity) -> None:
        self._entities[entity.canonical_label] = entity

    def add_relation(self, relation: ExtractedRelation) -> None:
        self._relations.append(relation)

    # -- protocol implementation --

    def fragments(self) -> list[Fragment]:
        return list(self._fragments.values())

    def fragment_by_id(self, fragment_id: str) -> Fragment | None:
        return self._fragments.get(fragment_id)

    def entities(self) -> list[ExtractedEntity]:
        return list(self._entities.values())

    def entity_by_name(self, name: str) -> ExtractedEntity | None:
        lower = name.lower()
        for e in self._entities.values():
            if e.canonical_label.lower() == lower or e.label.lower() == lower:
                return e
        return None

    def relations(self) -> list[ExtractedRelation]:
        return list(self._relations)

    def relations_for(self, entity_name: str) -> list[ExtractedRelation]:
        lower = entity_name.lower()
        return [
            r
            for r in self._relations
            if r.source_label.lower() == lower or r.target_label.lower() == lower
        ]

    def traverse(self, query: str, judge: Any) -> TraversalResult:
        selected, reasoning = retrieve_relevant_fragments(
            query=query,
            fragments=self.fragments(),
            entities=self.entities(),
            relations=self.relations(),
        )
        return TraversalResult(fragments=selected, reasoning=reasoning)

    def snapshot(self) -> InMemoryGraphState:
        snap = InMemoryGraphState()
        snap._fragments = copy.deepcopy(self._fragments)
        snap._entities = copy.deepcopy(self._entities)
        snap._relations = copy.deepcopy(self._relations)
        snap._fragment_total_count = self._fragment_total_count
        snap._fragment_total_bytes = self._fragment_total_bytes
        return snap

    def apply_mutation(self, mutation: ProposedMutation) -> MutationRecord:
        return MutationRecord(
            mutation_id=f"mem-{id(mutation)}",
            mutation_type=mutation.mutation_type,
            provenance_recorded=True,
        )

    def total_bytes(self) -> int:
        return self._fragment_total_bytes

    def unique_bytes(self) -> int:
        total = 0
        for fragment in self._fragments.values():
            if fragment.value is not None:
                total += len(str(fragment.value).encode("utf-8"))
        return total

    def fragment_count(self) -> int:
        return self._fragment_total_count

    def unique_fragment_count(self) -> int:
        # In-memory: all fragments are unique by key
        return len(self._fragments)


__all__ = [
    "GraphState",
    "InMemoryGraphState",
    "MutationRecord",
    "ProposedMutation",
    "TraversalResult",
]
