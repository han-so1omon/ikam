import pytest
from modelado.plans.mapping import StructuralMap, StructuralMapNode, compute_map_dna
from modelado.operators.mapping import MapDNAOperator
from modelado.operators.core import OperatorEnv, OperatorParams
from modelado.environment_scope import EnvironmentScope

_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/run/test")

def test_structural_map_dna_fingerprint():
    # Define a sample structural map
    node2_1 = StructuralMapNode(id="n2.1", title="Subsection A", level=2, kind="section")
    node2_2 = StructuralMapNode(id="n2.2", title="Subsection B", level=2, kind="section")
    
    node1_1 = StructuralMapNode(id="n1.1", title="Section 1", level=1, kind="section", children=[node2_1, node2_2])
    node1_2 = StructuralMapNode(id="n1.2", title="Section 2", level=1, kind="section")
    
    root = StructuralMapNode(id="root", title="Document Root", level=0, kind="document", children=[node1_1, node1_2])
    
    smap = StructuralMap(artifact_id="artifact-123", root=root)
    
    # Compute DNA
    dna = compute_map_dna(smap)
    
    assert dna.fingerprint is not None
    assert len(dna.structural_hashes) == 5 # 1 root + 2 level-1 + 2 level-2
    assert dna.version == "1"
    
    # Verify stability: re-computing DNA for the same map should produce the same fingerprint
    dna2 = compute_map_dna(smap)
    assert dna2.fingerprint == dna.fingerprint
    
    # Verify sensitivity: changing a title should change the fingerprint
    node1_2.title = "Section 2 modified"
    smap_mod = StructuralMap(artifact_id="artifact-123", root=root)
    dna_mod = compute_map_dna(smap_mod)
    assert dna_mod.fingerprint != dna.fingerprint

def test_map_dna_operator_apply():
    op = MapDNAOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    
    raw_hierarchy = {
        "id": "root",
        "title": "Document",
        "level": 0,
        "kind": "document",
        "children": [
            {
                "id": "sec1",
                "title": "Introduction",
                "level": 1,
                "kind": "section"
            }
        ]
    }
    
    params = OperatorParams(
        name="map_dna",
        parameters={
            "artifact_id": "art-456",
            "structural_hierarchy": raw_hierarchy
        }
    )
    
    result = op.apply(None, params, env)
    
    assert result["artifact_id"] == "art-456"
    assert result["root"]["title"] == "Document"
    assert result["dna"]["fingerprint"] is not None
    assert len(result["dna"]["structural_hashes"]) == 2
