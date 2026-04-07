"""Editing evaluator — provenance, CAS integrity, and stale edge detection."""
from __future__ import annotations

from dataclasses import dataclass

from ikam.oraculo.graph_state import GraphState, MutationRecord


@dataclass
class EditingReport:
    """Result of evaluating a graph mutation."""

    provenance_recorded: bool = False
    cas_integrity: bool = False
    stale_edges: bool = False  # True means stale edges exist (bad)
    passed: bool = False


class EditingEvaluator:
    """Compares before/after graph snapshots for editing correctness."""

    def evaluate_mutation(
        self,
        before: GraphState,
        after: GraphState,
        mutation: MutationRecord,
    ) -> EditingReport:
        provenance_recorded = mutation.provenance_recorded

        # CAS integrity: all fragments from before must still be accessible in after
        before_ids = {f.cas_id for f in before.fragments() if f.cas_id}
        after_ids = {f.cas_id for f in after.fragments() if f.cas_id}
        cas_integrity = before_ids.issubset(after_ids)

        # Stale edges: relations referencing entity keys that no longer exist
        after_entity_keys = {e.entity_key for e in after.entities()}
        stale_edges = False
        for rel in after.relations():
            src_key = rel.source_entity_key
            tgt_key = rel.target_entity_key
            if src_key and src_key not in after_entity_keys:
                stale_edges = True
                break
            if tgt_key and tgt_key not in after_entity_keys:
                stale_edges = True
                break

        passed = provenance_recorded and cas_integrity and not stale_edges

        return EditingReport(
            provenance_recorded=provenance_recorded,
            cas_integrity=cas_integrity,
            stale_edges=stale_edges,
            passed=passed,
        )
