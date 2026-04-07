from __future__ import annotations

import types

from modelado.operators.affine_text_map import AffineTextMap, AffineTextParams, OperatorEnv
from modelado.environment_scope import EnvironmentScope

_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/run/test")


class DummyFragment:
    def __init__(self, content: str, **kwargs):
        self.content = content
        for k, v in kwargs.items():
            setattr(self, k, v)


def test_deterministic_normalization_and_provenance():
    # Set up params and env
    params = AffineTextParams(lower=True, trim=True, collapse_whitespace=True).to_operator_params()
    env = OperatorEnv(seed=42, renderer_version="1.0.0", policy="default", model_hash="abc123", env_scope=_DEV_SCOPE)

    op = AffineTextMap()

    frag1 = DummyFragment(content="  Hello   WORLD  ")
    frag2 = DummyFragment(content="  Hello   WORLD  ")

    out1 = op.apply(frag1, params, env)
    out2 = op.apply(frag2, params, env)

    assert isinstance(out1, DummyFragment)
    assert isinstance(out2, DummyFragment)
    assert out1.content == out2.content == "hello world"

    prov = op.provenance(params, env)
    assert prov.op_type == "AffineTextMap"
    assert prov.seed == 42
    assert prov.renderer_version == "1.0.0"
    assert prov.policy == "default"
    assert prov.model_hash == "abc123"
    assert isinstance(prov.params_hash, str) and len(prov.params_hash) > 0

    # Provenance persisted to metadata
    assert isinstance(getattr(out1, "metadata", {}), dict)
    assert "operators" in out1.metadata
    assert out1.metadata["operators"][0]["op_type"] == "AffineTextMap"


def test_noop_on_non_text_content():
    params = AffineTextParams(lower=True, trim=True, collapse_whitespace=True).to_operator_params()
    env = OperatorEnv(seed=0, renderer_version="1.0.0", policy="default", env_scope=_DEV_SCOPE)

    op = AffineTextMap()

    # Non-text objects
    class NonText:
        pass

    frag = DummyFragment(content="ok")
    frag_non_text = DummyFragment(content=None)

    out_text = op.apply(frag, params, env)
    out_non_text = op.apply(frag_non_text, params, env)

    assert out_text.content == "ok"
    assert out_non_text is frag_non_text  # no-op retains original object
    # Even for no-op, provenance record should be attached
    assert "operators" in out_non_text.metadata
