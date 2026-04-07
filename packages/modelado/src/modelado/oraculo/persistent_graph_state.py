"""PersistentGraphState — modelado adapter implementing GraphState protocol.

Initially dict-backed (same structure as InMemoryGraphState) but lives in
the modelado layer so it can be extended to wrap Postgres repositories
(ikam_graph_repository) for real pipeline runs.
"""
from __future__ import annotations

import copy
from typing import Any

from ikam.forja.contracts import ExtractedEntity, ExtractedRelation
from ikam.fragments import Fragment
from ikam.oraculo.graph_state import (
    MutationRecord,
    ProposedMutation,
    TraversalResult,
)
from ikam.oraculo.retrieval import retrieve_relevant_fragments


class PersistentGraphState:
    """GraphState implementation for real pipeline runs.

    Currently dict-backed for bootstrapping. Will be extended to use
    modelado.ikam_graph_repository for Postgres-backed persistence.
    """

    def __init__(self) -> None:
        self._fragments: dict[str, Fragment] = {}
        self._entities: dict[str, ExtractedEntity] = {}
        self._relations: list[ExtractedRelation] = []
        self._inspection_subgraphs: dict[str, dict[str, Any]] = {}
        self._fragment_total_count: int = 0
        self._fragment_total_bytes: int = 0

    # -- mutation helpers --

    def add_fragment(self, fragment: Fragment) -> None:
        self._fragment_total_count += 1
        if fragment.value is not None:
            self._fragment_total_bytes += len(str(fragment.value).encode("utf-8"))
        key = fragment.cas_id or str(id(fragment))
        self._fragments[key] = fragment

    def add_entity(self, entity: ExtractedEntity) -> None:
        self._entities[entity.canonical_label] = entity

    def add_relation(self, relation: ExtractedRelation) -> None:
        self._relations.append(relation)

    def register_inspection_subgraph(self, payload: dict[str, Any]) -> None:
        subgraph_ref = payload.get("subgraph_ref")
        if not isinstance(subgraph_ref, str) or not subgraph_ref:
            raise ValueError("persistent inspection subgraph requires subgraph_ref")
        self._inspection_subgraphs[subgraph_ref] = copy.deepcopy(payload)

    def inspection_subgraph_by_ref(self, subgraph_ref: str) -> dict[str, Any] | None:
        payload = self._inspection_subgraphs.get(subgraph_ref)
        if payload is None:
            return None
        return copy.deepcopy(payload)

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

    def snapshot(self) -> PersistentGraphState:
        snap = PersistentGraphState()
        snap._fragments = copy.deepcopy(self._fragments)
        snap._entities = copy.deepcopy(self._entities)
        snap._relations = copy.deepcopy(self._relations)
        snap._inspection_subgraphs = copy.deepcopy(self._inspection_subgraphs)
        snap._fragment_total_count = self._fragment_total_count
        snap._fragment_total_bytes = self._fragment_total_bytes
        return snap

    def apply_mutation(self, mutation: ProposedMutation) -> MutationRecord:
        return MutationRecord(
            mutation_id=f"pgs-{id(mutation)}",
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
        return len(self._fragments)
