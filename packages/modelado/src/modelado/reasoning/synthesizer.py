from __future__ import annotations
import json
import re
from typing import Dict, List, Any, Optional, Union
from modelado.reasoning.query import SemanticQuery, Subgraph
from modelado.oraculo.ai_client import AIClient, GenerateRequest

class SynthesizerService:
    """
    Service to generate natural-language interpretations from discovered subgraphs.
    
    This service satisfies Architecture Decision D18 by ensuring that every 
    synthesized claim is attributed to a source fragment ID (CAS hash).
    """
    
    def __init__(self, ai_client: AIClient, model: str = "gpt-4o-mini"):
        self.ai_client = ai_client
        self.model = model

    def _claim_supported_by_interpretation(self, claim: str, interpretation: str) -> bool:
        if claim.lower() in interpretation.lower():
            return True

        def _norm_tokens(text: str) -> set[str]:
            raw = re.findall(r"[a-z0-9]+", text.lower())
            synonyms = {
                "growth": "grow",
                "grew": "grow",
                "growing": "grow",
                "increase": "grow",
                "increased": "grow",
                "decrease": "decline",
                "decreased": "decline",
                "fell": "decline",
                "falling": "decline",
            }
            stop = {"the", "a", "an", "and", "or", "of", "to", "by", "in", "on", "for", "is", "are"}
            out: set[str] = set()
            for token in raw:
                if token in stop:
                    continue
                out.add(synonyms.get(token, token))
            return out

        claim_tokens = _norm_tokens(claim)
        interpretation_tokens = _norm_tokens(interpretation)
        if not claim_tokens:
            return False
        overlap = claim_tokens.intersection(interpretation_tokens)
        return len(overlap) >= min(2, len(claim_tokens))

    def _validate_d18_payload(self, payload: Dict[str, Any], subgraph: Subgraph) -> None:
        interpretation = str(payload.get("interpretation") or "")
        attribution = payload.get("attribution")
        if not isinstance(attribution, list):
            raise ValueError("Invalid attribution payload")
        if not attribution:
            raise ValueError("Attribution must not be empty")

        allowed_fragment_ids = {node.fragment_id for node in subgraph.nodes if getattr(node, "fragment_id", None)}
        cited_fragment_ids = set(re.findall(r"\[([^\[\]]+)\]", interpretation))
        if not cited_fragment_ids:
            raise ValueError("Missing bracketed citations in interpretation")
        if not cited_fragment_ids.issubset(allowed_fragment_ids):
            raise ValueError("Interpretation cites unknown fragment_ids")

        attributed_fragment_ids = set()
        for entry in attribution:
            if not isinstance(entry, dict):
                raise ValueError("Invalid attribution payload")
            claim = entry.get("claim")
            if not isinstance(claim, str) or not claim.strip():
                raise ValueError("Invalid attribution payload: claim must be non-empty")
            if not self._claim_supported_by_interpretation(claim, interpretation):
                raise ValueError("Invalid attribution payload: claim must be represented in interpretation")

            fragment_ids = entry.get("fragment_ids")
            if not isinstance(fragment_ids, list) or not fragment_ids:
                raise ValueError("Invalid attribution payload")
            for fid in fragment_ids:
                if not isinstance(fid, str) or not fid.strip():
                    raise ValueError("Invalid attribution payload: fragment_ids must be non-empty strings")
                attributed_fragment_ids.add(fid)

        if not attributed_fragment_ids.issubset(allowed_fragment_ids):
            raise ValueError("Attribution contains unknown fragment_ids")
        
    async def synthesize(self, query: SemanticQuery, subgraph: Subgraph) -> Dict[str, Any]:
        """
        Generate a natural language interpretation with byte-fidelity attribution.
        
        This handles the 'Interpretation' phase of the SQI Framework.
        
        Args:
            query: The original semantic query with intent and directives.
            subgraph: The result of the discovery traversal (nodes + edges).
            
        Returns:
            A JSON object containing the 'interpretation' and 'attribution'.
        """
        # 1. Prepare fragment context with IDs for attribution
        fragment_context = []
        for node in subgraph.nodes:
            # We use model_dump for serializing the IR nodes
            node_data = node.model_dump(mode="json", exclude_none=True)
            # Ensure the fragment_id is prominent in the context for LLM
            fragment_context.append({
                "fragment_id": node.fragment_id,
                "ir_kind": node.__class__.__name__.replace("IR", ""),
                "payload": node_data
            })
            
        # 2. Extract directives and interpretation context
        directives = "\n".join([f"- {d}" for d in query.interpretation.directives])
        audience = query.interpretation.audience or "General"
        purpose = query.interpretation.purpose or "Explanation"
        
        # 3. Construct System Prompt
        system_prompt = f"""
You are the Narraciones Synthesizer. Your goal is to interpret a subgraph of computational fragments.
Target Audience: {audience}
Primary Purpose: {purpose}

Directives:
{directives}

Every claim you make MUST include a citation to the source fragment_id in brackets, e.g., [frag_hash].
Strict Byte-Fidelity (D18): You must only synthesize claims that can be directly attributed to the provided IR nodes.

Return your response as a JSON object with two fields:
1. "interpretation": The natural language text of the interpretation.
2. "attribution": A list of claim-to-fragment mappings, e.g. [{{"claim": "...", "fragment_ids": ["..."]}}].
"""

        # 4. Construct User Prompt
        user_prompt = f"""
Intent: {query.intent}

Subgraph Context (Fragments):
{json.dumps(fragment_context, indent=2)}

Please provide the interpretation now.
"""

        # 5. Execute LLM Call
        request = GenerateRequest(
            messages=[
                {"role": "system", "content": system_prompt},
                {"role": "user", "content": user_prompt}
            ],
            model=self.model,
            response_format={"type": "json_object"}
        )
        
        response = await self.ai_client.generate(request)
        
        # 6. Parse and return result
        try:
            parsed = json.loads(response.text)
        except json.JSONDecodeError:
            raise ValueError("LLM failed to produce valid JSON attribution")

        self._validate_d18_payload(parsed, subgraph)
        return parsed
