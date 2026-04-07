"""Provenance backend facade (DB-agnostic).

This module intentionally contains **no DB/graph execution**. All storage and
traversal is delegated through the :class:`ikam.almacen.provenance_store.ProvenanceGraphStore`
port, implemented in `packages/modelado`.
"""
from __future__ import annotations

from collections import defaultdict

from .provenance_store import ProvenanceGraphStore
from ..provenance import DerivationRecord, DerivationType, ProvenanceChain


class ProvenanceBackend:
    """Facade providing provenance queries through a `ProvenanceGraphStore`.

    The backend is intentionally storage-agnostic: it coordinates traversal logic
    and returns IKAM domain types, but never talks to a database or graph server.
    """

    def __init__(self, graph_store: ProvenanceGraphStore):
        self._graph_store = graph_store

    def record_derivation(self, derivation: DerivationRecord) -> str:
        return self._graph_store.record_derivation(derivation)

    def get_derivations(
        self,
        *,
        source_key: str | None = None,
        target_key: str | None = None,
        derivation_type: DerivationType | None = None,
    ) -> list[DerivationRecord]:
        return list(
            self._graph_store.list_derivations(
                source_key=source_key,
                target_key=target_key,
                derivation_type=derivation_type,
            )
        )

    def get_derivation_chain(self, fragment_key: str, *, max_depth: int = 50) -> list[DerivationRecord]:
        """Return a single canonical (deterministic) parent chain.

        If the graph branches, this picks the first parent deterministically by:
        (created_at, source_key, target_key).
        """

        chain: list[DerivationRecord] = []
        current = fragment_key

        for _ in range(max(0, int(max_depth))):
            parents = list(self._graph_store.list_derivations(target_key=current))
            if not parents:
                break
            parents.sort(key=lambda d: (d.created_at, d.source_key, d.target_key))
            edge = parents[0]
            chain.append(edge)
            current = edge.source_key

        chain.reverse()  # oldest → newest
        return chain

    def get_provenance_chains(
        self,
        target_key: str,
        *,
        max_depth: int = 50,
        max_nodes: int = 10_000,
        max_edges: int = 50_000,
    ) -> list[ProvenanceChain]:
        """Return all simple (cycle-free) derivation chains for a target.

        Chains are returned ordered by decreasing length to make a canonical pick
        easy for callers.
        """

        max_depth = max(0, int(max_depth))
        max_nodes = max(1, int(max_nodes))
        max_edges = max(1, int(max_edges))

        parent_cache: dict[str, list[DerivationRecord]] = {}
        total_edges_seen = 0
        total_nodes_seen: set[str] = {target_key}

        def parents(node: str) -> list[DerivationRecord]:
            nonlocal total_edges_seen
            cached = parent_cache.get(node)
            if cached is not None:
                return cached
            edges = list(self._graph_store.list_derivations(target_key=node))
            total_edges_seen += len(edges)
            if total_edges_seen > max_edges:
                raise RuntimeError(f"max_edges exceeded: {total_edges_seen} > {max_edges}")
            for e in edges:
                total_nodes_seen.add(e.source_key)
            if len(total_nodes_seen) > max_nodes:
                raise RuntimeError(f"max_nodes exceeded: {len(total_nodes_seen)} > {max_nodes}")
            parent_cache[node] = edges
            return edges

        if not parents(target_key):
            return []

        # Stack items are: current_node, artifact_path (target→...→ancestor),
        # derivation records (target-edge→...→).
        stack: list[tuple[str, list[str], list[DerivationRecord]]] = [
            (target_key, [target_key], [])
        ]
        chains: list[ProvenanceChain] = []

        while stack:
            node, path, derivations = stack.pop()
            if len(derivations) >= max_depth:
                chains.append(_to_chain(target_key, path, derivations))
                continue

            ps = parents(node)
            if not ps:
                chains.append(_to_chain(target_key, path, derivations))
                continue

            for edge in ps:
                if edge.source_key in path:
                    continue
                stack.append(
                    (
                        edge.source_key,
                        path + [edge.source_key],
                        derivations + [edge],
                    )
                )

        chains.sort(key=lambda c: c.chain_length, reverse=True)
        return chains

    def calculate_fisher_info_total(self, target_key: str | None = None) -> float:
        """Sum Fisher Information contributions.

        When `target_key` is provided, sums only the canonical derivation chain
        for that target. Otherwise sums all known derivations.
        """

        derivations = (
            self.get_derivation_chain(target_key) if target_key is not None else self.get_derivations()
        )
        return float(sum(d.fisher_info_contribution or 0.0 for d in derivations))

    def get_fisher_info_breakdown(self, target_key: str | None = None) -> dict[str, float]:
        """Return Fisher Information totals grouped by derivation type.

        Returns a mapping like {"reuse": 2.5, "delta": 1.0}.
        """

        derivations = (
            self.get_derivation_chain(target_key) if target_key is not None else self.get_derivations()
        )
        breakdown: dict[str, float] = defaultdict(float)
        for d in derivations:
            breakdown[d.derivation_type.value] += float(d.fisher_info_contribution or 0.0)
        return dict(breakdown)


def _to_chain(target_key: str, path: list[str], derivations: list[DerivationRecord]) -> ProvenanceChain:
    return ProvenanceChain(
        target_artifact_id=target_key,
        chain_length=len(derivations),
        artifacts=list(reversed(path)),
        derivations=[
            f"{d.source_key}->{d.target_key}:{d.derivation_type.value}:{d.created_at.isoformat()}"
            for d in reversed(derivations)
        ],
        derivation_types=[d.derivation_type.value for d in reversed(derivations)],
    )


__all__ = ["ProvenanceBackend"]
