from __future__ import annotations

from dataclasses import dataclass
from typing import Protocol, Optional, Dict, Any


@dataclass(frozen=True)
class OperatorEnv:
    seed: int
    renderer_version: str
    policy: str
    model_hash: Optional[str] = None
    variation_id: Optional[str] = None


@dataclass(frozen=True)
class OperatorParams:
    # Concrete operators should extend with specific, typed params
    name: str
    parameters: Dict[str, Any]


@dataclass(frozen=True)
class ProvenanceRecord:
    op_type: str
    params_hash: str
    seed: int
    renderer_version: str
    policy: str
    model_hash: Optional[str]
    variation_id: Optional[str]


class Operator(Protocol):
    """Typed operator interface for deterministic, provenance-tracked transforms.

    Implementations MUST be deterministic given (params, env) and MUST emit
    a ProvenanceRecord to support exact replay.
    """

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> Any:
        ...

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        ...


def params_hash(params: OperatorParams) -> str:
    import hashlib
    import json

    # Stable hash over sorted parameter keys/values
    payload = {"name": params.name, "parameters": params.parameters}
    blob = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.blake2b(blob, digest_size=16).hexdigest()


def record_provenance(params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
    return ProvenanceRecord(
        op_type=params.name,
        params_hash=params_hash(params),
        seed=env.seed,
        renderer_version=env.renderer_version,
        policy=env.policy,
        model_hash=env.model_hash,
        variation_id=env.variation_id,
    )
