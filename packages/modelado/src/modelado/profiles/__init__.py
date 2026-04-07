"""Modeling Profile Registry (Layer 1).

This module defines the versioned Profile IDs used to specialize core IR types
for specific modeling tasks (e.g., Prose, Tabular, Style).

Naming Convention: <layer>/<domain>-<variant>@<version>
"""

PROSE_BACKBONE_V1 = "modelado/prose-backbone@1"
PHRASING_DELTA_V1 = "modelado/phrasing-delta@1"
TABULAR_V1 = "modelado/tabular@1"
STYLE_SUBGRAPH_V1 = "modelado/style-subgraph@1"
REASONING_V1 = "modelado/reasoning@1"

__all__ = [
    "PROSE_BACKBONE_V1",
    "PHRASING_DELTA_V1",
    "TABULAR_V1",
    "STYLE_SUBGRAPH_V1",
    "REASONING_V1",
]
