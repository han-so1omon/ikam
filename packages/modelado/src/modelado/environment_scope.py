from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Mapping


def _validate_ref(ref: str) -> str:
    normalized = ref.strip()
    if not normalized.startswith("refs/heads/"):
        raise ValueError(f"Invalid ref: {ref}")
    suffix = normalized[len("refs/heads/") :]
    if not suffix or suffix.startswith("/") or suffix.endswith("/") or "//" in suffix:
        raise ValueError(f"Invalid ref: {ref}")
    return normalized


@dataclass(frozen=True)
class EnvironmentScope:
    ref: str

    def __post_init__(self) -> None:
        object.__setattr__(self, "ref", _validate_ref(str(self.ref)))


def parse_reference_scopes(raw_scopes: Iterable[dict[str, Any]] | None) -> list[EnvironmentScope]:
    scopes: list[EnvironmentScope] = []
    for raw in raw_scopes or []:
        scopes.append(EnvironmentScope(ref=str(raw["ref"])))
    return scopes


def scope_ref_from_qualifiers(properties: Mapping[str, Any] | None) -> str | None:
    if not properties:
        return None
    ref = properties.get("ref")
    if ref is not None:
        try:
            return _validate_ref(str(ref))
        except ValueError:
            return None
    return None


def validate_cross_environment_mutation(
    *,
    target_scope: EnvironmentScope,
    reference_scopes: Iterable[EnvironmentScope],
    delta_intents: Iterable[dict[str, Any]] | None,
) -> None:
    """Require explicit delta intents when mutating with cross-env references."""
    has_cross_env_ref = any(scope != target_scope for scope in reference_scopes)
    if has_cross_env_ref and not list(delta_intents or []):
        raise ValueError("cross-environment mutation requires explicit delta_intents")


def add_scope_qualifiers(
    *,
    properties: dict[str, Any],
    scope: EnvironmentScope,
    pipeline_id: str | None = None,
    pipeline_run_id: str | None = None,
    operation_id: str | None = None,
) -> dict[str, Any]:
    out = dict(properties)
    out["ref"] = scope.ref
    if pipeline_id:
        out["pipelineId"] = pipeline_id
    if pipeline_run_id:
        out["pipelineRunId"] = pipeline_run_id
    if operation_id:
        out["operationId"] = operation_id
    return out
