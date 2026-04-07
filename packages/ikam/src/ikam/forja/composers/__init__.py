"""Composition strategies for the reconstruction pipeline.

Strategy dispatcher routes strategy names to their implementations.
"""
from __future__ import annotations

from typing import Callable

from ikam.forja.composers.overlay import overlay_compose
from ikam.forja.composers.format_strategy import format_compose
from ikam.forja.composers.concatenate import concatenate_compose

_STRATEGIES: dict[str, Callable] = {
    "overlay": overlay_compose,
    "concatenate": concatenate_compose,
    "format": format_compose,
}


def dispatch_strategy(strategy: str) -> Callable:
    """Return the composer function for a strategy name.

    Raises:
        ValueError: if the strategy is not registered.
    """
    fn = _STRATEGIES.get(strategy)
    if fn is None:
        raise ValueError(
            f"Unknown composition strategy: {strategy!r}. "
            f"Available: {sorted(_STRATEGIES)}"
        )
    return fn
