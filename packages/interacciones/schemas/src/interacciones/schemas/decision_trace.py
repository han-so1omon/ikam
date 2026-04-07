"""Decision trace models for audit-friendly routing diagnostics.

These are structured, safe-to-store summaries (not chain-of-thought).
They are designed to match the current agent-dispatcher `decision_trace` shape.
"""

from __future__ import annotations

import json
from typing import Any, Optional, List

from pydantic import BaseModel, ConfigDict, Field


class DecisionTraceQuery(BaseModel):
    domain: Optional[str] = None
    action: Optional[str] = None
    tags: List[str] = Field(default_factory=list)
    require_healthy: bool = True

    model_config = ConfigDict(extra="forbid")


class DecisionTraceCandidate(BaseModel):
    agent_id: str
    score: float
    reasons: List[str] = Field(default_factory=list)
    status: Optional[str] = None
    in_flight: Optional[int] = None

    model_config = ConfigDict(extra="forbid")


class DecisionTrace(BaseModel):
    version: str
    kind: str
    matched_at_ms: int
    query: DecisionTraceQuery
    candidates: List[DecisionTraceCandidate] = Field(default_factory=list)
    selected_agent_id: Optional[str] = None

    model_config = ConfigDict(extra="forbid")


def coerce_decision_trace(raw: Any) -> Optional[DecisionTrace]:
    """Coerce a decision trace from a dict or JSON string.

    This supports the base-api behavior where `metadata.decision_trace` may be a
    JSON string (attached during SSE enrichment).

    Returns None for empty/unsupported values.
    """

    if raw is None or raw == "":
        return None

    if isinstance(raw, (bytes, bytearray)):
        raw = raw.decode("utf-8", errors="replace")

    if isinstance(raw, DecisionTrace):
        return raw

    if isinstance(raw, str):
        try:
            parsed = json.loads(raw)
        except Exception:
            return None
        if not isinstance(parsed, dict):
            return None
        return DecisionTrace.model_validate(parsed)

    if isinstance(raw, dict):
        return DecisionTrace.model_validate(raw)

    return None


def decision_trace_to_json(raw: Any) -> Optional[str]:
    """Normalize a decision trace to a compact JSON string.

    - If given a JSON string, attempts to parse and re-dump compactly.
    - If given a dict or DecisionTrace, dumps compactly.
    - Returns None for empty/unsupported values.
    """

    trace = coerce_decision_trace(raw)
    if trace is not None:
        return json.dumps(trace.model_dump(mode="json"), separators=(",", ":"))

    if isinstance(raw, str) and raw.strip():
        # Preserve as-is if it isn't valid JSON (best-effort).
        return raw

    return None
