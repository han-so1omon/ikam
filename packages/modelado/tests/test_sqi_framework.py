import pytest
from ikam.ir import PropositionIR, EvidenceRef
from modelado.profiles.reasoning import PollockClaim, ReasoningProfileV1, REASONING_V1
from modelado.reasoning.query import SemanticQuery, InterpretationContext, QueryConstraints, SearchStrategy
from modelado.reasoning.registry import get_search_strategy_registry
from modelado.environment_scope import EnvironmentScope

def test_proposition_ir_profile_aware():
    prop = PropositionIR(
        artifact_id="art_1",
        profile=REASONING_V1,
        statement={
            "subject": "Revenue",
            "predicate": "increases_by",
            "object": "20%",
            "context": {"unit": "percentage"}
        },
        evidence_refs=[EvidenceRef(fragment_id="frag_1")]
    )
    assert prop.profile == REASONING_V1
    assert prop.statement["subject"] == "Revenue"

def test_pollock_profile_validation():
    payload = {
        "subject": "Revenue",
        "predicate": "increases_by",
        "object": "20%",
        "context": {"unit": "percentage"}
    }
    claim = ReasoningProfileV1.validate(payload)
    assert isinstance(claim, PollockClaim)
    assert claim.subject == "Revenue"

def test_pollock_profile_interpretation():
    claim = PollockClaim(
        subject="Revenue",
        predicate="increases_by",
        object="20%",
        context={"scenario": "base"}
    )
    prose = ReasoningProfileV1.interpret(claim)
    assert prose == "Revenue increases by 20%"
    
    template = "{subject} {predicate} {object} in the {scenario} scenario"
    prose_templated = ReasoningProfileV1.interpret(claim, template=template)
    assert prose_templated == "Revenue increases_by 20% in the base scenario"

def test_semantic_query_structure():
    query = SemanticQuery(
        intent="Explain the revenue growth",
        interpretation=InterpretationContext(
            directives=["simple language", "in Spanish"]
        ),
        constraints=QueryConstraints(
            env_scope=EnvironmentScope(ref="refs/heads/run/run_1"),
            node_types=["Proposition"]
        )
    )
    assert query.intent == "Explain the revenue growth"
    assert "simple language" in query.interpretation.directives
    assert query.constraints.env_scope.ref == "refs/heads/run/run_1"

def mock_strategy(constraints):
    return {"nodes": [], "edges": []}

def test_search_strategy_registry(db_connection):
    reg = get_search_strategy_registry(db_connection)
    
    reg.register("mock", mock_strategy)
    assert "mock" in reg
    assert reg.get("mock") == mock_strategy

import json
from unittest.mock import MagicMock, AsyncMock, patch
from modelado.reasoning.strategies import bfs_search
from modelado.reasoning.synthesizer import SynthesizerService
from modelado.reasoning.query import Subgraph
from modelado.graph_edge_event_log import GraphEdgeEvent


def _mock_reasoning_node(node_id: str) -> PropositionIR:
    return PropositionIR(
        artifact_id="proj_1",
        fragment_id=node_id,
        profile=REASONING_V1,
        statement={"subject": "Mock"},
        evidence_refs=[EvidenceRef(fragment_id=node_id)],
    )


def _graph_edge(*, edge_id: int, in_id: str, properties: dict) -> GraphEdgeEvent:
    return GraphEdgeEvent(
        id=edge_id,
        project_id="proj_1",
        op="upsert",
        out_id="frag_1",
        in_id=in_id,
        edge_label="knowledge:ref",
        properties=properties,
        t=100 + edge_id,
        idempotency_key=None,
    )

def test_bfs_search_isolation_d19():
    """Verify D19: unauthorized cross-scope events fail fast."""
    cx = MagicMock()
    constraints = QueryConstraints(
        env_scope=EnvironmentScope(ref="refs/heads/run/run_1"),
        extra_filters={"anchor_ids": ["frag_1"], "project_id": "proj_1"}
    )
    
    # Mock edges: one matching dev, one from a different dev run, one committed
    edge_dev = GraphEdgeEvent(
        id=1, project_id="proj_1", op="upsert", out_id="frag_1", in_id="frag_2", edge_label="knowledge:ref",
        properties={"ref": "refs/heads/run/run_1"}, t=100, idempotency_key=None
    )
    edge_other_dev = GraphEdgeEvent(
        id=2, project_id="proj_1", op="upsert", out_id="frag_1", in_id="frag_3", edge_label="knowledge:ref",
        properties={"ref": "refs/heads/run/run_2"}, t=101, idempotency_key=None
    )
    edge_committed = GraphEdgeEvent(
        id=3, project_id="proj_1", op="upsert", out_id="frag_1", in_id="frag_4", edge_label="knowledge:ref",
        properties={"ref": "refs/heads/main"}, t=102, idempotency_key=None
    )
    
    with patch("modelado.reasoning.strategies.list_graph_edge_events") as mock_list_edges, \
         patch("modelado.reasoning.strategies._hydrate_ir") as mock_hydrate:
        
        mock_list_edges.return_value = [edge_dev, edge_other_dev, edge_committed]
        # Return a real IR node
        def mock_hydrate_fn(node_id, cx):
            return PropositionIR(
                artifact_id="proj_1",
                fragment_id=node_id,
                profile=REASONING_V1,
                statement={"subject": "Mock"},
                evidence_refs=[EvidenceRef(fragment_id=node_id)]
            )
            
        mock_hydrate.side_effect = mock_hydrate_fn
        
        with pytest.raises(ValueError, match="Unauthorized scope traversal"):
            bfs_search(cx, constraints)

def test_bfs_search_isolation_strict_committed():
    """Verify D19: unauthorized scope encounters fail fast."""
    cx = MagicMock()
    constraints = QueryConstraints(
        env_scope=EnvironmentScope(ref="refs/heads/main"),
        extra_filters={"anchor_ids": ["frag_1"], "project_id": "proj_1"}
    )
    
    edge_dev = GraphEdgeEvent(
        id=1, project_id="proj_1", op="upsert", out_id="frag_1", in_id="frag_2", edge_label="knowledge:ref",
        properties={"ref": "refs/heads/run/run_1"}, t=100, idempotency_key=None
    )
    edge_committed = GraphEdgeEvent(
        id=3, project_id="proj_1", op="upsert", out_id="frag_1", in_id="frag_4", edge_label="knowledge:ref",
        properties={"ref": "refs/heads/main"}, t=102, idempotency_key=None
    )
    
    with patch("modelado.reasoning.strategies.list_graph_edge_events") as mock_list_edges, \
         patch("modelado.reasoning.strategies._hydrate_ir") as mock_hydrate:
        
        mock_list_edges.return_value = [edge_dev, edge_committed]
        def mock_hydrate_fn(node_id, cx):
            return PropositionIR(
                artifact_id="proj_1",
                fragment_id=node_id,
                profile=REASONING_V1,
                statement={"subject": "Mock"},
                evidence_refs=[EvidenceRef(fragment_id=node_id)]
            )
        mock_hydrate.side_effect = mock_hydrate_fn
        
        with pytest.raises(ValueError, match="Unauthorized scope traversal"):
            bfs_search(cx, constraints)


def test_query_constraints_accept_base_refs():
    constraints = QueryConstraints(
        env_scope=EnvironmentScope(ref="refs/heads/run/run-1"),
        base_refs=["refs/heads/main"],
    )

    assert constraints.base_refs == ["refs/heads/main"]


def test_bfs_search_allows_same_ref_traversal():
    cx = MagicMock()
    constraints = QueryConstraints(
        env_scope=EnvironmentScope(ref="refs/heads/run/run-1"),
        extra_filters={"anchor_ids": ["frag_1"], "project_id": "proj_1"},
    )

    with patch("modelado.reasoning.strategies.list_graph_edge_events") as mock_list_edges, \
         patch("modelado.reasoning.strategies._hydrate_ir") as mock_hydrate:
        mock_list_edges.return_value = [
            _graph_edge(
                edge_id=1,
                in_id="frag_2",
                properties={"ref": "refs/heads/run/run-1"},
            )
        ]
        mock_hydrate.side_effect = lambda node_id, _cx: _mock_reasoning_node(node_id)

        result = bfs_search(cx, constraints)

    assert {node.fragment_id for node in result.nodes} == {"frag_1", "frag_2"}


def test_bfs_search_allows_explicit_base_ref_traversal():
    cx = MagicMock()
    constraints = QueryConstraints(
        env_scope=EnvironmentScope(ref="refs/heads/run/run-1"),
        base_refs=["refs/heads/main"],
        extra_filters={"anchor_ids": ["frag_1"], "project_id": "proj_1"},
    )

    with patch("modelado.reasoning.strategies.list_graph_edge_events") as mock_list_edges, \
         patch("modelado.reasoning.strategies._hydrate_ir") as mock_hydrate:
        mock_list_edges.return_value = [
            _graph_edge(
                edge_id=1,
                in_id="frag_2",
                properties={"ref": "refs/heads/main"},
            )
        ]
        mock_hydrate.side_effect = lambda node_id, _cx: _mock_reasoning_node(node_id)

        result = bfs_search(cx, constraints)

    assert {node.fragment_id for node in result.nodes} == {"frag_1", "frag_2"}


def test_bfs_search_rejects_unrelated_ref_by_ref_relationship():
    cx = MagicMock()
    constraints = QueryConstraints(
        env_scope=EnvironmentScope(ref="refs/heads/run/run-1"),
        extra_filters={"anchor_ids": ["frag_1"], "project_id": "proj_1"},
    )

    with patch("modelado.reasoning.strategies.list_graph_edge_events") as mock_list_edges, \
         patch("modelado.reasoning.strategies._hydrate_ir") as mock_hydrate:
        mock_list_edges.return_value = [
            _graph_edge(
                edge_id=1,
                in_id="frag_2",
                properties={"ref": "refs/heads/run/run-2"},
            )
        ]
        mock_hydrate.side_effect = lambda node_id, _cx: _mock_reasoning_node(node_id)

        with pytest.raises(
            ValueError,
            match="Unauthorized scope traversal from ref refs/heads/run/run-1 to refs/heads/run/run-2",
        ):
            bfs_search(cx, constraints)


def test_bfs_search_rejects_legacy_scope_qualifiers_without_ref() -> None:
    cx = MagicMock()
    constraints = QueryConstraints(
        env_scope=EnvironmentScope(ref="refs/heads/run/run-1"),
        extra_filters={"anchor_ids": ["frag_1"], "project_id": "proj_1"},
    )

    with patch("modelado.reasoning.strategies.list_graph_edge_events") as mock_list_edges, \
         patch("modelado.reasoning.strategies._hydrate_ir") as mock_hydrate:
        mock_list_edges.return_value = [
            _graph_edge(
                edge_id=1,
                in_id="frag_2",
                properties={"envType": "dev", "envId": "run-1"},
            )
        ]
        mock_hydrate.side_effect = lambda node_id, _cx: _mock_reasoning_node(node_id)

        with pytest.raises(ValueError, match="Graph edge event missing canonical ref qualifier"):
            bfs_search(cx, constraints)

@pytest.mark.anyio
async def test_synthesizer_attribution_d18():
    """Verify D18: Synthesis includes fragment citations."""
    ai_client = MagicMock()
    # Mock LLM response with citation
    ai_client.generate = AsyncMock()
    ai_client.generate.return_value = MagicMock(
        text=json.dumps({
            "interpretation": "Revenue growth was 20% [frag_1].",
            "attribution": [{"claim": "Revenue growth", "fragment_ids": ["frag_1"]}]
        })
    )
    
    synthesizer = SynthesizerService(ai_client)
    
    query = SemanticQuery(
        intent="Explain growth",
        interpretation=InterpretationContext(directives=["be concise"]),
        interpretation_context={} # default factory
    )
    
    node = PropositionIR(
        artifact_id="proj_1",
        fragment_id="frag_1",
        profile=REASONING_V1,
        statement={"subject": "Mock"},
        evidence_refs=[EvidenceRef(fragment_id="frag_1")]
    )
    
    subgraph = Subgraph(nodes=[node])
    
    result = await synthesizer.synthesize(query, subgraph)
    
    assert "frag_1" in result["interpretation"]
    assert result["attribution"][0]["fragment_ids"] == ["frag_1"]


@pytest.mark.anyio
async def test_synthesizer_rejects_missing_bracketed_citations():
    """Verify D18 hard gate rejects interpretation without [fragment_id] citations."""
    ai_client = MagicMock()
    ai_client.generate = AsyncMock()
    ai_client.generate.return_value = MagicMock(
        text=json.dumps(
            {
                "interpretation": "The revenue grew by 20 percent.",
                "attribution": [{"claim": "Revenue growth", "fragment_ids": ["frag_1"]}],
            }
        )
    )

    synthesizer = SynthesizerService(ai_client)
    query = SemanticQuery(
        intent="Explain growth",
        interpretation=InterpretationContext(directives=["be concise"]),
        interpretation_context={},
    )
    node = PropositionIR(
        artifact_id="proj_1",
        fragment_id="frag_1",
        profile=REASONING_V1,
        statement={"subject": "Mock"},
        evidence_refs=[EvidenceRef(fragment_id="frag_1")],
    )
    subgraph = Subgraph(nodes=[node])

    with pytest.raises(ValueError, match="Missing bracketed citations"):
        await synthesizer.synthesize(query, subgraph)


@pytest.mark.anyio
async def test_synthesizer_rejects_unknown_attribution_fragments():
    """Verify D18 hard gate rejects attribution IDs outside discovered subgraph."""
    ai_client = MagicMock()
    ai_client.generate = AsyncMock()
    ai_client.generate.return_value = MagicMock(
        text=json.dumps(
            {
                "interpretation": "The revenue grew by 20% [frag_2].",
                "attribution": [{"claim": "revenue grew", "fragment_ids": ["frag_2"]}],
            }
        )
    )

    synthesizer = SynthesizerService(ai_client)
    query = SemanticQuery(
        intent="Explain growth",
        interpretation=InterpretationContext(directives=["be concise"]),
        interpretation_context={},
    )
    node = PropositionIR(
        artifact_id="proj_1",
        fragment_id="frag_1",
        profile=REASONING_V1,
        statement={"subject": "Mock"},
        evidence_refs=[EvidenceRef(fragment_id="frag_1")],
    )
    subgraph = Subgraph(nodes=[node])

    with pytest.raises(ValueError, match="unknown fragment_ids"):
        await synthesizer.synthesize(query, subgraph)


@pytest.mark.anyio
async def test_synthesizer_rejects_empty_attribution_block():
    """D18 hard gate: attribution must be non-empty and valid."""
    ai_client = MagicMock()
    ai_client.generate = AsyncMock()
    ai_client.generate.return_value = MagicMock(
        text=json.dumps(
            {
                "interpretation": "Revenue improved [frag_1].",
                "attribution": [],
            }
        )
    )

    synthesizer = SynthesizerService(ai_client)
    query = SemanticQuery(
        intent="Explain growth",
        interpretation=InterpretationContext(directives=["be concise"]),
        interpretation_context={},
    )
    node = PropositionIR(
        artifact_id="proj_1",
        fragment_id="frag_1",
        profile=REASONING_V1,
        statement={"subject": "Mock"},
        evidence_refs=[EvidenceRef(fragment_id="frag_1")],
    )
    subgraph = Subgraph(nodes=[node])

    with pytest.raises(ValueError, match="Attribution must not be empty"):
        await synthesizer.synthesize(query, subgraph)


@pytest.mark.anyio
async def test_synthesizer_rejects_missing_claim_field():
    ai_client = MagicMock()
    ai_client.generate = AsyncMock()
    ai_client.generate.return_value = MagicMock(
        text=json.dumps(
            {
                "interpretation": "Revenue improved [frag_1].",
                "attribution": [{"fragment_ids": ["frag_1"]}],
            }
        )
    )

    synthesizer = SynthesizerService(ai_client)
    query = SemanticQuery(
        intent="Explain growth",
        interpretation=InterpretationContext(directives=["be concise"]),
        interpretation_context={},
    )
    node = PropositionIR(
        artifact_id="proj_1",
        fragment_id="frag_1",
        profile=REASONING_V1,
        statement={"subject": "Mock"},
        evidence_refs=[EvidenceRef(fragment_id="frag_1")],
    )
    subgraph = Subgraph(nodes=[node])

    with pytest.raises(ValueError, match="claim"):
        await synthesizer.synthesize(query, subgraph)


@pytest.mark.anyio
async def test_synthesizer_rejects_claim_not_in_interpretation():
    ai_client = MagicMock()
    ai_client.generate = AsyncMock()
    ai_client.generate.return_value = MagicMock(
        text=json.dumps(
            {
                "interpretation": "Revenue improved [frag_1].",
                "attribution": [{"claim": "Margin improved", "fragment_ids": ["frag_1"]}],
            }
        )
    )

    synthesizer = SynthesizerService(ai_client)
    query = SemanticQuery(
        intent="Explain growth",
        interpretation=InterpretationContext(directives=["be concise"]),
        interpretation_context={},
    )
    node = PropositionIR(
        artifact_id="proj_1",
        fragment_id="frag_1",
        profile=REASONING_V1,
        statement={"subject": "Mock"},
        evidence_refs=[EvidenceRef(fragment_id="frag_1")],
    )
    subgraph = Subgraph(nodes=[node])

    with pytest.raises(ValueError, match="claim.*interpretation"):
        await synthesizer.synthesize(query, subgraph)


@pytest.mark.anyio
async def test_synthesizer_rejects_non_string_fragment_ids():
    ai_client = MagicMock()
    ai_client.generate = AsyncMock()
    ai_client.generate.return_value = MagicMock(
        text=json.dumps(
            {
                "interpretation": "Revenue improved [frag_1].",
                "attribution": [{"claim": "Revenue improved", "fragment_ids": [123]}],
            }
        )
    )

    synthesizer = SynthesizerService(ai_client)
    query = SemanticQuery(
        intent="Explain growth",
        interpretation=InterpretationContext(directives=["be concise"]),
        interpretation_context={},
    )
    node = PropositionIR(
        artifact_id="proj_1",
        fragment_id="frag_1",
        profile=REASONING_V1,
        statement={"subject": "Mock"},
        evidence_refs=[EvidenceRef(fragment_id="frag_1")],
    )
    subgraph = Subgraph(nodes=[node])

    with pytest.raises(ValueError, match="fragment_ids"):
        await synthesizer.synthesize(query, subgraph)


@pytest.mark.anyio
async def test_synthesizer_allows_semantic_claim_variation_with_overlap_tokens():
    ai_client = MagicMock()
    ai_client.generate = AsyncMock()
    ai_client.generate.return_value = MagicMock(
        text=json.dumps(
            {
                "interpretation": "Revenue grew by 20% [frag_1].",
                "attribution": [{"claim": "Revenue growth", "fragment_ids": ["frag_1"]}],
            }
        )
    )

    synthesizer = SynthesizerService(ai_client)
    query = SemanticQuery(
        intent="Explain growth",
        interpretation=InterpretationContext(directives=["be concise"]),
        interpretation_context={},
    )
    node = PropositionIR(
        artifact_id="proj_1",
        fragment_id="frag_1",
        profile=REASONING_V1,
        statement={"subject": "Mock"},
        evidence_refs=[EvidenceRef(fragment_id="frag_1")],
    )
    subgraph = Subgraph(nodes=[node])

    result = await synthesizer.synthesize(query, subgraph)
    assert result["attribution"][0]["claim"] == "Revenue growth"
