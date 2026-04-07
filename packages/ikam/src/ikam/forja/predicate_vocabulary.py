"""Predicate vocabulary as a CAS-addressable artifact.

Ships with a default set of 23 canonical predicates (6 structural,
13 knowledge, 4 semantic) from the compression/re-render design doc.
Stored as a Fragment so it can be modified/extended at runtime.

PredicateResolver embeds predicate labels into vector space for
similarity search (e.g., AI-based graph/vector retrieval).
"""
from __future__ import annotations

from typing import Any, Dict, List

import numpy as np

from ikam.forja.cas import cas_fragment
from ikam.fragments import Fragment
from ikam.ir.mime_types import PREDICATE_VOCABULARY

# -- Canonical predicate definitions (Appendix A) --

_STRUCTURAL: List[Dict[str, str]] = [
    {"name": "contains", "category": "structural",
     "description": "Parent contains child fragment"},
    {"name": "lifted-from", "category": "structural",
     "description": "IR was lifted from surface fragment"},
    {"name": "normalized-by", "category": "structural",
     "description": "Fragment produced by normalization"},
    {"name": "composed-by", "category": "structural",
     "description": "Fragment produced by ReconstructionProgram"},
    {"name": "verified-by", "category": "structural",
     "description": "Fragment verified by VerificationResult"},
    {"name": "original-bytes-of", "category": "structural",
     "description": "CAS fragment holding original imported bytes"},
]

_KNOWLEDGE: List[Dict[str, str]] = [
    {"name": "normalize:expression", "category": "knowledge",
     "description": "Normalization produced an expression form"},
    {"name": "normalize:numeric", "category": "knowledge",
     "description": "Normalization produced a numeric form"},
    {"name": "normalize:temporal", "category": "knowledge",
     "description": "Normalization produced a temporal form"},
    {"name": "normalize:unit", "category": "knowledge",
     "description": "Normalization produced a unit-annotated form"},
    {"name": "normalize:label", "category": "knowledge",
     "description": "Normalization produced a label form"},
    {"name": "normalize:template", "category": "knowledge",
     "description": "Normalization produced a template form"},
    {"name": "normalize:base-delta", "category": "knowledge",
     "description": "Normalization produced a base-delta form"},
    {"name": "compose:overlay", "category": "knowledge",
     "description": "Composition via overlay strategy"},
    {"name": "compose:concatenate", "category": "knowledge",
     "description": "Composition via concatenation strategy"},
    {"name": "compose:instantiate", "category": "knowledge",
     "description": "Composition via template instantiation"},
    {"name": "compose:format", "category": "knowledge",
     "description": "Composition via format application"},
    {"name": "compose:transform", "category": "knowledge",
     "description": "Composition via transformation"},
    {"name": "compose:generate", "category": "knowledge",
     "description": "Composition via generation"},
]

_SEMANTIC: List[Dict[str, str]] = [
    {"name": "shadows", "category": "semantic",
     "description": "Fragment shadows (supersedes) another fragment"},
    {"name": "derived-from", "category": "semantic",
     "description": "Fragment derived from another via semantic operation"},
    {"name": "depends-on", "category": "semantic",
     "description": "Fragment depends on another for correctness"},
    {"name": "feeds", "category": "semantic",
     "description": "Fragment feeds data into another fragment"},
]

_VERSION = "2026-02-10"


def build_default_vocabulary() -> Fragment:
    """Build the default predicate vocabulary as a CAS-addressed Fragment.

    Returns a Fragment with mime_type PREDICATE_VOCABULARY whose value is::

        {"predicates": [...], "version": "2026-02-10"}

    Deterministic: same call always produces the same CAS ID.
    """
    value: Dict[str, Any] = {
        "predicates": _STRUCTURAL + _KNOWLEDGE + _SEMANTIC,
        "version": _VERSION,
    }
    return cas_fragment(value, PREDICATE_VOCABULARY)


class PredicateResolver:
    """Embeds predicate labels into vector space for similarity search.

    Uses sentence-transformers to encode predicate names + descriptions,
    then computes cosine similarity for retrieval.
    """

    def __init__(
        self,
        predicate_names: List[str],
        categories: Dict[str, str],
        embeddings: np.ndarray,
    ):
        self.predicate_names = predicate_names
        self._categories = categories
        self._embeddings = embeddings  # shape (N, dim)
        # Pre-compute norms for cosine similarity
        norms = np.linalg.norm(embeddings, axis=1, keepdims=True)
        norms[norms == 0] = 1.0
        self._normed = embeddings / norms

    @classmethod
    def from_vocabulary(cls, vocab: Fragment) -> PredicateResolver:
        """Build a resolver from a vocabulary Fragment.

        Embeds each predicate as "{name}: {description}" for richer semantics.
        """
        from modelado.fragment_embedder import get_shared_embedder

        predicates = vocab.value["predicates"]
        texts = [f"{p['name']}: {p['description']}" for p in predicates]
        embeddings = get_shared_embedder().encode_texts(texts)

        names = [p["name"] for p in predicates]
        categories = {p["name"]: p["category"] for p in predicates}
        return cls(names, categories, embeddings)

    def similar(self, query: str, top_k: int = 5) -> List[Dict[str, Any]]:
        """Find predicates most similar to a query string.

        Args:
            query: predicate name or natural language description
            top_k: number of results to return

        Returns:
            List of dicts with 'name', 'score' (0-1 cosine), 'category',
            sorted by descending similarity.
        """
        from modelado.fragment_embedder import get_shared_embedder

        # If query matches a known predicate name, use the same rich text
        matching = [
            f"{name}: {desc}"
            for name, desc in (
                (p["name"], p["description"])
                for p in (_STRUCTURAL + _KNOWLEDGE + _SEMANTIC)
            )
            if name == query
        ]
        query_text = matching[0] if matching else query

        q_vec = get_shared_embedder().encode_texts([query_text])
        q_vec = np.array(q_vec, dtype=np.float32).reshape(1, -1)
        q_norm = np.linalg.norm(q_vec)
        if q_norm > 0:
            q_vec = q_vec / q_norm

        scores = (self._normed @ q_vec.T).flatten()
        # Clamp to [0, 1]
        scores = np.clip(scores, 0.0, 1.0)

        top_indices = np.argsort(-scores)[:top_k]
        return [
            {
                "name": self.predicate_names[i],
                "score": float(scores[i]),
                "category": self._categories[self.predicate_names[i]],
            }
            for i in top_indices
        ]
