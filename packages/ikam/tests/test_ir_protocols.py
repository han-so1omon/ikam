# packages/ikam/tests/test_ir_protocols.py
"""Tests for IR protocols (Lifter, Renderer, Canonicalizer, FragmentEmbedder)."""
from typing import Any, Dict, List, Optional


def test_lifter_protocol_is_runtime_checkable():
    from ikam.ir.protocols import Lifter
    from ikam.fragments import Fragment

    class FakeLifter:
        async def lift(self, fragment: Fragment) -> List[Fragment]:
            return []

    assert isinstance(FakeLifter(), Lifter)


def test_renderer_protocol_is_runtime_checkable():
    from ikam.ir.protocols import Renderer
    from ikam.fragments import Fragment

    class FakeRenderer:
        def render(
            self,
            fragment: Fragment,
            *,
            deterministic: bool = True,
            seed: Optional[int] = None,
            style: Optional[Dict[str, Any]] = None,
        ) -> bytes:
            return b""

    assert isinstance(FakeRenderer(), Renderer)


def test_canonicalizer_protocol_is_runtime_checkable():
    from ikam.ir.protocols import Canonicalizer
    from ikam.fragments import Fragment

    class FakeCanonicalize:
        async def canonicalize(self, fragment: Fragment) -> Fragment:
            return fragment

    assert isinstance(FakeCanonicalize(), Canonicalizer)


def test_fragment_embedder_protocol_is_runtime_checkable():
    from ikam.ir.protocols import FragmentEmbedder
    from ikam.fragments import Fragment

    class FakeEmbedder:
        async def embed(self, fragment: Fragment) -> list[float]:
            return [0.0] * 768

        @property
        def dimensions(self) -> int:
            return 768

        @property
        def model_name(self) -> str:
            return "fake"

    embedder = FakeEmbedder()
    assert isinstance(embedder, FragmentEmbedder)
    assert embedder.dimensions == 768
    assert embedder.model_name == "fake"
