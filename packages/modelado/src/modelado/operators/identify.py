"""IdentifyOperator — Extracts factual concepts (propositions) from text.

Satisfies the IDENTIFY step in the 10-step ingestion Petri Net.
Uses TaskModelSelector to choose the appropriate LLM for extraction.
"""

from __future__ import annotations

import json
from typing import Any, Dict, List, Optional, Protocol, cast

from modelado.config.llm_config import LLMConfig, LLMTask, TaskModelSelector
from modelado.operators.core import (
    MIME_PROPOSITION,
    Operator,
    OperatorEnv,
    OperatorParams,
    ProvenanceRecord,
    record_provenance,
)


class LLMResult(Protocol):
    output: str


class AIClient(Protocol):
    async def call_model(self, prompt: str, model: str, temperature: float) -> LLMResult: ...


class IdentifyOperator(Operator):
    """
    Extracts atomic factual statements from a text fragment using an LLM.

    Parameters:
        - artifact_id: str - ID of the artifact.
    """

    def __init__(self, ai_client: Optional[AIClient] = None, config: Optional[LLMConfig] = None):
        self.ai_client = ai_client
        self.config = config or TaskModelSelector.get_config(LLMTask.IDENTIFY)

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> List[Dict[str, Any]]:
        # In a real async environment, we'd await self.ai_client.call_model.
        # But our current Operator.apply is synchronous.
        # For Stage 1, we'll use a synchronous mock or assume the LLM is handled by the worker.
        # If no ai_client is provided, we'll do a basic sentence split as fallback.

        text = ""
        if isinstance(fragment, str):
            text = fragment
        elif isinstance(fragment, dict) and "text" in fragment:
            text = fragment["text"]
        else:
            raise ValueError("IdentifyOperator requires a string or dict with 'text'")

        if not self.ai_client:
            # Fallback to simple sentence split if no LLM client is provided
            sentences = [s.strip() for s in text.split(".") if s.strip()]
            return [
                {
                    "mime_type": MIME_PROPOSITION,
                    "content": s,
                    "confidence": 1.0,
                    "modality": "factual",
                }
                for s in sentences
            ]

        # In a real implementation, we'd have a sync wrapper or use asyncio.run (not recommended in all contexts).
        # For now, we'll keep the fallback or return a placeholder if we're in a purely sync path.
        return [
            {
                "mime_type": MIME_PROPOSITION,
                "content": text,
                "confidence": 0.5,
                "modality": "factual",
                "note": "LLM Identification pending async execution",
            }
        ]

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)
