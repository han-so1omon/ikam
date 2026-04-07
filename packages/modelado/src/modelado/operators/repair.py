from __future__ import annotations
import asyncio
import json
from typing import Any, Dict, List, Optional, Literal

from modelado.operators.core import (
    Operator,
    OperatorEnv,
    OperatorParams,
    record_provenance,
    ProvenanceRecord,
    MIME_STRUCTURED_DATA,
    _run_async_safely,
)
from modelado.config.llm_config import LLMConfig, LLMTask, TaskModelSelector
from modelado.oraculo.ai_client import GenerateRequest
from modelado.profiles.prose import PhrasingDelta, TextPatchOp

class RepairOperator(Operator):
    """
    Consumes a DriftMap and generates a corrected PhrasingDelta to resolve fidelity failures.
    Satisfies Architecture Decision M15/M22 for drift-informed repair.
    Also supports agentic repair using a whitelisted LLM (Plan C, Task 6).
    """

    def __init__(self, config: LLMConfig | None = None):
        self.config = config or TaskModelSelector.get_config(LLMTask.REPAIR)

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Dict[str, Any]:
        """
        Input: DriftMap (from VerifyOperator) or a result dict containing it.
        Output: An updated fragment set or a set of corrective deltas.
        """
        drift_data = fragment.get("drift") if isinstance(fragment, dict) else None
        if not drift_data:
            # If no drift, maybe we are being called on the whole verify result
            drift_data = fragment
            
        if not drift_data or "mismatches" not in drift_data:
            # Nothing to repair if there's no drift data
            return {"status": "no-op", "reason": "No drift data provided"}

        # Agentic repair if LLM is available
        if env.llm:
            try:
                import logging
                # Synchronously run async generate via bridge
                mismatches_summary = json.dumps(drift_data.get("mismatches", []), indent=2)
                response = _run_async_safely(env.llm.generate(GenerateRequest(
                    messages=[
                        {"role": "system", "content": "You are a fidelity repair agent. Given a list of byte-mismatches (drift), generate a set of character-level patch operations (insert/delete) to achieve 100% byte-fidelity. Return a JSON object with a single key 'ops' containing a list of operations with 'op', 'at', 'text', and 'length' fields."},
                        {"role": "user", "content": f"Drift Data:\n{mismatches_summary}"}
                    ],
                    model=self.config.model,
                    temperature=0.0,
                    response_format={"type": "json_object"}
                )))
                data = json.loads(response.text)
                if isinstance(data, dict) and "ops" in data:
                    ops_data = data["ops"]
                    return {
                        "status": "repaired",
                        "corrective_ops": ops_data,
                        "drift_context": drift_data,
                        "method": "agentic"
                    }
            except Exception as e:
                import logging
                logging.warning(f"Agentic repair failed, falling back to heuristic: {e}")
                # Fallback to heuristic
                pass

        # Heuristic fallback (Stage 1 implementation)
        mismatches = drift_data.get("mismatches", [])
        corrective_ops: List[TextPatchOp] = []
        
        for m in mismatches:
            offset = m.get("offset")
            if offset is None: continue
            
            if m.get("type") == "size_mismatch":
                # Handle trailing bytes
                actual_rem = m.get("actual_remaining")
                expected_rem = m.get("expected_remaining")
                
                # If we have expected trailing but not actual, we need to insert
                if expected_rem and not actual_rem:
                    corrective_ops.append(TextPatchOp(
                        op="insert",
                        at=offset,
                        text=bytes.fromhex(expected_rem).decode("utf-8", errors="replace"),
                        length=0
                    ))
            else:
                # Byte mismatch: Replace actual with expected
                expected_byte = m.get("expected")
                actual_byte = m.get("actual")
                
                if expected_byte is not None:
                    # Replace 1 byte at offset
                    corrective_ops.append(TextPatchOp(
                        op="delete",
                        at=offset,
                        length=1,
                        text=""
                    ))
                    corrective_ops.append(TextPatchOp(
                        op="insert",
                        at=offset,
                        text=chr(expected_byte),
                        length=0
                    ))

        return {
            "status": "repaired",
            "corrective_ops": [op.model_dump(exclude_none=True) for op in corrective_ops],
            "drift_context": drift_data,
            "method": "heuristic"
        }

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)
