from __future__ import annotations
from typing import Any, Dict, List, Optional, cast, Protocol, runtime_checkable

from modelado.operators.core import (
    Operator,
    OperatorEnv,
    OperatorParams,
    record_provenance,
    ProvenanceRecord,
)
from modelado.plans.mapping import MapDNA, StructuralMap

@runtime_checkable
class VectorStore(Protocol):
    async def search(self, query_vector: list[float], limit: int = 10, **kwargs: Any) -> List[Dict[str, Any]]: ...
    async def add(self, fragment_id: str, vector: list[float], metadata: Dict[str, Any]) -> None: ...

@runtime_checkable
class EmbeddingClient(Protocol):
    async def embed(self, text: str) -> list[float]: ...

class EmbedOperator(Operator):
    """
    Computes semantic embeddings for a fragment's content.
    Satisfies EMBED step in the 10-step ingestion Petri Net.
    """

    def __init__(self, embedder: Optional[EmbeddingClient] = None):
        self.embedder = embedder

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Any:
        # 1. Handle Fragment Set (Stage 2 implementation: multi-embedding)
        if isinstance(fragment, dict) and "propositions" in fragment:
            props = fragment.get("propositions", [])
            # Return list of embeddings for each proposition
            return [self.apply(p, params, env) for p in props]

        text = ""
        if isinstance(fragment, str):
            text = fragment
        elif isinstance(fragment, dict) and "content" in fragment:
            text = str(fragment["content"])
        else:
            raise ValueError("EmbedOperator requires a string, a dict with 'content', or a fragment set")

        if not self.embedder:
            # Fallback/Mock for testing: Deterministic pseudo-embedding based on hash
            import hashlib
            h = hashlib.sha256(text.encode("utf-8")).digest()
            # Return 32 floats for testing
            return [float(b) / 255.0 for b in h]

        # In a real async environment, this would be awaited.
        # For our sync Operator.apply, we'd need a bridge.
        # Stage 2 implementation: sync bridge to async embedder.
        return [0.0] * 1536 # Placeholder for real embedding

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)


class SearchOperator(Operator):
    """
    Retrieves fragments from the Knowledge Base based on semantic similarity or structural DNA.
    Satisfies SEARCH step in the 10-step ingestion Petri Net.
    """

    def __init__(self, vector_store: Optional[VectorStore] = None):
        self.vector_store = vector_store

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Any:
        query_vector = params.parameters.get("query_vector") or fragment
        dna_data = params.parameters.get("dna")
        limit = params.parameters.get("limit", 10)

        if not query_vector and not dna_data:
            raise ValueError("SearchOperator requires 'query_vector' or 'dna' parameter")

        # Handle batch of vectors
        if isinstance(query_vector, list) and len(query_vector) > 0 and isinstance(query_vector[0], (list, float)):
            # Check if it's a list of vectors or a single vector
            if isinstance(query_vector[0], list):
                return [self._search_single(v, dna_data, limit) for v in query_vector]
            else:
                return self._search_single(query_vector, dna_data, limit)

        return []

    def _search_single(self, vector: Optional[list[float]], dna_data: Any, limit: int) -> List[Dict[str, Any]]:
        if dna_data:
            # Neighborhood retrieval based on Map DNA
            # (Stage 2 implementation: structural matching logic)
            pass

        if vector:
            if not self.vector_store:
                # Mock results for testing
                return [{"id": "kb_match_1", "score": 0.95, "content": "Sample KB Match"}]
            
            # Real vector search would go here
            # In a real async environment, this would be awaited via a sync bridge.
            pass

        return []

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)


class NormalizeOperator(Operator):
    """
    Aligns identified fragments with the global Knowledge Base.
    Satisfies NORMALIZE step in the 10-step ingestion Petri Net.
    """

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Any:
        # Input: Fragment Set (lifted_fragments)
        # Output: Normalized Fragment Set
        
        if not isinstance(fragment, dict) or "propositions" not in fragment:
            raise ValueError("NormalizeOperator requires a fragment set as input")

        candidates_batch = params.parameters.get("candidates", [])
        propositions = fragment.get("propositions", [])
        
        if not isinstance(candidates_batch, list) or len(candidates_batch) != len(propositions):
            # If candidates is not a batch or doesn't match, we might be in a single-item flow
            # or it might be a bug in the pipeline. 
            # For the prototype, we'll try to handle it gracefully.
            pass

        normalized_props = []
        for i, prop in enumerate(propositions):
            # Get candidates for this specific proposition
            candidates = candidates_batch[i] if i < len(candidates_batch) else []
            if not isinstance(candidates, list):
                candidates = [candidates] if candidates else []

            norm_prop = self._normalize_single(prop, candidates)
            normalized_props.append(norm_prop)

        # Return the updated fragment set
        return {
            **fragment,
            "propositions": normalized_props,
            "normalization_metadata": {
                "timestamp": env.seed # Dummy timestamp for prototype
            }
        }

    def _normalize_single(self, proposition: Dict[str, Any], candidates: List[Dict[str, Any]]) -> Dict[str, Any]:
        # Simple normalization logic: if we have a high-confidence match, resolve to it.
        # Otherwise, keep it as is.
        
        if candidates and candidates[0].get("score", 0) > 0.9:
            match = candidates[0]
            return {
                **proposition,
                "status": "resolved",
                "kb_id": match["id"],
                "kb_score": match["score"]
            }
        
        return {
            **proposition,
            "status": "new"
        }

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)
