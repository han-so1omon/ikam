import pytest
from modelado.operators.lifting import LiftOperator
from modelado.operators.monadic import JoinOperator, ApplyOperator, VerifyOperator, ResolveOperator
from modelado.operators.core import OperatorEnv, OperatorParams, FragmentLoader
from modelado.environment_scope import EnvironmentScope
from modelado.plans.mapping import StructuralMap, StructuralMapNode
from modelado.oraculo.ai_client import GenerateRequest, GenerateResponse
from modelado.mcp.directive_server import (
    PHRASING_DELTA_SCHEMA_ID,
    PROSE_BACKBONE_SCHEMA_ID,
    propose_rerender_subgraph,
)
from typing import Dict, Any

class MockLoader(FragmentLoader):
    def __init__(self, fragments: Dict[str, Any]):
        self.fragments = fragments
    
    def load(self, fragment_id: str) -> Any:
        return self.fragments.get(fragment_id)


class FakeLLMClient:
    def __init__(self, response_text: str):
        self.response_text = response_text
        self.last_request: GenerateRequest | None = None

    async def generate(self, request: GenerateRequest) -> GenerateResponse:
        self.last_request = request
        return GenerateResponse(
            text=self.response_text,
            provider="fake",
            model=request.model,
        )

def test_lift_operator_fidelity():
    """
    Verifies that LiftOperator produces fragments that can be reconstructed
    to 100% byte-identity using the monadic kernel.
    """
    source_text = "The quick brown fox, it jumps over the lazy dog."
    
    # 1. Define Structural Map
    smap = StructuralMap(
        artifact_id="art_1",
        root=StructuralMapNode(
            id="node_1",
            title="Root Node",
            level=0,
            kind="prose",
            source_range={"start": 0, "end": len(source_text)},
            children=[]
        )
    )
    
    # 2. Lift
    lift_op = LiftOperator()
    fragments = lift_op.apply(
        None,
        OperatorParams(name="lift", parameters={
            "source_text": source_text,
            "structural_map": smap.model_dump(mode="json", by_alias=True)
        }),
        OperatorEnv(seed=42, renderer_version="1.0.0", policy="strict", env_scope=EnvironmentScope(ref="refs/heads/run/run/test"))
    )
    
    # 3. Setup Loader with lifted fragments
    fragment_store = {}
    for p in fragments["propositions"]:
        fragment_store[p["id"]] = p["content"]
    for b in fragments["backbones"]:
        fragment_store[b["id"]] = b["content"]
        
    loader = MockLoader(fragment_store)
    env = OperatorEnv(seed=42, renderer_version="1.0.0", policy="strict", loader=loader, env_scope=EnvironmentScope(ref="refs/heads/run/run/test"))
    
    # 4. Reconstruct
    # 4a. Join Propositions from Backbone
    backbone_data = fragments["backbones"][0]["content"]
    prop_ids = backbone_data["proposition_ids"]
    props = [loader.load(pid) for pid in prop_ids]
    
    join_op = JoinOperator()
    joined_text = join_op.apply(
        props,
        OperatorParams(name="join", parameters={"strategy": "concat"}),
        env
    )
    
    # 4b. Apply Phrasing Delta
    delta_data = fragments["deltas"][0]["content"]
    apply_op = ApplyOperator()
    reconstructed_text = apply_op.apply(
        joined_text,
        OperatorParams(name="apply", parameters={
            "delta": delta_data["ops"],
            "delta_type": "text"
        }),
        env
    )
    
    # 5. Verify
    verify_op = VerifyOperator()
    v_params = OperatorParams(name="verify", parameters={
        "original_bytes": source_text.encode("utf-8"),
        "reconstructor": lambda x: x.encode("utf-8")
    })
    v_result = verify_op.apply(reconstructed_text, v_params, env)
    
    assert reconstructed_text == source_text
    assert v_result["status"] == "success"
    assert v_result["drift"] is None


def test_lift_operator_uses_env_llm_for_agentic_lifting():
    source_text = "Revenue improved. Margins expanded."
    smap = StructuralMap(
        artifact_id="art_agentic",
        root=StructuralMapNode(
            id="node_agentic",
            title="Root",
            level=0,
            kind="prose",
            source_range={"start": 0, "end": len(source_text)},
            children=[],
        ),
    )
    fake_llm = FakeLLMClient('{"propositions": ["Revenue improved", "Margins expanded"]}')
    env = OperatorEnv(
        seed=42,
        renderer_version="1.0.0",
        policy="strict",
        env_scope=EnvironmentScope(ref="refs/heads/run/run/test-agentic-lift"),
        llm=fake_llm,
    )

    result = LiftOperator().apply(
        source_text,
        OperatorParams(
            name="lift",
            parameters={"source_text": source_text, "structural_map": smap.model_dump(mode="json", by_alias=True)},
        ),
        env,
    )

    assert fake_llm.last_request is not None
    assert len(result["propositions"]) == 2
    assert result["propositions"][0]["content"] == "Revenue improved"
    assert result["propositions"][1]["content"] == "Margins expanded"


def test_lift_output_includes_blueprint_schema_contract_fields():
    """Lift output should include schema IDs required by SubgraphProposal contracts."""
    source_text = "One. Two."
    smap = StructuralMap(
        artifact_id="art_contract",
        root=StructuralMapNode(
            id="node_contract",
            title="Root",
            level=0,
            kind="prose",
            source_range={"start": 0, "end": len(source_text)},
            children=[],
        ),
    )
    proposal = propose_rerender_subgraph({"fingerprint": "contract-lift"})
    expected_prop_slots = set(proposal["relational_metadata"]["backbone_slots_propositions"])

    env = OperatorEnv(
        seed=1,
        renderer_version="1.0.0",
        policy="strict",
        env_scope=EnvironmentScope(ref="refs/heads/run/run/test-contract"),
    )
    result = LiftOperator().apply(
        source_text,
        OperatorParams(
            name="lift",
            parameters={"source_text": source_text, "structural_map": smap.model_dump(mode="json", by_alias=True)},
        ),
        env,
    )

    backbone = result["backbones"][0]
    delta = result["deltas"][0]
    assert backbone["schema"] == PROSE_BACKBONE_SCHEMA_ID
    assert delta["schema"] == PHRASING_DELTA_SCHEMA_ID

    proposition_ids = backbone["content"]["proposition_ids"]
    assert len(proposition_ids) == len(expected_prop_slots)
