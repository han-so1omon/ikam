from ikam.fragments import Fragment
from ikam.inspection import InspectionRef, ResolveInspectionRequest

from modelado.hot_subgraph_store import InMemoryHotSubgraphStore
from modelado.inspection_runtime import HotInspectionResolver, PersistentInspectionResolver
from modelado.oraculo.persistent_graph_state import PersistentGraphState


def _resolve_hot(payload: dict, subgraph_ref: str):
    store = InMemoryHotSubgraphStore()
    stored_ref = store.put(payload)
    resolver = HotInspectionResolver(store)
    return resolver.resolve(
        ResolveInspectionRequest(
            inspection_ref=InspectionRef(
                backend="hot",
                locator={
                    "subgraph_ref": subgraph_ref,
                    "head_fragment_id": stored_ref["head_fragment_id"],
                },
            ),
            max_depth=1,
            include_payload=True,
            include_edges=True,
            include_provenance=True,
        )
    )


def _resolve_persistent(state: PersistentGraphState, subgraph_ref: str):
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


def _parity_projection(subgraph) -> dict:
    return {
        "root_node_id": subgraph.root_node_id,
        "nodes": {
            node.id: {
                "kind": node.kind,
                "ir_kind": node.ir_kind,
            }
            for node in subgraph.nodes
        },
        "edges": {
            (getattr(edge, "from"), edge.to, edge.relation)
            for edge in subgraph.edges
        },
    }


def test_document_set_inspection_matches_across_hot_and_persistent_backends() -> None:
    subgraph_ref = "shared://inspection/document-set"
    hot_subgraph = _resolve_hot(
        {
            "kind": "document_set",
            "subgraph_ref": subgraph_ref,
            "document_refs": ["frag-doc-1", "frag-doc-2"],
            "documents": [
                {
                    "cas_id": "frag-doc-1",
                    "mime_type": "text/markdown",
                    "value": "# Doc 1",
                },
                {
                    "cas_id": "frag-doc-2",
                    "mime_type": "application/json",
                    "value": {"title": "Doc 2"},
                },
            ],
        },
        subgraph_ref,
    )

    state = PersistentGraphState()
    state.add_fragment(Fragment(cas_id="frag-doc-1", mime_type="text/markdown", value="# Doc 1"))
    state.add_fragment(Fragment(cas_id="frag-doc-2", mime_type="application/json", value={"title": "Doc 2"}))
    state.register_inspection_subgraph(
        {
            "kind": "document_set",
            "subgraph_ref": subgraph_ref,
            "member_refs": ["frag-doc-1", "frag-doc-2"],
            "edges": [
                {"to": "frag-doc-1", "edge_label": "knowledge:contains"},
                {"to": "frag-doc-2", "edge_label": "knowledge:contains"},
            ],
        }
    )
    persistent_subgraph = _resolve_persistent(state, subgraph_ref)

    assert _parity_projection(persistent_subgraph) == _parity_projection(hot_subgraph)


def test_chunk_set_inspection_matches_hot_parity_after_relation_normalization() -> None:
    document_subgraph_ref = "shared://inspection/document-set"
    chunk_subgraph_ref = "shared://inspection/chunk-set"
    hot_subgraph = _resolve_hot(
        {
            "kind": "chunk_extraction_set",
            "subgraph_ref": chunk_subgraph_ref,
            "source_subgraph_ref": document_subgraph_ref,
            "extraction_refs": ["frag-chunk-1"],
            "extractions": [
                {
                    "cas_id": "frag-chunk-1",
                    "mime_type": "application/vnd.ikam.chunk+json",
                    "value": {"text": "alpha", "span": [0, 5]},
                }
            ],
        },
        chunk_subgraph_ref,
    )

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
            "subgraph_ref": chunk_subgraph_ref,
            "member_refs": ["frag-chunk-1"],
            "edges": [
                {"to": "frag-chunk-1", "edge_label": "knowledge:contains"},
                {
                    "to_subgraph_ref": document_subgraph_ref,
                    "edge_label": "knowledge:source-subgraph",
                },
            ],
        }
    )
    persistent_subgraph = _resolve_persistent(state, chunk_subgraph_ref)

    assert _parity_projection(persistent_subgraph) == _parity_projection(hot_subgraph)
