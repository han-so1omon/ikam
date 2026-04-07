"""Mutation suggestion — identify spec-vs-graph gaps and propose mutations."""
from __future__ import annotations

from ikam.oraculo.graph_state import GraphState, ProposedMutation
from ikam.oraculo.spec import OracleSpec


def suggest_mutations(spec: OracleSpec, graph_state: GraphState) -> list[ProposedMutation]:
    """Analyze gaps between spec expectations and graph state, propose mutations."""
    proposals: list[ProposedMutation] = []

    # 1. Missing entities → entity_correction
    graph_entities = graph_state.entities()
    canonical_labels = {e.canonical_label.lower() for e in graph_entities}
    raw_labels = {e.label.lower() for e in graph_entities}
    all_labels = canonical_labels | raw_labels

    for expected in spec.entities:
        names_to_check = [expected.name.lower()] + [a.lower() for a in expected.aliases]
        found = any(name in all_labels for name in names_to_check)
        if not found:
            proposals.append(ProposedMutation(
                mutation_type="entity_correction",
                description=f"Missing entity: {expected.name} (type: {expected.entity_type})",
                target_entities=[expected.name],
                metadata={"source_hint": expected.source_hint, "entity_type": expected.entity_type},
            ))

    # 2. Unresolved contradictions → contradiction_resolution
    for contradiction in spec.contradictions:
        proposals.append(ProposedMutation(
            mutation_type="contradiction_resolution",
            description=f"Contradiction in '{contradiction.field}': {contradiction.conflicting_values}",
            target_fragments=[],
            metadata={
                "field": contradiction.field,
                "conflicting_values": contradiction.conflicting_values,
                "artifacts_involved": contradiction.artifacts_involved,
                "resolution_hint": contradiction.resolution_hint,
            },
        ))

    # 3. Missing predicate chains → artifact_injection
    relations = graph_state.relations()
    relation_set: set[tuple[str, str, str]] = set()
    for r in relations:
        relation_set.add((r.source_label.lower(), r.target_label.lower(), r.predicate.lower()))

    for expected_pred in spec.predicates:
        for hop in expected_pred.chain:
            key = (hop.source.lower(), hop.target.lower(), hop.relation_type.lower())
            reverse_key = (hop.target.lower(), hop.source.lower(), hop.relation_type.lower())
            if key not in relation_set and reverse_key not in relation_set:
                proposals.append(ProposedMutation(
                    mutation_type="artifact_injection",
                    description=f"Missing predicate hop: {hop.source} -[{hop.relation_type}]-> {hop.target}",
                    target_entities=[hop.source, hop.target],
                    metadata={"evidence_hint": hop.evidence_hint},
                ))

    return proposals
