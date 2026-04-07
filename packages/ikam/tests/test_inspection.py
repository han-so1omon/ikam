from ikam import (
    InspectionEdge,
    InspectionNode,
    InspectionRef,
    InspectionResolver,
    ResolveInspectionRequest,
    InspectionSubgraph,
)
from ikam.inspection import (
    edge_id_for,
    node_id_for,
)


def test_inspection_node_exposes_canonical_fields_and_defaults():
    node = InspectionNode(
        id="node:root",
        kind="fragment",
        label="Root fragment",
    )

    assert node.id == "node:root"
    assert node.kind == "fragment"
    assert node.ir_kind is None
    assert node.label == "Root fragment"
    assert node.summary is None
    assert node.payload == {}
    assert node.preview is None
    assert node.refs == {}
    assert node.provenance == {}
    assert node.capabilities == {}
    assert node.children == {}


def test_inspection_ref_is_backend_aware_opaque_shape():
    ref = InspectionRef(backend="cas", locator={"address": "sha256:abc"})

    assert ref.backend == "cas"
    assert ref.locator == {"address": "sha256:abc"}
    assert ref.hint is None


def test_inspection_ref_parses_canonical_inspect_refs():
    subgraph_ref = InspectionRef.parse("inspect://subgraph/sg:root")
    fragment_ref = InspectionRef.parse("inspect://fragment/sha256:abc")
    artifact_ref = InspectionRef.parse("inspect://artifact/art-123")
    edge_ref = InspectionRef.parse("inspect://edge/edge-123")
    context_ref = InspectionRef.parse("inspect://context/story:intro")

    assert subgraph_ref.backend == "inspect"
    assert subgraph_ref.locator == {"category": "subgraph", "subgraph_ref": "sg:root"}
    assert fragment_ref.locator == {"category": "fragment", "cas_id": "sha256:abc"}
    assert artifact_ref.locator == {"category": "artifact", "artifact_id": "art-123"}
    assert edge_ref.locator == {"category": "edge", "edge_id": "edge-123"}
    assert context_ref.locator == {
        "category": "context",
        "context_anchor": "story:intro",
    }


def test_inspection_ref_rejects_unknown_canonical_category():
    try:
        InspectionRef.parse("inspect://unknown/value")
    except ValueError as exc:
        assert str(exc) == "Unsupported inspection ref category: unknown"
    else:
        raise AssertionError("Expected ValueError for unknown canonical category")


def test_node_id_for_uses_canonical_identity_fields():
    assert node_id_for("fragment", {"cas_id": "sha256:abc", "artifact_id": "art-1"}) == "fragment:sha256:abc"
    assert node_id_for("artifact", {"artifact_id": "art-1", "cas_id": "sha256:abc"}) == "artifact:art-1"
    assert node_id_for("subgraph", {"subgraph_ref": "sg:root", "artifact_id": "art-1"}) == "subgraph:sg:root"


def test_edge_id_for_is_deterministic_from_relation_and_endpoints():
    assert edge_id_for("contains", "fragment:sha256:abc", "artifact:art-1") == (
        "edge:contains:fragment:sha256:abc:artifact:art-1"
    )
    assert edge_id_for("contains", "fragment:sha256:abc", "artifact:art-1") == (
        edge_id_for("contains", "fragment:sha256:abc", "artifact:art-1")
    )


def test_inspection_edge_accepts_source_subgraph_relation():
    edge = InspectionEdge(
        id="edge:source_subgraph",
        **{"from": "subgraph:child", "to": "subgraph:parent"},
        relation="source_subgraph",
    )

    assert edge.relation == "source_subgraph"


def test_inspection_edge_exposes_canonical_fields_and_defaults():
    edge = InspectionEdge(
        id="edge:contains",
        **{"from": "node:root", "to": "node:child"},
        relation="contains",
    )

    assert edge.id == "edge:contains"
    assert getattr(edge, "from") == "node:root"
    assert getattr(edge, "to") == "node:child"
    assert edge.relation == "contains"
    assert edge.label is None
    assert edge.summary is None
    assert edge.payload == {}
    assert edge.refs == {}
    assert edge.provenance == {}
    assert edge.capabilities == {}


def test_inspection_subgraph_collects_nodes_edges_and_navigation():
    ref = InspectionRef(backend="cas", locator={"address": "sha256:child"})
    node = InspectionNode(
        id="node:root",
        kind="fragment",
        label="Root fragment",
        refs={"primary": ref},
        capabilities={"inspect": True},
        children={"contains": ["node:child"]},
    )
    edge = InspectionEdge(
        id="edge:contains",
        **{"from": "node:root", "to": "node:child"},
        relation="contains",
        refs={"evidence": ref},
        capabilities={"traverse": True},
    )
    subgraph = InspectionSubgraph(
        schema_version="v1",
        root_node_id="node:root",
        nodes=[node],
        edges=[edge],
        navigation={"focus": {"node_id": "node:root"}},
    )

    assert subgraph.schema_version == "v1"
    assert subgraph.root_node_id == "node:root"
    assert subgraph.nodes == [node]
    assert subgraph.edges == [edge]
    assert subgraph.navigation == {"focus": {"node_id": "node:root"}}
    assert subgraph.nodes[0].refs["primary"] == ref
    assert subgraph.nodes[0].capabilities == {"inspect": True}
    assert subgraph.nodes[0].children == {"contains": ["node:child"]}
    assert subgraph.edges[0].refs["evidence"] == ref
    assert subgraph.edges[0].capabilities == {"traverse": True}


def test_inspection_types_are_exported_from_ikam_package():
    assert InspectionRef.__name__ == "InspectionRef"
    assert InspectionNode.__name__ == "InspectionNode"
    assert InspectionEdge.__name__ == "InspectionEdge"
    assert ResolveInspectionRequest.__name__ == "ResolveInspectionRequest"
    assert InspectionResolver.__name__ == "InspectionResolver"
    assert InspectionSubgraph.__name__ == "InspectionSubgraph"


def test_resolve_inspection_request_exposes_backend_agnostic_options():
    request = ResolveInspectionRequest(
        inspection_ref=InspectionRef(backend="cas", locator={"address": "sha256:abc"}),
        max_depth=2,
        include_payload=True,
        include_edges=False,
        include_provenance=True,
    )

    assert request.inspection_ref == InspectionRef(
        backend="cas", locator={"address": "sha256:abc"}
    )
    assert request.max_depth == 2
    assert request.include_payload is True
    assert request.include_edges is False
    assert request.include_provenance is True


def test_inspection_resolver_protocol_accepts_backend_agnostic_resolvers():
    class InlineResolver:
        def resolve(
            self,
            request: ResolveInspectionRequest,
        ) -> InspectionSubgraph:
            return InspectionSubgraph(
                schema_version="v1",
                root_node_id="node:root",
            )

    resolver = InlineResolver()

    assert isinstance(resolver, InspectionResolver)
    assert resolver.resolve(
        ResolveInspectionRequest(
            inspection_ref=InspectionRef(backend="cas", locator={"address": "sha256:abc"}),
            max_depth=1,
            include_payload=False,
            include_edges=True,
            include_provenance=False,
        )
    ) == InspectionSubgraph(schema_version="v1", root_node_id="node:root")
