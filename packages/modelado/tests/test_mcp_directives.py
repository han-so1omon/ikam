import json
import io
import sys
import pytest
from modelado.mcp.directive_server import (
    handle_get_ir_directives,
    handle_match_proven_patterns,
    propose_rerender_subgraph,
    PROSE_BACKBONE_SCHEMA_ID,
    PHRASING_DELTA_SCHEMA_ID,
)

def test_get_ir_directives():
    """Verifies that get_ir_directives returns correct schemas and invariants."""
    params = {"kind": "prose"}
    result = handle_get_ir_directives(params)
    
    assert result["schema"] == PROSE_BACKBONE_SCHEMA_ID
    assert "proposition_ids" in result["examples"]["backbone"]
    assert "ProseBackbone must sequence Proposition IDs only" in result["invariants"]

def test_get_ir_directives_unknown():
    """Verifies error handling for unknown IR kinds."""
    params = {"kind": "unknown"}
    result = handle_get_ir_directives(params)
    assert "error" in result

def test_match_proven_patterns():
    """Verifies pattern matching logic (mocked for Stage 1)."""
    dna = {"fingerprint": "any-fingerprint"}
    params = {"dna": dna}
    result = handle_match_proven_patterns(params)
    
    assert len(result["matches"]) >= 1
    assert result["matches"][0]["pattern_id"] == "pattern_std_paragraph_v1"
    assert "RESOLVE" in result["matches"][0]["program_template"]["ops"]

def test_match_proven_patterns_demo():
    """Verifies that demo fingerprint returns more matches."""
    dna = {"fingerprint": "demo-fingerprint"}
    params = {"dna": dna}
    result = handle_match_proven_patterns(params)
    
    assert len(result["matches"]) >= 2
    assert any(m["pattern_id"] == "pattern_complex_table_v2" for m in result["matches"])


# -- propose_rerender_subgraph contract tests --

def test_propose_rerender_subgraph_simple():
    """Simple DNA produces a valid SubgraphProposal with backbone + 2 props + delta."""
    dna = {"fingerprint": "prose-standard-v1"}
    result = propose_rerender_subgraph(dna)

    assert result["dna_fingerprint"] == "prose-standard-v1"
    assert result["proposal_id"].startswith("subgraph:")
    assert result["confidence"] == 0.95

    slots = {t["slot"] for t in result["fragment_templates"]}
    assert "backbone" in slots
    assert "prop_0" in slots
    assert "prop_1" in slots
    assert "delta_0" in slots

    # backbone uses prose-backbone schema
    backbone = next(t for t in result["fragment_templates"] if t["slot"] == "backbone")
    assert backbone["schema"] == PROSE_BACKBONE_SCHEMA_ID

    # delta uses phrasing-delta schema
    delta = next(t for t in result["fragment_templates"] if t["slot"] == "delta_0")
    assert delta["schema"] == PHRASING_DELTA_SCHEMA_ID

    assert "hard-gate" == result["relational_metadata"]["verify_strategy"]
    assert "VERIFY" in result["reconstruction_ops"]
    assert "COMMIT" in result["reconstruction_ops"]


def test_propose_rerender_subgraph_complex():
    """Complex DNA produces more propositions and includes TABULATE op."""
    dna = {"fingerprint": "complex-table-analysis"}
    result = propose_rerender_subgraph(dna)

    assert result["confidence"] == 0.88
    slots = {t["slot"] for t in result["fragment_templates"]}
    assert "prop_2" in slots  # 3 props for complex
    assert "TABULATE" in result["reconstruction_ops"]


def test_propose_rerender_subgraph_slot_bindings():
    """Slot bindings use {{placeholder}} convention."""
    dna = {"fingerprint": "any"}
    result = propose_rerender_subgraph(dna)

    for template in result["fragment_templates"]:
        assert template["binding"].startswith("{{")
        assert template["binding"].endswith("}}")


def test_propose_rerender_subgraph_deterministic():
    """Same fingerprint always produces same proposal_id."""
    dna = {"fingerprint": "deterministic-test"}
    r1 = propose_rerender_subgraph(dna)
    r2 = propose_rerender_subgraph(dna)
    assert r1["proposal_id"] == r2["proposal_id"]


def test_propose_rerender_subgraph_missing_fingerprint_raises():
    """Missing or empty fingerprint raises ValueError."""
    with pytest.raises(ValueError, match="fingerprint"):
        propose_rerender_subgraph({})

    with pytest.raises(ValueError, match="fingerprint"):
        propose_rerender_subgraph({"fingerprint": ""})


def test_propose_rerender_subgraph_non_dict_raises():
    """Non-dict dna argument raises ValueError."""
    with pytest.raises(ValueError, match="dict"):
        propose_rerender_subgraph(None)  # type: ignore[arg-type]


def test_propose_rerender_subgraph_whitespace_fingerprint_raises():
    """Whitespace-only fingerprint is invalid and must fail fast."""
    with pytest.raises(ValueError, match="fingerprint"):
        propose_rerender_subgraph({"fingerprint": "   "})


def test_propose_rerender_subgraph_schema_shape_contract():
    """Proposal includes required keys with stable types."""
    result = propose_rerender_subgraph({"fingerprint": "shape-contract"})

    assert isinstance(result["proposal_id"], str)
    assert isinstance(result["dna_fingerprint"], str)
    assert isinstance(result["fragment_templates"], list)
    assert isinstance(result["relational_metadata"], dict)
    assert isinstance(result["reconstruction_ops"], list)
    assert isinstance(result["confidence"], float)

    for template in result["fragment_templates"]:
        assert set(template.keys()) == {"slot", "schema", "binding"}
        assert isinstance(template["slot"], str)
        assert isinstance(template["schema"], str)
        assert isinstance(template["binding"], str)
