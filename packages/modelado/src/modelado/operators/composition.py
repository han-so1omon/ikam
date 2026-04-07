from __future__ import annotations
from typing import Any, Dict, List, Optional, cast, Protocol, runtime_checkable

from modelado.operators.core import (
    Operator,
    OperatorEnv,
    OperatorParams,
    record_provenance,
    ProvenanceRecord,
)

@runtime_checkable
class FragmentRenderer(Protocol):
    def render(self, fragment: Any, env: OperatorEnv, **kwargs: Any) -> bytes: ...

class ComposeOperator(Operator):
    """
    Aggregates fragments and renders them back to a composite surface representation.
    Satisfies COMPOSE step in the 10-step ingestion Petri Net.
    """

    def __init__(self, renderer: Optional[FragmentRenderer] = None):
        self.renderer = renderer

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> bytes:
        """
        Input: Normalized fragments + hierarchy (backbones) or raw content.
        Output: Reconstructed artifact bytes.
        """
        
        # 1. Handle Fragment Set (from LIFT or NORMALIZE)
        if isinstance(fragment, dict) and "propositions" in fragment and "deltas" in fragment:
            return self._reconstruct_from_fragments(fragment, env)

        # 2. Handle Renderer Protocol
        if self.renderer:
            return self.renderer.render(fragment, env, **params.parameters)

        # 3. Fallback/Mock for testing: basic concatenation of contents if they're bytes
        if isinstance(fragment, list):
            return b"".join([str(f).encode("utf-8") if not isinstance(f, bytes) else f for f in fragment])
        
        if isinstance(fragment, bytes):
            return fragment
            
        return str(fragment).encode("utf-8")

    def _reconstruct_from_fragments(self, fragment_set: Dict[str, Any], env: OperatorEnv) -> bytes:
        """
        Implements Option B reconstruction: Join propositions + Apply Phrasing Deltas.
        """
        # A. Join propositions (logical merge)
        props = fragment_set.get("propositions", [])
        # We assume they are sorted if they are in a list, or we might need to use backbone order.
        # For now, we follow the test logic:
        joined_text = "".join([p["content"] for p in props])

        # B. Apply Phrasing Deltas
        deltas = fragment_set.get("deltas", [])
        if not deltas:
            return joined_text.encode("utf-8")

        # In current LiftOperator, we generate one delta per prose unit.
        # For the prototype, we apply the first delta to the whole joined text.
        # Stage 2 will handle multiple deltas matched to backbones.
        from modelado.operators.monadic import ApplyOperator
        
        apply_op = ApplyOperator()
        # The delta content is expected to have 'ops'
        delta_content = deltas[0]["content"]
        ops = delta_content.get("ops", [])
        
        reconstructed_text = apply_op.apply(
            joined_text,
            OperatorParams(name="compose_apply", parameters={
                "delta": ops,
                "delta_type": "text"
            }),
            env
        )
        
        return reconstructed_text.encode("utf-8")

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)
