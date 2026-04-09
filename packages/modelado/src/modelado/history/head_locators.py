from __future__ import annotations

from dataclasses import dataclass

from modelado.environment_scope import EnvironmentScope


@dataclass(frozen=True)
class HeadLocator:
    kind: str
    semantic_id: str
    ref: str | None
    explicit_ref: bool


@dataclass(frozen=True)
class ResolvedArtifactHead:
    semantic_id: str
    ref: str
    head_object_id: str
    head_commit_id: str | None = None


@dataclass(frozen=True)
class ResolvedGraphTarget:
    """Canonical read-time graph target selected by a locator.

    Artifact locators resolve through the selected ref head to the artifact's
    current root fragment. Fragment locators resolve directly to the referenced
    fragment id.
    """

    kind: str
    semantic_id: str
    ref: str
    target_id: str
    target_ref: str


def parse_head_locator(raw: str) -> HeadLocator:
    locator = raw.strip()
    if locator.startswith("artifact://"):
        semantic_id = locator[len("artifact://") :]
        return _build_locator(kind="artifact", semantic_id=semantic_id, ref=None, explicit_ref=False)
    if locator.startswith("fragment://"):
        semantic_id = locator[len("fragment://") :]
        return _build_locator(kind="fragment", semantic_id=semantic_id, ref=None, explicit_ref=False)
    if locator.startswith("subgraph://"):
        semantic_id = locator[len("subgraph://") :]
        return _build_locator(kind="subgraph", semantic_id=semantic_id, ref=None, explicit_ref=False)
    if not locator.startswith("ref://"):
        raise ValueError(f"Invalid head locator: {raw}")
    remainder = locator[len("ref://") :]
    ref, kind, semantic_id = _split_explicit_locator(remainder)
    EnvironmentScope(ref=ref)
    return _build_locator(kind=kind, semantic_id=semantic_id, ref=ref, explicit_ref=True)


def resolve_head_locator(raw: str, *, env_scope: EnvironmentScope) -> HeadLocator:
    locator = parse_head_locator(raw)
    if locator.explicit_ref:
        return locator
    return HeadLocator(
        kind=locator.kind,
        semantic_id=locator.semantic_id,
        ref=env_scope.ref,
        explicit_ref=False,
    )


def canonicalize_locator_ref(raw: str) -> str:
    locator = parse_head_locator(raw)
    if locator.explicit_ref:
        return f"ref://{locator.ref}/{locator.kind}/{locator.semantic_id}"
    return f"{locator.kind}://{locator.semantic_id}"


def try_canonicalize_locator_ref(raw: object, *, kind: str | None = None) -> str | None:
    if not isinstance(raw, str) or not raw:
        return None
    try:
        locator = parse_head_locator(raw)
    except ValueError:
        return None
    if kind is not None and locator.kind != kind:
        return None
    return canonicalize_locator_ref(raw)


def resolve_locator_identity(raw: str, *, fallback_kind: str) -> tuple[str, str]:
    try:
        locator = parse_head_locator(raw)
    except ValueError:
        if raw.startswith(("ref://", "artifact://", "fragment://", "subgraph://")):
            raise
        return fallback_kind, raw
    return locator.kind, canonicalize_locator_ref(raw)


def resolve_artifact_head(raw: str, *, env_scope: EnvironmentScope, cx: object) -> ResolvedArtifactHead:
    return resolve_ref_scoped_artifact_head(raw, env_scope=env_scope, cx=cx)


def resolve_ref_scoped_artifact_head(raw: str, *, env_scope: EnvironmentScope, cx: object) -> ResolvedArtifactHead:
    locator = resolve_head_locator(raw, env_scope=env_scope)
    if locator.kind != "artifact" or locator.ref is None:
        raise ValueError("artifact head locator required")
    branch_name = _branch_name_from_ref(locator.ref)
    row = cx.execute(
        """
        SELECT b.artifact_id,
               b.name AS branch_name,
               b.head_commit_id,
               c.result_ref
          FROM ikam_artifact_branches b
          JOIN ikam_artifact_commits c
            ON c.id = b.head_commit_id
         WHERE b.artifact_id = %s AND b.name = %s
        """,
        (locator.semantic_id, branch_name),
    ).fetchone()
    if not row:
        raise LookupError(f"No artifact head for {locator.semantic_id} in {locator.ref}")
    resolved_artifact_id = row.get("artifact_id") if isinstance(row, dict) else None
    if str(resolved_artifact_id) != locator.semantic_id:
        raise LookupError(f"No artifact head for {locator.semantic_id} in {locator.ref}")
    result_ref = row.get("result_ref") if isinstance(row, dict) else None
    if not isinstance(result_ref, dict):
        raise LookupError(f"No artifact head object for {locator.semantic_id} in {locator.ref}")
    head_object_id = result_ref.get("head_object_id")
    if not isinstance(head_object_id, str) or not head_object_id:
        raise LookupError(f"No artifact head object for {locator.semantic_id} in {locator.ref}")
    return ResolvedArtifactHead(
        semantic_id=str(row["artifact_id"]),
        ref=str(locator.ref),
        head_commit_id=str(row["head_commit_id"]) if row.get("head_commit_id") is not None else None,
        head_object_id=head_object_id,
    )


def resolve_graph_target(raw: str, *, env_scope: EnvironmentScope | None, cx: object) -> ResolvedGraphTarget:
    """Resolve an artifact or fragment locator to a concrete graph target.

    - `artifact://...` and `ref://.../artifact/...` resolve to the selected
      artifact head's `root_fragment_id`
    - `fragment://...` and `ref://.../fragment/...` resolve directly to that
      fragment id
    """

    locator = _resolve_locator_with_optional_scope(raw, env_scope=env_scope)
    if locator.kind == "fragment":
        return ResolvedGraphTarget(
            kind="fragment",
            semantic_id=locator.semantic_id,
            ref=str(locator.ref),
            target_id=locator.semantic_id,
            target_ref=f"ref://{locator.ref}/fragment/{locator.semantic_id}",
        )
    if locator.kind != "artifact":
        raise ValueError("artifact or fragment head locator required")
    resolved = resolve_ref_scoped_artifact_head(raw, env_scope=_required_env_scope(raw, env_scope), cx=cx)
    row = cx.execute(
        """
        SELECT root_fragment_id
          FROM ikam_fragment_objects
         WHERE object_id = %s
        """,
        (resolved.head_object_id,),
    ).fetchone()
    if not row:
        raise LookupError(f"No root fragment for {resolved.semantic_id} in {resolved.ref}")
    root_fragment_id = row["root_fragment_id"] if isinstance(row, dict) else row[0]
    if not isinstance(root_fragment_id, str) or not root_fragment_id:
        raise LookupError(f"No root fragment for {resolved.semantic_id} in {resolved.ref}")
    return ResolvedGraphTarget(
        kind="fragment",
        semantic_id=resolved.semantic_id,
        ref=resolved.ref,
        target_id=root_fragment_id,
        target_ref=f"ref://{resolved.ref}/fragment/{root_fragment_id}",
    )


def resolve_graph_target_input(
    artifact_id: str | None,
    target_ref: str | None,
    env_scope: EnvironmentScope | None,
    cx: object,
) -> ResolvedGraphTarget | None:
    if target_ref:
        return resolve_graph_target(target_ref, env_scope=env_scope, cx=cx)
    if artifact_id and artifact_id.startswith(("artifact://", "fragment://", "ref://")):
        return resolve_graph_target(artifact_id, env_scope=env_scope, cx=cx)
    if artifact_id:
        return ResolvedGraphTarget(
            kind="artifact",
            semantic_id=artifact_id,
            ref=env_scope.ref if env_scope is not None else "",
            target_id=artifact_id,
            target_ref=artifact_id,
        )
    return None


def _split_explicit_locator(remainder: str) -> tuple[str, str, str]:
    for marker, kind in (("/artifact/", "artifact"), ("/fragment/", "fragment"), ("/subgraph/", "subgraph")):
        head, separator, tail = remainder.rpartition(marker)
        if separator:
            return head, kind, tail
    raise ValueError(f"Invalid head locator: {remainder}")


def _build_locator(*, kind: str, semantic_id: str, ref: str | None, explicit_ref: bool) -> HeadLocator:
    semantic = semantic_id.strip()
    if kind not in {"artifact", "fragment", "subgraph"} or not semantic or "/" in semantic:
        raise ValueError("Invalid head locator")
    return HeadLocator(kind=kind, semantic_id=semantic, ref=ref, explicit_ref=explicit_ref)


def _resolve_locator_with_optional_scope(raw: str, *, env_scope: EnvironmentScope | None) -> HeadLocator:
    if raw.strip().startswith("ref://"):
        return parse_head_locator(raw)
    return resolve_head_locator(raw, env_scope=_required_env_scope(raw, env_scope))


def _required_env_scope(raw: str, env_scope: EnvironmentScope | None) -> EnvironmentScope:
    if env_scope is None:
        raise ValueError(f"env_scope is required for shorthand locator: {raw}")
    return env_scope


def _branch_name_from_ref(ref: str) -> str:
    prefix = "refs/heads/"
    if not ref.startswith(prefix):
        raise ValueError(f"Unsupported ref: {ref}")
    branch_name = ref[len(prefix) :]
    if not branch_name:
        raise ValueError(f"Unsupported ref: {ref}")
    return branch_name
