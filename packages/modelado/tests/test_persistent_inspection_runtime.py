from ikam.fragments import Fragment
from ikam.inspection import InspectionRef, ResolveInspectionRequest

from modelado.inspection_runtime import PersistentInspectionResolver
from modelado.oraculo.persistent_graph_state import PersistentGraphState


def _resolve(state: PersistentGraphState, subgraph_ref: str):
    resolver = PersistentInspectionResolver(state)
    return resolver.resolve(
        ResolveInspectionRequest(
            inspection_ref=InspectionRef(
                backend="persistent",
                locator={"subgraph_ref": subgraph_ref},
            ),
            max_depth=1,
            include_payload=True,
            include_edges=True,
            include_provenance=True,
        )
    )


def test_resolve_persistent_document_set_emits_canonical_contains_edges() -> None:
    state = PersistentGraphState()
    state.add_fragment(Fragment(cas_id="frag-doc-1", mime_type="text/markdown", value="# Doc 1"))
    state.add_fragment(Fragment(cas_id="frag-doc-2", mime_type="application/json", value={"title": "Doc 2"}))
    state.register_inspection_subgraph(
        {
            "kind": "document_set",
            "subgraph_ref": "refs/heads/main/subgraphs/document-set",
            "member_refs": ["frag-doc-1", "frag-doc-2"],
            "edges": [
                {"to": "frag-doc-1", "edge_label": "knowledge:contains"},
                {"to": "frag-doc-2", "edge_label": "knowledge:contains"},
            ],
        }
    )

    subgraph = _resolve(state, "refs/heads/main/subgraphs/document-set")

    root = next(node for node in subgraph.nodes if node.id == subgraph.root_node_id)
    member_ir_kinds = {node.id: node.ir_kind for node in subgraph.nodes if node.kind == "fragment"}
    edges = {(getattr(edge, "from"), edge.to, edge.relation) for edge in subgraph.edges}

    assert subgraph.root_node_id == "subgraph:refs/heads/main/subgraphs/document-set"
    assert root.refs["self"] == InspectionRef(
        backend="persistent",
        locator={"subgraph_ref": "refs/heads/main/subgraphs/document-set"},
    )
    assert root.provenance["source_backend"] == "persistent"
    assert member_ir_kinds == {
        "fragment:frag-doc-1": "document",
        "fragment:frag-doc-2": "json",
    }
    assert edges == {
        (
            "subgraph:refs/heads/main/subgraphs/document-set",
            "fragment:frag-doc-1",
            "contains",
        ),
        (
            "subgraph:refs/heads/main/subgraphs/document-set",
            "fragment:frag-doc-2",
            "contains",
        ),
    }


def test_resolve_persistent_chunk_set_normalizes_source_subgraph_edge() -> None:
    state = PersistentGraphState()
    state.add_fragment(
        Fragment(
            cas_id="frag-chunk-1",
            mime_type="application/vnd.ikam.chunk+json",
            value={"text": "alpha", "span": [0, 5]},
        )
    )
    state.register_inspection_subgraph(
        {
            "kind": "chunk_extraction_set",
            "subgraph_ref": "refs/heads/main/subgraphs/chunk-set",
            "member_refs": ["frag-chunk-1"],
            "edges": [
                {"to": "frag-chunk-1", "edge_label": "knowledge:contains"},
                {
                    "to_subgraph_ref": "refs/heads/main/subgraphs/document-set",
                    "edge_label": "knowledge:source_subgraph",
                },
            ],
        }
    )

    subgraph = _resolve(state, "refs/heads/main/subgraphs/chunk-set")

    node_ids = {node.id for node in subgraph.nodes}
    edges = {(getattr(edge, "from"), edge.to, edge.relation) for edge in subgraph.edges}
    chunk_node = next(node for node in subgraph.nodes if node.id == "fragment:frag-chunk-1")

    assert subgraph.root_node_id == "subgraph:refs/heads/main/subgraphs/chunk-set"
    assert "subgraph:refs/heads/main/subgraphs/document-set" in node_ids
    assert chunk_node.ir_kind == "chunk_extraction"
    assert chunk_node.provenance["source_backend"] == "persistent"
    assert (
        "subgraph:refs/heads/main/subgraphs/chunk-set",
        "fragment:frag-chunk-1",
        "contains",
    ) in edges
    assert (
        "subgraph:refs/heads/main/subgraphs/chunk-set",
        "subgraph:refs/heads/main/subgraphs/document-set",
        "source_subgraph",
    ) in edges
