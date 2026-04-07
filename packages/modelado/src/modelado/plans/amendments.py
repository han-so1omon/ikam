from __future__ import annotations

import copy
from typing import Any, Dict, List, Sequence, Union

from .schema import (
    Plan,
    PlanAmendment,
    PlanPatchOp,
    PetriNetEnvelope,
    amendment_cas_id,
    canonicalize_plan_amendment_json,
)

_PETRI_PLAN_PATCH_ROOTS = (
    "/project_id",
    "/scope_id",
    "/title",
    "/goal",
    "/place_fragment_ids",
    "/transition_fragment_ids",
    "/arc_fragment_ids",
    "/initial_marking_fragment_id",
    "/index_fragment_id",
    "/version",
)


def _unescape_json_pointer_token(token: str) -> str:
    return token.replace("~1", "/").replace("~0", "~")


def _parse_json_pointer(path: Any) -> List[str]:
    if not isinstance(path, str) or not path.startswith("/"):
        raise ValueError("path must be a JSON Pointer starting with '/'")
    parts = path.split("/")[1:]
    return [_unescape_json_pointer_token(p) for p in parts]


def _json_set(container: Any, token: str, value: Any, *, create_missing: bool) -> None:
    if isinstance(container, dict):
        if (token not in container) and (not create_missing):
            raise ValueError(f"Path does not exist: {token}")
        container[token] = value
        return

    if isinstance(container, list):
        if token == "-":
            if not create_missing:
                raise ValueError("Path does not exist: -")
            container.append(value)
            return
        try:
            idx = int(token)
        except Exception as exc:
            raise ValueError(f"Invalid array index token: {token}") from exc
        if idx < 0 or idx > len(container):
            raise ValueError(f"Array index out of range: {idx}")
        if create_missing:
            if idx == len(container):
                container.append(value)
            else:
                container.insert(idx, value)
            return

        if idx >= len(container):
            raise ValueError(f"Array index out of range: {idx}")
        container[idx] = value
        return

    raise ValueError("Cannot set value at path: parent is not an object/array")


def _json_remove(container: Any, token: str) -> None:
    if isinstance(container, dict):
        if token not in container:
            raise ValueError(f"Path does not exist: {token}")
        del container[token]
        return

    if isinstance(container, list):
        try:
            idx = int(token)
        except Exception as exc:
            raise ValueError(f"Invalid array index token: {token}") from exc
        if idx < 0 or idx >= len(container):
            raise ValueError(f"Array index out of range: {idx}")
        del container[idx]
        return

    raise ValueError("Cannot remove value at path: parent is not an object/array")


def apply_plan_patch(doc: Dict[str, Any], ops: Sequence[Union[PlanPatchOp, Dict[str, Any]]]) -> Dict[str, Any]:
    """Apply a minimal RFC6902-style JSON Patch (add/replace/remove) deterministically.

    Strictness:
    - The base document must be an object.
    - Only add/replace/remove are supported.
    - 'replace' and 'remove' require the path to exist.
    - For 'add', missing intermediate objects are created (as objects).
    """

    if not isinstance(doc, dict):
        raise ValueError("Base document must be an object")

    out: Dict[str, Any] = copy.deepcopy(doc)

    for i, op in enumerate(ops):
        op_model = op if isinstance(op, PlanPatchOp) else PlanPatchOp.model_validate(op)

        tokens = _parse_json_pointer(op_model.path)
        if not tokens:
            raise ValueError(f"delta[{i}].path is invalid")

        parent: Any = out
        for t in tokens[:-1]:
            if isinstance(parent, dict):
                if t not in parent:
                    if op_model.op != "add":
                        raise ValueError(f"Path does not exist: {op_model.path}")
                    parent[t] = {}
                parent = parent[t]
            elif isinstance(parent, list):
                try:
                    idx = int(t)
                except Exception as exc:
                    raise ValueError(f"Invalid array index token: {t}") from exc
                if idx < 0 or idx >= len(parent):
                    raise ValueError(f"Array index out of range in path: {op_model.path}")
                parent = parent[idx]
            else:
                raise ValueError(f"Path does not exist: {op_model.path}")

        last = tokens[-1]
        if op_model.op == "remove":
            _json_remove(parent, last)
            continue

        if op_model.op in {"add", "replace"}:
            if op_model.op != "add" and op_model.value is None and ("value" not in op_model.model_fields_set):
                raise ValueError(f"delta[{i}].value is required for {op_model.op}")
            _json_set(parent, last, op_model.value, create_missing=(op_model.op == "add"))
            continue

        raise ValueError(f"delta[{i}].op must be one of: add, replace, remove")

    return out


def validate_petri_plan_patch_ops(
    ops: Sequence[Union[PlanPatchOp, Dict[str, Any]]],
) -> None:
    """Validate patch ops against the Petri plan patch surface.

    Allowed patch roots are limited to the PlanNetEnvelope JSON fields.
    This intentionally excludes structural fragment payloads (places/transitions)
    which must be amended via new fragment content, not in-place mutation.
    """

    for i, op in enumerate(ops):
        op_model = op if isinstance(op, PlanPatchOp) else PlanPatchOp.model_validate(op)
        path = op_model.path
        if not isinstance(path, str) or not path.startswith("/"):
            raise ValueError(f"delta[{i}].path must be a JSON Pointer")

        if not any(path == root or path.startswith(root + "/") for root in _PETRI_PLAN_PATCH_ROOTS):
            raise ValueError(
                f"delta[{i}].path '{path}' is not allowed for Petri plan amendments"
            )


def apply_plan_amendment(plan: Union[Plan, Dict[str, Any]], amendment: Union[PlanAmendment, Dict[str, Any]]) -> Plan:
    plan_model = plan if isinstance(plan, Plan) else Plan.model_validate(plan)
    amendment_model = amendment if isinstance(amendment, PlanAmendment) else PlanAmendment.model_validate(amendment)

    base_payload = plan_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    patched = apply_plan_patch(base_payload, amendment_model.delta)

    return Plan.model_validate(patched)


def apply_petri_net_amendment(
    envelope: Union[PetriNetEnvelope, Dict[str, Any]],
    amendment: Union[PlanAmendment, Dict[str, Any]],
) -> PetriNetEnvelope:
    """Apply a PlanAmendment delta to a PetriNetEnvelope.

    The delta surface is restricted to PetriNetEnvelope JSON fields.
    """

    envelope_model = (
        envelope if isinstance(envelope, PetriNetEnvelope) else PetriNetEnvelope.model_validate(envelope)
    )
    amendment_model = (
        amendment if isinstance(amendment, PlanAmendment) else PlanAmendment.model_validate(amendment)
    )

    validate_petri_plan_patch_ops(amendment_model.delta)
    base_payload = envelope_model.model_dump(mode="json", by_alias=True, exclude_none=True)
    patched = apply_plan_patch(base_payload, amendment_model.delta)

    return PetriNetEnvelope.model_validate(patched)


def apply_plan_amendments(
    plan: Union[Plan, Dict[str, Any]],
    amendments: Sequence[Union[PlanAmendment, Dict[str, Any]]],
) -> Plan:
    """Apply a set of amendments in a deterministic order.

    Replay safety / determinism:
    - If an amendment has `amendment_id`, we sort by it.
    - Otherwise, we sort by the amendment CAS id derived from canonical amendment JSON.

    This ensures applying the same set of amendments yields the same plan, regardless
    of arrival order.
    """

    plan_model = plan if isinstance(plan, Plan) else Plan.model_validate(plan)

    amendment_models: List[PlanAmendment] = [
        a if isinstance(a, PlanAmendment) else PlanAmendment.model_validate(a) for a in amendments
    ]

    def _sort_key(am: PlanAmendment) -> str:
        if am.amendment_id:
            return f"id:{am.amendment_id}"
        cas = amendment_cas_id(canonicalize_plan_amendment_json(am))
        return f"cas:{cas}"

    out = plan_model
    for amendment_model in sorted(amendment_models, key=_sort_key):
        out = apply_plan_amendment(out, amendment_model)
    return out
