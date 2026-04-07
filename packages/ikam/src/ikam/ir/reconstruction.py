"""ReconstructionProgram — describes how to compose fragments back into surface bytes."""
from __future__ import annotations

from typing import Any, Dict, List

from pydantic import BaseModel, Field

from ikam.fragments import Fragment
from ikam.forja.cas import cas_fragment
from ikam.ir.mime_types import RECONSTRUCTION_PROGRAM


class CompositionStep(BaseModel):
    """One step in a ReconstructionProgram."""
    strategy: str  # "overlay", "concatenate", "instantiate", "format", "transform", "generate"
    inputs: Dict[str, Any] = Field(default_factory=dict)  # CAS IDs or inline parameters


class ReconstructionProgram(BaseModel):
    """Describes HOW to compose fragments into surface bytes.

    Stored as a Fragment with MIME application/ikam-reconstruction-program+json.
    The Renderer protocol executes this program.
    """
    steps: List[CompositionStep] = Field(default_factory=list)
    renderer_version: str = "1.0.0"
    output_mime_type: str = "application/octet-stream"


def program_to_fragment(program: ReconstructionProgram) -> Fragment:
    """Wrap a ReconstructionProgram as a CAS-addressed Fragment."""
    return cas_fragment(program.model_dump(mode="json"), RECONSTRUCTION_PROGRAM)


def render_program(
    program: ReconstructionProgram,
    fragment_store: Dict[str, Fragment],
) -> bytes:
    """Execute a ReconstructionProgram against a fragment store, returning bytes.

    Iterates steps in order. Currently supports:
      - concatenate: inputs["fragment_ids"] → concatenate bytes_b64 in order
      - overlay: inputs["base_cas_id"], inputs["delta_cas_id"] → overlay merge
      - format: inputs["raw_cas_id"], inputs["format_spec_cas_id"] → format apply

    Raises:
        ValueError: if a step uses an unknown strategy.
    """
    from ikam.forja.composers import dispatch_strategy
    from ikam.forja.composers.concatenate import concatenate_compose

    result = b""
    for step in program.steps:
        # Validate strategy is known (raises ValueError if not)
        dispatch_strategy(step.strategy)

        if step.strategy == "concatenate":
            fragment_ids = step.inputs.get("fragment_ids", [])
            result = concatenate_compose(fragment_ids, fragment_store)
        elif step.strategy == "overlay":
            from ikam.forja.composers.overlay import overlay_compose
            base = fragment_store[step.inputs["base_cas_id"]]
            delta = fragment_store[step.inputs["delta_cas_id"]]
            merged = overlay_compose(base, delta)
            # Store result back for chained steps
            fragment_store[merged.cas_id] = merged
            # Overlay returns a Fragment, extract bytes if this is the last step
            if merged.value is not None:
                import json
                result = json.dumps(merged.value, sort_keys=True).encode("utf-8")
        elif step.strategy == "format":
            from ikam.forja.composers.format_strategy import format_compose
            raw = fragment_store[step.inputs["raw_cas_id"]]
            fmt_spec = fragment_store[step.inputs["format_spec_cas_id"]]
            formatted = format_compose(raw, fmt_spec)
            fragment_store[formatted.cas_id] = formatted
            if formatted.value is not None:
                import json
                result = json.dumps(formatted.value, sort_keys=True).encode("utf-8")

    return result
