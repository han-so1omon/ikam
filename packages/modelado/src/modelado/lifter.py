"""ClaimLifter — extracts factual claims as SPO triples from surface fragments.

Satisfies the Lifter protocol (ikam.ir.protocols.Lifter).
Uses an LLM client to extract claims, following the same pattern as
SemanticNormalizer in ikam.forja.normalizer.

Layer 1 (modelado) — depends on LLM client, produces CAS-addressed Fragments.
"""
from __future__ import annotations

import json
from typing import Any, Dict, List, Protocol

from ikam.forja.cas import cas_fragment
from ikam.fragments import Fragment
from ikam.ir import ClaimIR
from ikam.ir.mime_types import CLAIM_IR
from modelado.config.llm_config import LLMConfig, LLMTask, TaskModelSelector


class LLMResult(Protocol):
    output: str


class AIClient(Protocol):
    async def call_model(self, prompt: str, model: str, temperature: float) -> LLMResult: ...


class LiftingError(Exception):
    """Raised when claim extraction fails (LLM error, malformed response)."""


class ClaimLifter:
    """Extracts factual claims as SPO triples from surface text fragments.

    Each surface fragment (heading/paragraph) is sent to the LLM which
    extracts structured claims. Each claim becomes a CAS-addressed Fragment
    with application/ikam-claim+v1+json MIME type.
    """

    def __init__(self, ai_client: AIClient, config: LLMConfig | None = None):
        self.ai_client = ai_client
        self.config = config or TaskModelSelector.get_config(LLMTask.LIFTING)

    async def lift(
        self,
        fragment: Fragment,
        *,
        artifact_id: str = "unknown",
        cluster_context: Dict[str, Any] | None = None,
        **kwargs: Any,
    ) -> List[Fragment]:
        """Extract claims from a surface fragment, returning IR fragments.

        Args:
            fragment: Surface fragment to lift into IR claims.
            artifact_id: Canonical artifact identity for IRBase metadata.
            cluster_context: Optional dict from embed_decomposed containing
                cluster membership info for this fragment.  Keys:
                - ``cluster_members``: list of cas_ids in the same cluster
                - ``centroid_id``: cas_id of the cluster centroid
                - ``member_texts``: list of text snippets from sibling members
                When provided, the LLM prompt is augmented so that claims
                extracted from semantically similar fragments stay consistent.
            **kwargs: Additional metadata passed from the ingestion pipeline.
        """
        content = ""
        if fragment.value and isinstance(fragment.value, dict) and "text" in fragment.value:
            content = fragment.value["text"]
        elif fragment.value and isinstance(fragment.value, str):
            content = fragment.value

        if not content.strip():
            return []

        # Build cluster-awareness section when context is available
        cluster_section = ""
        if cluster_context and cluster_context.get("member_texts"):
            siblings = cluster_context["member_texts"]
            cluster_section = (
                "\n\nContext: The following related text fragments are "
                "semantically similar to the text above. Ensure that claims "
                "extracted from this text are consistent with claims that "
                "would be extracted from these related fragments:\n"
                + "\n".join(f"- {t[:200]}" for t in siblings[:5])
                + "\n"
            )

        prompt = (
            "Extract factual claims from the following text as JSON.\n"
            "Return a JSON array of objects, each with keys: "
            '"subject", "predicate", "object", "confidence" (0-1).\n'
            "Only extract explicit factual statements. "
            "Do NOT invent claims not supported by the text.\n\n"
            f"Text:\n{content}\n"
            f"{cluster_section}\n"
            "Return ONLY valid JSON array, no markdown fencing."
        )

        try:
            result = await self.ai_client.call_model(
                prompt=prompt,
                model=self.config.model.value,
                temperature=self.config.temperature,
            )
            response_text = result.output.strip()
            if response_text.startswith("```json"):
                response_text = response_text[7:]
            if response_text.startswith("```"):
                response_text = response_text[3:]
            if response_text.endswith("```"):
                response_text = response_text[:-3]
            response_text = response_text.strip()
            claims_raw = json.loads(response_text)
            if not isinstance(claims_raw, list):
                claims_raw = [claims_raw]
        except json.JSONDecodeError as exc:
            raise LiftingError(f"LLM returned malformed JSON: {exc}") from exc
        except Exception as exc:
            raise LiftingError(f"LLM lifting failed: {exc}") from exc

        fragments: List[Fragment] = []
        for raw in claims_raw:
            if not isinstance(raw, dict):
                continue
            subject = str(raw.get("subject", "")).strip()
            predicate = str(raw.get("predicate", "")).strip()
            obj = str(raw.get("object", "")).strip()
            if not (subject and predicate and obj):
                continue

            confidence = float(raw.get("confidence", 1.0))
            confidence = max(0.0, min(1.0, confidence))

            claim = ClaimIR(
                artifact_id=artifact_id,
                subject=subject,
                predicate=predicate,
                object=obj,
                confidence=confidence,
                qualifiers=raw.get("qualifiers") or {},
                scope_id=kwargs.get("scope_id"),
                provenance_id=kwargs.get("provenance_id"),
            )
            frag = cas_fragment(claim.model_dump(mode="json"), CLAIM_IR)
            fragments.append(frag)

        return fragments
