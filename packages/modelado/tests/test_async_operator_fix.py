import pytest
import asyncio
from modelado.operators.core import OperatorEnv, OperatorParams
from modelado.operators.lifting import LiftOperator
from modelado.plans.mapping import StructuralMap, StructuralMapNode
from modelado.environment_scope import EnvironmentScope
from modelado.oraculo.ai_client import GenerateResponse, AIClient, GenerateRequest

class DummyLLMClient(AIClient):
    async def generate(self, request: GenerateRequest) -> GenerateResponse:
        return GenerateResponse(
            text='{"propositions": ["One", "Two"]}',
            provider="dummy",
            model="dummy"
        )
    async def judge(self, request):
        pass

def test_lift_operator_async_context_fix():
    """
    Tests that LiftOperator doesn't crash with "asyncio.run() cannot be called from a running event loop"
    when run from within an async context (like the pipeline engine).
    """
    async def run_in_async_context():
        source_text = "Revenue improved. Margins expanded."
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

        env = OperatorEnv(
            seed=42, 
            renderer_version="1.0.0", 
            policy="strict", 
            env_scope=EnvironmentScope(ref="refs/heads/run/run/test"),
            llm=DummyLLMClient()
        )

        lift_op = LiftOperator()
        
        # This previously threw RuntimeError about nested asyncio.run
        fragments = lift_op.apply(
            None,
            OperatorParams(name="lift", parameters={
                "source_text": source_text,
                "structural_map": smap.model_dump(mode="json", by_alias=True)
            }),
            env
        )
        return fragments
        
    fragments = asyncio.run(run_in_async_context())
    
    assert len(fragments["propositions"]) == 2
    assert fragments["propositions"][0]["content"] == "One"
    assert fragments["propositions"][1]["content"] == "Two"
