"""IKAM configuration types for decomposition and reconstruction.

Migrated from legacy fragment module during V3 Fragment Algebra migration.
These are pure configuration types — they do NOT define fragment identity.
"""

from __future__ import annotations

from enum import Enum
from typing import Annotated, List, Optional

from pydantic import BaseModel, ConfigDict, Field


class DecompositionStrategy(str, Enum):
    """Strategy for decomposing artifacts into fragments."""
    SEMANTIC = "semantic"
    STRUCTURAL = "structural"
    FIXED_SIZE = "fixed-size"
    HYBRID = "hybrid"


class DecompositionConfig(BaseModel):
    """Configuration for artifact decomposition."""
    strategy: DecompositionStrategy = DecompositionStrategy.SEMANTIC
    max_depth: Annotated[
        int,
        Field(
            ge=1,
            le=10,
            description="Preferred: maximum decomposition depth (1=root-only)",
        ),
    ] | None = None
    target_levels: int = Field(
        3,
        ge=1,
        le=10,
        description="Deprecated alias for max_depth (maximum decomposition depth)",
    )
    max_tokens_per_fragment: int = Field(
        700, ge=100, le=4000, description="Max tokens per fragment"
    )
    overlap_tokens: int = Field(150, ge=0, le=500, description="Overlap between fragments")
    preserve_structure: bool = Field(
        True, description="Maintain document/slide structural boundaries"
    )

    def effective_max_depth(self) -> int:
        """Return the configured maximum decomposition depth."""
        return self.max_depth if self.max_depth is not None else self.target_levels

    model_config = ConfigDict(populate_by_name=True)


class ReconstructionConfig(BaseModel):
    """Configuration for fragment reconstruction.

    Note: In V3, render_policy and render_levels are unused — reconstruction
    is driven by the root relation DAG. These fields are retained for backward
    compatibility with callers that pass them (they are ignored).
    """
    render_levels: Optional[List[int]] = Field(
        None,
        description="Deprecated: V3 reconstruction ignores level filtering",
    )
    target_format: str = Field("ikam-document", description="Output format")
    include_metadata: bool = Field(True, description="Include provenance metadata")
    validate_integrity: bool = Field(True, description="Validate byte-level equality")

    model_config = ConfigDict(populate_by_name=True)
