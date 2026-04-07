import pytest
from modelado.reasoning.pollock import PollockGraph, PollockEdgeType, PollockTargetType

def test_pollock_argument_chaining():
    """
    Verifies that Pollock relations can be chained and that undercuts 
    can target supporting links (D15, D16).
    """
    graph = PollockGraph()
    
    # 1. Base argument: A supports B
    rel_sup = graph.add_support("frag_evidence_1", "frag_claim_1", confidence=0.9)
    assert rel_sup.edge_type == PollockEdgeType.SUPPORTS
    assert rel_sup.target_type == PollockTargetType.FRAGMENT
    
    # 2. Rebuttal: C rebuts B
    rel_reb = graph.add_rebuttal("frag_counter_claim_1", "frag_claim_1", confidence=0.8)
    assert rel_reb.edge_type == PollockEdgeType.REBUTS
    
    # 3. Undercut: D undercuts the supporting link from A to B
    rel_und = graph.add_undercut("frag_bias_alert_1", rel_sup.relation_id, confidence=0.95)
    assert rel_und.edge_type == PollockEdgeType.UNDERCUTS
    assert rel_und.target_type == PollockTargetType.RELATION
    assert rel_und.target_id == rel_sup.relation_id
    
    # 4. Chaining: E supports A (the evidence)
    rel_chain = graph.add_support("frag_witness_1", "frag_evidence_1", confidence=1.0)
    assert rel_chain.target_id == "frag_evidence_1"
    
    # Verify incoming relations
    incoming_to_claim = graph.get_incoming("frag_claim_1")
    assert len(incoming_to_claim) == 2
    assert any(r.edge_type == PollockEdgeType.SUPPORTS for r in incoming_to_claim)
    assert any(r.edge_type == PollockEdgeType.REBUTS for r in incoming_to_claim)
    
    incoming_to_link = graph.get_incoming(rel_sup.relation_id, PollockTargetType.RELATION)
    assert len(incoming_to_link) == 1
    assert incoming_to_link[0].edge_type == PollockEdgeType.UNDERCUTS

if __name__ == "__main__":
    pytest.main([__file__])
