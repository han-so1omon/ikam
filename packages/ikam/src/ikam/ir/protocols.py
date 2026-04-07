"""Protocols for the compression/re-render pipeline.

Layer 0 (ikam) defines only the protocols. Implementations live in
Layer 1 (modelado) where external dependencies (LLM clients, embedding
models) are available.
"""
from __future__ import annotations

from typing import Any, Dict, List, Optional, Protocol, runtime_checkable

from ikam.fragments import Fragment


@runtime_checkable
class Lifter(Protocol):
    """Lifts surface fragments into IR fragments.

    Non-deterministic — may use LLM. Fragment → List[Fragment].
    """
    async def lift(self, fragment: Fragment, **kwargs: Any) -> List[Fragment]: ...


@runtime_checkable
class Renderer(Protocol):
    """Renders IR fragments back to surface bytes.

    Deterministic by default. IS the composition executor.
    """
    def render(
        self,
        fragment: Fragment,
        *,
        deterministic: bool = True,
        seed: Optional[int] = None,
        style: Optional[Dict[str, Any]] = None,
    ) -> bytes: ...


@runtime_checkable
class Canonicalizer(Protocol):
    """Best-effort normalization for comparison purposes.

    NOT guaranteed deterministic. Produces a canonical form.
    """
    async def canonicalize(self, fragment: Fragment) -> Fragment: ...


@runtime_checkable
class FragmentEmbedder(Protocol):
    """Embeds a fragment's content into a dense vector."""
    async def embed(self, fragment: Fragment) -> list[float]: ...

    @property
    def dimensions(self) -> int: ...

    @property
    def model_name(self) -> str: ...
