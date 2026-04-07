import pytest
from modelado.operators.monadic import VerifyOperator
from modelado.operators.repair import RepairOperator
from modelado.operators.core import OperatorEnv, OperatorParams
from modelado.environment_scope import EnvironmentScope
from modelado.profiles.prose import PhrasingDelta, TextPatchOp
from modelado.oraculo.ai_client import GenerateRequest, GenerateResponse

_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/run/test")


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

def test_repair_operator_heuristic_success():
    """
    Verifies that RepairOperator can generate corrective ops from a DriftMap.
    """
    # 1. Setup a drift scenario: "The quick brown fox" vs "The quick red fox"
    # Offset 10: 'b' (98) vs 'r' (114)
    # Offset 11: 'r' (114) vs 'e' (101)
    # Offset 12: 'o' (111) vs 'd' (100)
    # Offset 13: 'w' (119) vs ' ' (32)
    # Offset 14: 'n' (110) vs 'f' (102)
    
    drift_map = {
        "status": "drift",
        "drift": {
            "size_diff": 0,
            "mismatches": [
                {"offset": 10, "expected": ord('b'), "actual": ord('r')},
                {"offset": 11, "expected": ord('r'), "actual": ord('e')},
                {"offset": 12, "expected": ord('o'), "actual": ord('d')},
                {"offset": 13, "expected": ord('w'), "actual": ord(' ')},
                {"offset": 14, "expected": ord('n'), "actual": ord('f')},
            ]
        }
    }
    
    env = OperatorEnv(seed=42, renderer_version="1.0.0", policy="strict", env_scope=_DEV_SCOPE)
    repair_op = RepairOperator()
    
    # 2. Apply Repair
    result = repair_op.apply(
        drift_map,
        OperatorParams(name="repair", parameters={}),
        env
    )
    
    assert result["status"] == "repaired"
    ops = result["corrective_ops"]
    assert len(ops) == 10 # 5 deletes + 5 inserts
    
    # Verify the first mismatch repair: delete 'r' at 10, insert 'b' at 10
    # Note: RepairOperator applies them in reverse offset order or just collects them.
    # Our implementation sorts them by offset DESC to be safe for patching.
    
    # The ops in result are NOT sorted yet, they are in the order they were added.
    # But since we iterate through mismatches (sorted by offset ASC in drift map),
    # the ops are added in ASC order.
    
    # Check if we have the right ops
    delete_ops = [op for op in ops if op["op"] == "delete"]
    insert_ops = [op for op in ops if op["op"] == "insert"]
    
    assert any(op["at"] == 10 and op["length"] == 1 for op in delete_ops)
    assert any(op["at"] == 10 and op["text"] == "b" for op in insert_ops)

def test_repair_operator_size_mismatch():
    """Verifies repair of trailing size mismatch."""
    drift_map = {
        "status": "drift",
        "drift": {
            "size_diff": -1,
            "mismatches": [
                {
                    "offset": 5, 
                    "type": "size_mismatch", 
                    "expected_remaining": "2e", # '.'
                    "actual_remaining": None
                }
            ]
        }
    }
    
    env = OperatorEnv(seed=42, renderer_version="1.0.0", policy="strict", env_scope=_DEV_SCOPE)
    repair_op = RepairOperator()
    
    result = repair_op.apply(drift_map, OperatorParams(name="repair", parameters={}), env)
    
    assert result["status"] == "repaired"
    ops = result["corrective_ops"]
    assert len(ops) == 1
    assert ops[0]["op"] == "insert"
    assert ops[0]["at"] == 5
    assert ops[0]["text"] == "."


def test_repair_operator_uses_env_llm_for_agentic_repair():
    drift_map = {
        "status": "drift",
        "drift": {
            "size_diff": 0,
            "mismatches": [{"offset": 0, "expected": ord("A"), "actual": ord("X")}],
        },
    }
    fake_llm = FakeLLMClient('{"ops": [{"op": "delete", "at": 0, "length": 1, "text": ""}, {"op": "insert", "at": 0, "length": 0, "text": "A"}]}')
    env = OperatorEnv(
        seed=42,
        renderer_version="1.0.0",
        policy="strict",
        env_scope=_DEV_SCOPE,
        llm=fake_llm,
    )

    result = RepairOperator().apply(drift_map, OperatorParams(name="repair", parameters={}), env)

    assert fake_llm.last_request is not None
    assert result["status"] == "repaired"
    assert result["method"] == "agentic"
    assert result["corrective_ops"][0]["op"] == "delete"
    assert result["corrective_ops"][1]["op"] == "insert"


def test_repair_operator_targets_exact_drift_offsets_only():
    """Repair ops must be generated only for mismatched offsets from DriftMap."""
    drift_map = {
        "status": "drift",
        "drift": {
            "size_diff": 0,
            "mismatches": [
                {"offset": 2, "expected": ord("A"), "actual": ord("X")},
                {"offset": 7, "expected": ord("B"), "actual": ord("Y")},
            ],
        },
    }
    env = OperatorEnv(seed=7, renderer_version="1.0.0", policy="strict", env_scope=_DEV_SCOPE)
    result = RepairOperator().apply(drift_map, OperatorParams(name="repair", parameters={}), env)

    assert result["status"] == "repaired"
    offsets = {op["at"] for op in result["corrective_ops"]}
    assert offsets == {2, 7}

if __name__ == "__main__":
    pytest.main([__file__])
