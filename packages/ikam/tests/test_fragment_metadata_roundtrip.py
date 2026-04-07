from __future__ import annotations

from dataclasses import dataclass, field

from modelado.operators.affine_text_map import AffineTextMap, AffineTextParams
from modelado.operators import OperatorEnv
from modelado.environment_scope import EnvironmentScope


@dataclass
class Fragment:
    content: str
    metadata: dict = field(default_factory=dict)

    def model_dump(self):
        return {"content": self.content, "metadata": dict(self.metadata)}


def test_metadata_operators_roundtrip():
    frag = Fragment(content="  Hello   WORLD  ")

    # Apply deterministic operator to attach provenance
    params = AffineTextParams(lower=True, trim=True, collapse_whitespace=True).to_operator_params()
    env = OperatorEnv(seed=7, renderer_version="1.0.0", policy="default", model_hash="hash1", env_scope=EnvironmentScope(ref="refs/heads/run/run/test"))
    op = AffineTextMap()
    frag2 = op.apply(frag, params, env)

    assert isinstance(frag2.metadata, dict)
    assert "operators" in frag2.metadata
    assert frag2.metadata["operators"][0]["op_type"] == "AffineTextMap"

    # Metadata survives simple dump/reload round-trip
    restored = Fragment(**frag2.model_dump())
    assert isinstance(restored.metadata, dict)
    assert "operators" in restored.metadata
    assert restored.metadata["operators"][0]["op_type"] == "AffineTextMap"
