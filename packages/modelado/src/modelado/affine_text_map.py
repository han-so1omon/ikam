from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Dict

from .operators.core import (
    OperatorEnv,
    OperatorParams,
    ProvenanceRecord,
    Operator,
    params_hash,
    record_provenance,
)

@dataclass(frozen=True)
class AffineTextParams:
    name: str = "AffineTextMap"
    lower: bool = True
    trim: bool = True
    collapse_whitespace: bool = True

    def to_operator_params(self) -> OperatorParams:
        payload: Dict[str, Any] = {
            "lower": self.lower,
            "trim": self.trim,
            "collapse_whitespace": self.collapse_whitespace,
        }
        return OperatorParams(name=self.name, parameters=payload)


class AffineTextMap(Operator):
    """Deterministic text normalization operator.

    Operates only on textual `fragment.content` if present; otherwise no-op.
    Deterministic given params and env; emits complete provenance.
    """

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Any:
        content = getattr(fragment, "content", None)
        if not isinstance(content, str):
            # Persist a provenance record even for no-op to maintain auditability
            self._persist_provenance(fragment, params, env)
            return fragment  # no-op for non-text content

        text = content
        if bool(params.parameters.get("trim", True)):
            text = text.strip()
        if bool(params.parameters.get("collapse_whitespace", True)):
            # Collapse internal whitespace deterministically
            text = " ".join(text.split())
        if bool(params.parameters.get("lower", True)):
            text = text.lower()

        # Create a shallow clone with updated content if possible
        try:
            fragment_dict = fragment.model_dump()  # type: ignore[attr-defined]
            fragment_dict["content"] = text
            # Reconstruct same class for determinism
            fragment = fragment.__class__(**fragment_dict)  # type: ignore[call-arg]
        except Exception:
            # Fallback: mutate if it's a simple object with 'content'
            try:
                setattr(fragment, "content", text)
            except Exception:
                pass

        # Persist provenance for replayability
        self._persist_provenance(fragment, params, env)
        return fragment

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)

    def _persist_provenance(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> None:
        prov = self.provenance(params, env)
        try:
            # Attach under metadata.operators list
            meta = getattr(fragment, "metadata", {}) or {}
            ops = list(meta.get("operators", []))
            ops.append({
                "op_type": prov.op_type,
                "params_hash": prov.params_hash,
                "seed": prov.seed,
                "renderer_version": prov.renderer_version,
                "policy": prov.policy,
                "model_hash": prov.model_hash,
                "variation_id": prov.variation_id,
            })
            meta["operators"] = ops
            setattr(fragment, "metadata", meta)
        except Exception:
            # Best-effort; operators should still be deterministic without metadata
            pass
