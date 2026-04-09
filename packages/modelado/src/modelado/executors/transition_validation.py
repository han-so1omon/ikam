from __future__ import annotations

import re
from typing import Any

from ikam.inspection import node_id_for
from modelado.history.head_locators import resolve_locator_identity


def resolve_target_value(payload: dict[str, Any], target: str) -> Any:
    current: Any = payload
    for part in target.split("."):
        if isinstance(current, dict) and part in current:
            current = current[part]
            continue
        return None
    return current


def validate_inline_schema(payload: dict[str, Any], schema_definition: dict[str, Any]) -> tuple[bool, dict[str, Any]]:
    schema_type = str(schema_definition.get("type") or "").strip()
    if schema_type != "object":
        return False, {"reason": "unsupported_schema_type", "schema_type": schema_type}
    required = [str(item) for item in schema_definition.get("required", []) if isinstance(item, str) and item]
    missing = [key for key in required if key not in payload]
    return (len(missing) == 0, {"schema": schema_definition, "missing_keys": missing})


def validate_regex(value: Any, pattern: str, flags: str | None = None) -> tuple[bool, dict[str, Any]]:
    if not isinstance(value, str):
        return False, {"reason": "non_string_value"}
    re_flags = 0
    if isinstance(flags, str) and flags.strip().upper() == "IGNORECASE":
        re_flags |= re.IGNORECASE
    matched = re.search(pattern, value, flags=re_flags) is not None
    return matched, {"pattern": pattern, "flags": flags or ""}


def _copy_json_object(value: Any) -> dict[str, Any]:
    return dict(value) if isinstance(value, dict) else {}


def _fragment_name(value: dict[str, Any], fragment_ref: str) -> str:
    for candidate in (value.get("name"), value.get("filename"), value.get("file_name"), value.get("document_id")):
        if isinstance(candidate, str) and candidate:
            return candidate
    return fragment_ref


def _inspection_refs(value: dict[str, Any], entry: dict[str, Any]) -> list[str]:
    for key in ("document_refs", "extraction_refs", "entity_relationship_refs", "claim_refs"):
        refs = value.get(key)
        if isinstance(refs, list):
            return [str(item) for item in refs if isinstance(item, str) and item]
    fragment_ref = entry.get("fragment_id") or entry.get("cas_id")
    return [str(fragment_ref)] if isinstance(fragment_ref, str) and fragment_ref else []


def _inspection_value_kind(value: dict[str, Any]) -> str:
    return str(value.get("kind") or value.get("document_id") and "loaded_document" or "value")


def _inspection_summary(value_kind: str, value: dict[str, Any]) -> str:
    if value_kind == "url":
        location = value.get("location")
        if isinstance(location, str) and location:
            return f"url {location}"
    identifier = value.get("document_id") or value.get("segment_id")
    if isinstance(identifier, str) and identifier:
        return f"{value_kind} {identifier}"
    refs = _inspection_refs(value, {})
    if refs:
        label = "ref" if len(refs) == 1 else "refs"
        return f"{value_kind} {len(refs)} {label}"
    return value_kind


def _inspection_payload(entry: dict[str, Any], ref_index: dict[str, dict[str, Any]]) -> dict[str, Any]:
    value = _copy_json_object(entry.get("value"))
    value_kind = _inspection_value_kind(value)
    refs = _inspection_refs(value, entry)
    resolved_refs = [dict(ref_index[ref]) for ref in refs if ref in ref_index]
    return {
        "value_kind": value_kind,
        "summary": _inspection_summary(value_kind, value),
        "refs": refs,
        "content": value,
        "resolved_refs": resolved_refs,
    }


def _inspection_identity(entry: dict[str, Any], value: dict[str, Any]) -> tuple[str, str] | None:
    subgraph_ref = value.get("subgraph_ref")
    if isinstance(subgraph_ref, str) and subgraph_ref:
        return resolve_locator_identity(subgraph_ref, fallback_kind="subgraph")
    cas_id = entry.get("cas_id") or entry.get("fragment_id")
    if isinstance(cas_id, str) and cas_id:
        return "fragment", cas_id
    artifact_id = value.get("artifact_head_ref") or value.get("location")
    if isinstance(artifact_id, str) and artifact_id:
        return resolve_locator_identity(artifact_id, fallback_kind="artifact")
    return None


def _inspection_label(value_kind: str, value: dict[str, Any], identity: str) -> str:
    for candidate in (value.get("location"), value.get("document_id"), value.get("segment_id"), value.get("kind")):
        if isinstance(candidate, str) and candidate:
            return candidate
    return identity


def _inspection_stub(entry: dict[str, Any], ref_index: dict[str, dict[str, Any]]) -> dict[str, Any] | None:
    value = _copy_json_object(entry.get("value"))
    identity = _inspection_identity(entry, value)
    if identity is None:
        return None
    inspection_kind, inspection_id = identity
    value_kind = _inspection_value_kind(value)
    return {
        "id": node_id_for(inspection_kind, {f"{inspection_kind}_id" if inspection_kind == "artifact" else "cas_id" if inspection_kind == "fragment" else "subgraph_ref": inspection_id}),
        "kind": inspection_kind,
        "ir_kind": value_kind,
        "label": _inspection_label(value_kind, value, inspection_id),
        "summary": _inspection_summary(value_kind, value),
        "inspection_ref": f"inspect://{inspection_kind}/{inspection_id}",
    }


def _build_ref_index(*pools: dict[str, Any]) -> dict[str, dict[str, Any]]:
    index: dict[str, dict[str, Any]] = {}
    for pool in pools:
        for value in pool.values():
            if not isinstance(value, list):
                continue
            for item in value:
                if not isinstance(item, dict):
                    continue
                ref = item.get("fragment_id") or item.get("cas_id")
                if not isinstance(ref, str) or not ref or ref in index:
                    continue
                index[ref] = {
                    "fragment_id": item.get("fragment_id"),
                    "cas_id": item.get("cas_id"),
                    "mime_type": item.get("mime_type"),
                    "name": _fragment_name(_copy_json_object(item.get("value")), ref),
                    "inspection_ref": f"inspect://fragment/{ref}",
                    "value": _copy_json_object(item.get("value")),
                }
    return index


def _with_inspection(entries: dict[str, Any], *, ref_index: dict[str, dict[str, Any]]) -> dict[str, Any]:
    resolved: dict[str, Any] = {}
    for key, value in entries.items():
        if not isinstance(value, list):
            continue
        enriched: list[dict[str, Any]] = []
        for item in value:
            if not isinstance(item, dict):
                continue
            enriched_item = dict(item)
            enriched_item["inspection"] = _inspection_payload(enriched_item, ref_index)
            inspection_stub = _inspection_stub(enriched_item, ref_index)
            if inspection_stub is not None:
                enriched_item["inspection_stub"] = inspection_stub
            enriched.append(enriched_item)
        resolved[key] = enriched
    return resolved


def build_transition_validation(
    *,
    validators: list[dict[str, Any]],
    runtime_inputs: dict[str, Any],
    runtime_outputs: dict[str, Any],
) -> dict[str, Any] | None:
    if not validators:
        return None

    resolved_inputs = {
        key: list(value)
        for key, value in runtime_inputs.items()
        if isinstance(value, list)
    }
    resolved_outputs = {
        key: list(value)
        for key, value in runtime_outputs.items()
        if isinstance(value, list)
    }

    allowed_input_keys = {
        str(validator.get("selector") or "").split(".", 1)[1]
        for validator in validators
        if validator.get("direction") == "input" and "." in str(validator.get("selector") or "")
    }
    allowed_output_keys = {
        str(validator.get("selector") or "").split(".", 1)[1]
        for validator in validators
        if validator.get("direction") == "output" and "." in str(validator.get("selector") or "")
    }
    if allowed_input_keys:
        resolved_inputs = {key: value for key, value in resolved_inputs.items() if key in allowed_input_keys}
    if allowed_output_keys:
        resolved_outputs = {key: value for key, value in resolved_outputs.items() if key in allowed_output_keys}
    ref_index = _build_ref_index(runtime_inputs, runtime_outputs)
    resolved_inputs = _with_inspection(resolved_inputs, ref_index=ref_index)
    resolved_outputs = _with_inspection(resolved_outputs, ref_index=ref_index)

    results: list[dict[str, Any]] = []
    for spec in validators:
        direction = spec.get("direction")
        pool = resolved_inputs if direction == "input" else resolved_outputs
        selector = str(spec.get("selector") or "")
        selector_key = selector.split(".", 1)[1] if "." in selector else selector
        candidates = pool.get(selector_key, [])

        matched_ids: list[str] = []
        status = "failed"
        evidence: dict[str, Any] = {"matched_count": len(candidates)}
        if candidates:
            candidate = candidates[0]
            matched_ids.append(str(candidate.get("fragment_id") or candidate.get("cas_id") or selector))
            if spec.get("kind") == "type":
                status_ok, details = validate_inline_schema(
                    candidate.get("value") if isinstance(candidate.get("value"), dict) else {},
                    (spec.get("config") or {}).get("schema") if isinstance((spec.get("config") or {}).get("schema"), dict) else {},
                )
            else:
                status_ok, details = validate_regex(
                    resolve_target_value(candidate, str(spec.get("target") or "")),
                    str((spec.get("config") or {}).get("pattern") or ""),
                    (spec.get("config") or {}).get("flags"),
                )
            status = "passed" if status_ok else "failed"
            evidence.update(details)

        results.append(
            {
                "name": spec.get("name"),
                "direction": direction,
                "kind": spec.get("kind"),
                "status": status,
                "matched_fragment_ids": matched_ids,
                "evidence": evidence,
            }
        )

    return {
        "specs": validators,
        "resolved_inputs": resolved_inputs,
        "resolved_outputs": resolved_outputs,
        "results": results,
    }


def build_transition_validation_for_direction(
    *,
    validators: list[dict[str, Any]],
    runtime_inputs: dict[str, Any],
    runtime_outputs: dict[str, Any],
    direction: str,
) -> dict[str, Any] | None:
    filtered = [validator for validator in validators if validator.get("direction") == direction]
    return build_transition_validation(validators=filtered, runtime_inputs=runtime_inputs, runtime_outputs=runtime_outputs)


def _resolve_fragment_payloads(fragment_ids: list[str], environment_fragments: list[dict[str, Any]]) -> list[dict[str, Any]]:
    by_id = {
        str(item.get("id") or item.get("cas_id") or ""): item
        for item in environment_fragments
        if isinstance(item, dict)
    }
    resolved: list[dict[str, Any]] = []
    for fragment_id in fragment_ids:
        payload = by_id.get(fragment_id)
        if not payload:
            continue
        value = payload.get("value") if isinstance(payload.get("value"), dict) else {}
        resolved.append(
            {
                "fragment_id": fragment_id,
                "cas_id": payload.get("cas_id"),
                "mime_type": payload.get("mime_type"),
                "name": _fragment_name(value, fragment_id),
                "inspection_ref": f"inspect://fragment/{fragment_id}",
                "value": value,
            }
        )
    return resolved


def _extract_output_fragment_ids(step_outputs: dict[str, Any]) -> list[str]:
    direct = [str(item) for item in (step_outputs.get("fragment_ids") or []) if isinstance(item, str) and item]
    if direct:
        return direct

    decomposition = step_outputs.get("decomposition")
    structural = None
    if decomposition is not None:
        structural = getattr(decomposition, "structural", None)
        if structural is None and isinstance(decomposition, dict):
            structural = decomposition.get("structural")
    ids = [
        str(getattr(item, "cas_id", None) or getattr(item, "id", None) or "")
        for item in (structural or [])
    ]
    ids = [item for item in ids if item]
    if ids:
        return ids

    candidates = step_outputs.get("map_segment_candidates") if isinstance(step_outputs.get("map_segment_candidates"), list) else []
    ids = [str(item.get("segment_id") or "") for item in candidates if isinstance(item, dict)]
    return [item for item in ids if item]


def _document_set_payload(*, artifact_id: str, fragment_refs: list[str], subgraph_ref: str) -> dict[str, Any]:
    return {
        "kind": "document_set",
        "artifact_head_ref": f"artifact://{artifact_id}",
        "subgraph_ref": subgraph_ref,
        "document_refs": fragment_refs,
    }


def _chunk_extraction_set_payload(*, source_subgraph_ref: str, subgraph_ref: str, extraction_refs: list[str]) -> dict[str, Any]:
    return {
        "kind": "chunk_extraction_set",
        "source_subgraph_ref": source_subgraph_ref,
        "subgraph_ref": subgraph_ref,
        "extraction_refs": extraction_refs,
    }


def _entity_relationship_set_payload(*, source_subgraph_ref: str, subgraph_ref: str, entity_relationship_refs: list[str]) -> dict[str, Any]:
    return {
        "kind": "entity_relationship_set",
        "source_subgraph_ref": source_subgraph_ref,
        "subgraph_ref": subgraph_ref,
        "entity_relationship_refs": entity_relationship_refs,
    }


def _claim_set_payload(*, source_subgraph_ref: str, subgraph_ref: str, claim_refs: list[str]) -> dict[str, Any]:
    return {
        "kind": "claim_set",
        "source_subgraph_ref": source_subgraph_ref,
        "subgraph_ref": subgraph_ref,
        "claim_refs": claim_refs,
    }


def _hot_ref(run_id: str, contract_type: str, step_id: str, fallback: str) -> str:
    if run_id and step_id:
        return f"subgraph://{run_id}-{contract_type.replace('_', '-')}-{step_id}"
    return f"subgraph://{fallback.replace(':', '-').replace('/', '-')}"


def _resolve_chunk_extraction_input_payload(*, artifact_id: str, step_outputs: dict[str, Any]) -> dict[str, Any] | None:
    inputs = step_outputs.get("inputs") if isinstance(step_outputs.get("inputs"), dict) else {}
    chunk_extraction_set_ref = inputs.get("chunk_extraction_set_ref")
    extraction_refs = [str(item) for item in (inputs.get("fragment_ids") or []) if isinstance(item, str) and item]
    if not isinstance(chunk_extraction_set_ref, str) or not chunk_extraction_set_ref or not extraction_refs:
        return None
    return _chunk_extraction_set_payload(
        source_subgraph_ref=f"subgraph://{artifact_id}-document-set",
        subgraph_ref=chunk_extraction_set_ref,
        extraction_refs=extraction_refs,
    )


def _resolve_entity_relationship_input_payload(*, artifact_id: str, step_outputs: dict[str, Any]) -> dict[str, Any] | None:
    inputs = step_outputs.get("inputs") if isinstance(step_outputs.get("inputs"), dict) else {}
    entity_relationship_set_ref = inputs.get("entity_relationship_set_ref")
    entity_relationship_refs = [str(item) for item in (inputs.get("fragment_ids") or []) if isinstance(item, str) and item]
    if not isinstance(entity_relationship_set_ref, str) or not entity_relationship_set_ref or not entity_relationship_refs:
        return None
    return _entity_relationship_set_payload(
        source_subgraph_ref=f"subgraph://{artifact_id}-chunk-extraction-set",
        subgraph_ref=entity_relationship_set_ref,
        entity_relationship_refs=entity_relationship_refs,
    )


def build_runtime_transition_validation(
    *,
    validators: list[dict[str, Any]],
    artifact_id: str,
    mime_type: str,
    fixture_path: str | None,
    run_id: str,
    step_id: str,
    step_outputs: dict[str, Any],
    environment_fragments: list[dict[str, Any]],
) -> dict[str, Any] | None:
    input_document_refs = [
        str(item)
        for item in (((step_outputs.get("inputs") or {}) if isinstance(step_outputs.get("inputs"), dict) else {}).get("document_fragment_refs") or [])
        if isinstance(item, str) and item
    ]
    input_chunk_extraction_set_payload = _resolve_chunk_extraction_input_payload(artifact_id=artifact_id, step_outputs=step_outputs)
    input_entity_relationship_set_payload = _resolve_entity_relationship_input_payload(artifact_id=artifact_id, step_outputs=step_outputs)
    output_document_refs = [str(item) for item in step_outputs.get("document_fragment_refs", []) if isinstance(item, str) and item]
    output_fragment_ids = _extract_output_fragment_ids(step_outputs)

    document_set_ref = _hot_ref(run_id, "document_set", step_id, f"subgraph:{artifact_id}")
    input_document_set_payload = _document_set_payload(artifact_id=artifact_id, fragment_refs=input_document_refs, subgraph_ref=document_set_ref)
    output_document_set_payload = _document_set_payload(artifact_id=artifact_id, fragment_refs=output_document_refs, subgraph_ref=document_set_ref)

    chunk_extraction_set_ref = _hot_ref(run_id, "chunk_extraction_set", step_id, f"subgraph:{artifact_id}:chunk_extraction_set")
    output_chunk_extraction_set_payload = _chunk_extraction_set_payload(
        source_subgraph_ref=document_set_ref,
        subgraph_ref=chunk_extraction_set_ref,
        extraction_refs=output_fragment_ids,
    )

    entity_relationship_set_ref = _hot_ref(run_id, "entity_relationship_set", step_id, f"subgraph:{artifact_id}:entity_relationship_set")
    output_entity_relationship_set_payload = _entity_relationship_set_payload(
        source_subgraph_ref=chunk_extraction_set_ref,
        subgraph_ref=entity_relationship_set_ref,
        entity_relationship_refs=output_fragment_ids,
    )

    claim_set_ref = _hot_ref(run_id, "claim_set", step_id, f"subgraph:{artifact_id}:claim_set")
    output_claim_set_payload = _claim_set_payload(
        source_subgraph_ref=entity_relationship_set_ref,
        subgraph_ref=claim_set_ref,
        claim_refs=output_fragment_ids,
    )

    runtime_inputs = {
        "url": [{"value": {"kind": "url", "location": fixture_path or artifact_id, "mime_type": mime_type}}] if artifact_id else [],
        "document_set": [{"value": input_document_set_payload}] if input_document_refs else [],
        "chunk_extraction_set": [{"value": input_chunk_extraction_set_payload}] if input_chunk_extraction_set_payload else [],
        "entity_relationship_set": [{"value": input_entity_relationship_set_payload}] if input_entity_relationship_set_payload else [],
        "document_fragment_refs": _resolve_fragment_payloads(input_document_refs, environment_fragments),
    }
    runtime_outputs = {
        "document_set": [{"value": output_document_set_payload}] if output_document_refs else [],
        "chunk_extraction_set": [{"value": output_chunk_extraction_set_payload}] if output_fragment_ids else [],
        "entity_relationship_set": [{"value": output_entity_relationship_set_payload}] if output_fragment_ids else [],
        "claim_set": [{"value": output_claim_set_payload}] if output_fragment_ids else [],
        "document_fragment_refs": _resolve_fragment_payloads(output_document_refs, environment_fragments),
        "fragment_ids": _resolve_fragment_payloads(output_fragment_ids, environment_fragments),
    }
    return build_transition_validation(validators=validators, runtime_inputs=runtime_inputs, runtime_outputs=runtime_outputs)


def build_runtime_transition_validation_for_direction(
    *,
    validators: list[dict[str, Any]],
    artifact_id: str,
    mime_type: str,
    fixture_path: str | None,
    run_id: str,
    step_id: str,
    step_outputs: dict[str, Any],
    environment_fragments: list[dict[str, Any]],
    direction: str,
) -> dict[str, Any] | None:
    filtered = [validator for validator in validators if validator.get("direction") == direction]
    return build_runtime_transition_validation(
        validators=filtered,
        artifact_id=artifact_id,
        mime_type=mime_type,
        fixture_path=fixture_path,
        run_id=run_id,
        step_id=step_id,
        step_outputs=step_outputs,
        environment_fragments=environment_fragments,
    )
