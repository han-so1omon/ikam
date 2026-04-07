import pytest
import json
from modelado.operators.core import OperatorEnv, OperatorParams
from modelado.environment_scope import EnvironmentScope
from modelado.operators.mapping import MapDNAOperator
from modelado.plans.mapping import StructuralMap, MapDNA

_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/run/test")

def test_map_dna_operator():
    """
    Verifies that MapDNAOperator correctly produces a StructuralMap and 
    computes deterministic Map DNA from a structural hierarchy.
    """
    
    # 1. Input: Sample Structural Hierarchy (TOC)
    hierarchy = {
        "id": "root",
        "title": "Document Root",
        "level": 0,
        "kind": "document",
        "children": [
            {
                "id": "section_1",
                "title": "Introduction",
                "level": 1,
                "kind": "section",
                "children": [
                    {
                        "id": "para_1",
                        "title": "First Paragraph",
                        "level": 2,
                        "kind": "paragraph",
                        "source_range": {"start": 0, "end": 100}
                    }
                ]
            },
            {
                "id": "section_2",
                "title": "Methodology",
                "level": 1,
                "kind": "section",
                "children": []
            }
        ]
    }
    
    artifact_id = "test-artifact-123"
    
    # 2. Setup Operator
    op = MapDNAOperator()
    env = OperatorEnv(seed=42, renderer_version="1.0.0", policy="strict", env_scope=_DEV_SCOPE)
    params = OperatorParams(
        name="map_dna",
        parameters={
            "artifact_id": artifact_id,
            "structural_hierarchy": hierarchy
        }
    )
    
    # 3. Execute Operator
    result = op.apply(None, params, env)
    
    # 4. Verify StructuralMap
    assert result["artifact_id"] == artifact_id
    assert result["schema"] == "modelado/structural-map@1"
    assert result["root"]["title"] == "Document Root"
    assert len(result["root"]["children"]) == 2
    
    # 5. Verify Map DNA
    dna_data = result["dna"]
    assert dna_data is not None
    assert "fingerprint" in dna_data
    assert "structural_hashes" in dna_data
    assert len(dna_data["structural_hashes"]) == 4 # root + s1 + p1 + s2
    
    # 6. Verify Determinism
    # Running again with same input should yield same DNA
    result_2 = op.apply(None, params, env)
    assert result_2["dna"]["fingerprint"] == dna_data["fingerprint"]
    assert result_2["dna"]["structural_hashes"] == dna_data["structural_hashes"]
    
    # 7. Verify DNA changes if structure changes
    hierarchy_modified = json.loads(json.dumps(hierarchy))
    hierarchy_modified["children"][1]["title"] = "Modified Methodology"
    
    params_mod = OperatorParams(
        name="map_dna",
        parameters={
            "artifact_id": artifact_id,
            "structural_hierarchy": hierarchy_modified
        }
    )
    
    result_mod = op.apply(None, params_mod, env)
    assert result_mod["dna"]["fingerprint"] != dna_data["fingerprint"]
