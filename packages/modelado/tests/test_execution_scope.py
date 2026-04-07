from __future__ import annotations

from modelado.core.execution_scope import _expand_debug_pipeline_steps


def test_expand_debug_pipeline_steps_expands_minimal_net_to_canonical_debug_pipeline() -> None:
    expanded = _expand_debug_pipeline_steps(["init.initialize", "parse_artifacts", "lift_fragments"])

    assert expanded == [
        "init.initialize",
        "map.conceptual.lift.surface_fragments",
        "map.conceptual.lift.entities_and_relationships",
        "map.conceptual.lift.claims",
        "map.conceptual.lift.summarize",
        "map.conceptual.embed.discovery_index",
        "map.conceptual.normalize.discovery",
        "map.reconstructable.embed",
        "map.reconstructable.search.dependency_resolution",
        "map.reconstructable.normalize",
        "map.reconstructable.compose.reconstruction_programs",
        "map.conceptual.verify.discovery_gate",
        "map.conceptual.commit.semantic_only",
        "map.reconstructable.build_subgraph.reconstruction",
    ]
