from ikam.inspection import InspectionRef, ResolveInspectionRequest
from modelado.hot_subgraph_store import InMemoryHotSubgraphStore
from modelado.inspection_runtime import HotInspectionResolver


def _resolve(payload: dict, hot_ref: str):
    store = InMemoryHotSubgraphStore()
    stored_ref = store.put(payload)

    assert stored_ref["type"] == "subgraph_ref"

    resolver = HotInspectionResolver(store)
    return resolver.resolve(
        ResolveInspectionRequest(
            inspection_ref=InspectionRef(
                backend="hot",
                locator={
                    "subgraph_ref": hot_ref,
                    "head_fragment_id": stored_ref["head_fragment_id"],
                },
            ),
            max_depth=1,
            include_payload=True,
            include_edges=True,
            include_provenance=True,
        )
    )


def test_resolve_document_set_hot_ref_into_inspection_subgraph() -> None:
    subgraph = _resolve(
        {
            "kind": "document_set",
            "artifact_head_ref": "artifact://doc-bundle",
            "subgraph_ref": "subgraph://run-1-document-set-step-load",
            "document_refs": ["frag-doc-1", "frag-doc-2"],
            "documents": [
                {
                    "cas_id": "frag-doc-1",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {
                        "document_id": "doc-1",
                        "filename": "doc-1.md",
                        "text": "# Doc 1",
                    },
                },
                {
                    "cas_id": "frag-doc-2",
                    "mime_type": "application/json",
                    "value": {"title": "Doc 2"},
                },
            ],
        },
        "subgraph://run-1-document-set-step-load",
    )

    root = next(node for node in subgraph.nodes if node.id == subgraph.root_node_id)
    member_ids = {node.id for node in subgraph.nodes if node.kind == "fragment"}
    edges = {(getattr(edge, "from"), edge.to, edge.relation) for edge in subgraph.edges}
    member_ir_kinds = {node.payload.get("cas_id"): node.ir_kind for node in subgraph.nodes if node.kind == "fragment"}
    member_labels = {node.payload.get("cas_id"): node.label for node in subgraph.nodes if node.kind == "fragment"}

    assert subgraph.root_node_id == "subgraph:subgraph://run-1-document-set-step-load"
    assert root.payload["subgraph_ref"] == "subgraph://run-1-document-set-step-load"
    assert root.provenance["source_backend"] == "hot"
    assert member_ids == {"fragment:frag-doc-1", "fragment:frag-doc-2"}
    assert member_ir_kinds == {
        "frag-doc-1": "document",
        "frag-doc-2": "json",
    }
    assert member_labels == {
        "frag-doc-1": "doc-1.md",
        "frag-doc-2": "frag-doc-2",
    }
    assert edges == {
        ("subgraph:subgraph://run-1-document-set-step-load", "fragment:frag-doc-1", "contains"),
        ("subgraph:subgraph://run-1-document-set-step-load", "fragment:frag-doc-2", "contains"),
    }


def test_resolve_chunk_extraction_set_includes_source_subgraph_edge() -> None:
    subgraph = _resolve(
        {
            "kind": "chunk_extraction_set",
            "source_subgraph_ref": "subgraph://run-1-document-set-step-load",
            "subgraph_ref": "subgraph://run-1-chunk-extraction-set-step-parse",
            "extraction_refs": ["frag-chunk-1"],
            "documents": [
                {
                    "cas_id": "frag-doc-1",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {"document_id": "doc-1", "artifact_id": "artifact://doc-1", "filename": "doc-1.md", "text": "alpha"},
                }
            ],
            "extractions": [
                {
                    "cas_id": "frag-chunk-1",
                    "mime_type": "application/vnd.ikam.chunk+json",
                    "value": {"chunk_id": "doc-1:chunk:0", "text": "alpha", "span": [0, 5], "source_document_fragment_id": "frag-doc-1"},
                }
            ],
            "edges": [
                {
                    "from": "fragment:frag-chunk-1",
                    "to": "fragment:frag-doc-1",
                    "edge_label": "knowledge:derives",
                }
            ],
        },
        "subgraph://run-1-chunk-extraction-set-step-parse",
    )

    node_ids = {node.id for node in subgraph.nodes}
    edges = {(getattr(edge, "from"), edge.to, edge.relation) for edge in subgraph.edges}
    chunk_node = next(node for node in subgraph.nodes if node.id == "fragment:frag-chunk-1")

    assert subgraph.root_node_id == "subgraph:subgraph://run-1-chunk-extraction-set-step-parse"
    assert "subgraph:subgraph://run-1-document-set-step-load" in node_ids
    assert chunk_node.ir_kind == "chunk_extraction"
    assert chunk_node.provenance["source_backend"] == "hot"
    assert (
        "subgraph:subgraph://run-1-chunk-extraction-set-step-parse",
        "fragment:frag-chunk-1",
        "contains",
    ) in edges
    assert (
        "subgraph:subgraph://run-1-chunk-extraction-set-step-parse",
        "subgraph:subgraph://run-1-document-set-step-load",
        "source_subgraph",
    ) in edges
    assert "fragment:frag-doc-1" in node_ids
    assert (
        "fragment:frag-chunk-1",
        "fragment:frag-doc-1",
        "derives",
    ) in edges
    assert chunk_node.label == "doc-1:chunk:0"


def test_resolve_entity_relationship_set_marks_relationship_nodes() -> None:
    subgraph = _resolve(
        {
            "kind": "entity_relationship_set",
            "source_subgraph_ref": "subgraph://run-1-chunk-extraction-set-step-parse",
            "subgraph_ref": "subgraph://run-1-entity-relationship-set-step-entities",
            "entity_relationship_refs": ["frag-rel-1"],
            "entity_relationships": [
                {
                    "cas_id": "frag-rel-1",
                    "mime_type": "application/vnd.ikam.entity-relationship+json",
                    "value": {"source": "A", "target": "B", "relationship": "supports"},
                }
            ],
        },
        "subgraph://run-1-entity-relationship-set-step-entities",
    )

    relationship_node = next(node for node in subgraph.nodes if node.id == "fragment:frag-rel-1")
    edges = {(getattr(edge, "from"), edge.to, edge.relation) for edge in subgraph.edges}

    assert relationship_node.ir_kind == "entity_relationship"
    assert relationship_node.provenance["source_backend"] == "hot"
    assert (
        "subgraph:subgraph://run-1-entity-relationship-set-step-entities",
        "fragment:frag-rel-1",
        "contains",
    ) in edges
    assert (
        "subgraph:subgraph://run-1-entity-relationship-set-step-entities",
        "subgraph:subgraph://run-1-chunk-extraction-set-step-parse",
        "source_subgraph",
    ) in edges


def test_resolve_claim_set_marks_claim_nodes() -> None:
    subgraph = _resolve(
        {
            "kind": "claim_set",
            "source_subgraph_ref": "subgraph://run-1-entity-relationship-set-step-entities",
            "subgraph_ref": "subgraph://run-1-claim-set-step-claims",
            "claim_refs": ["frag-claim-1"],
            "claims": [
                {
                    "cas_id": "frag-claim-1",
                    "mime_type": "application/vnd.ikam.claim+json",
                    "value": {"claim": "A implies B", "confidence": 0.91},
                }
            ],
        },
        "subgraph://run-1-claim-set-step-claims",
    )

    claim_node = next(node for node in subgraph.nodes if node.id == "fragment:frag-claim-1")
    edges = {(getattr(edge, "from"), edge.to, edge.relation) for edge in subgraph.edges}

    assert claim_node.ir_kind == "claim"
    assert claim_node.provenance["source_backend"] == "hot"
    assert (
        "subgraph:subgraph://run-1-claim-set-step-claims",
        "fragment:frag-claim-1",
        "contains",
    ) in edges
    assert (
        "subgraph:subgraph://run-1-claim-set-step-claims",
        "subgraph:subgraph://run-1-entity-relationship-set-step-entities",
        "source_subgraph",
    ) in edges


def test_resolve_chunk_extraction_set_keeps_document_chunk_groupings_addressable() -> None:
    subgraph = _resolve(
        {
            "kind": "chunk_extraction_set",
            "source_subgraph_ref": "subgraph://run-1-document-set-step-load",
            "subgraph_ref": "subgraph://run-1-chunk-extraction-set-step-parse",
            "extraction_refs": ["frag-chunk-1"],
            "documents": [
                {
                    "cas_id": "frag-doc-1",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {"document_id": "doc-1", "artifact_id": "artifact://doc-1", "filename": "doc-1.md", "text": "alpha"},
                }
            ],
            "extractions": [
                {
                    "cas_id": "frag-chunk-1",
                    "mime_type": "application/vnd.ikam.chunk+json",
                    "value": {"chunk_id": "doc-1:chunk:0", "text": "alpha", "span": [0, 5], "source_document_fragment_id": "frag-doc-1"},
                }
            ],
            "document_chunk_sets": [
                {
                    "cas_id": "frag-doc-chunks-1",
                    "mime_type": "application/vnd.ikam.document-chunk-set+json",
                    "value": {
                        "kind": "document_chunk_set",
                        "document_id": "doc-1",
                        "source_document_fragment_id": "frag-doc-1",
                        "chunk_refs": ["frag-chunk-1"],
                    },
                }
            ],
        },
        "subgraph://run-1-chunk-extraction-set-step-parse",
    )

    node_ids = {node.id for node in subgraph.nodes}

    assert "fragment:frag-doc-chunks-1" in node_ids


def test_resolve_hot_subgraph_canonicalizes_explicit_locator_refs() -> None:
    subgraph = _resolve(
        {
            "kind": "document_set",
            "artifact_head_ref": "artifact://doc-bundle",
            "subgraph_ref": "subgraph://run-1-document-set-step-load",
            "document_refs": ["frag-doc-1"],
            "documents": [
                {
                    "cas_id": "frag-doc-1",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {
                        "document_id": "doc-1",
                        "filename": "doc-1.md",
                        "text": "# Doc 1",
                    },
                }
            ],
        },
        "ref://refs/heads/main/subgraph/run-1-document-set-step-load",
    )

    root = next(node for node in subgraph.nodes if node.id == subgraph.root_node_id)

    assert subgraph.root_node_id == "subgraph:ref://refs/heads/main/subgraph/run-1-document-set-step-load"
    assert root.refs["self"].locator["subgraph_ref"] == "ref://refs/heads/main/subgraph/run-1-document-set-step-load"
