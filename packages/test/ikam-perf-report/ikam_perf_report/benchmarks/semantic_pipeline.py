from __future__ import annotations

import asyncio
import hashlib
import json
import os
import re
from typing import Any

from modelado.oraculo.factory import create_ai_client_from_env
from modelado.oraculo.ai_client import GenerateRequest, AIClient
from modelado.oraculo.llm_trace import emit_llm_trace


def _stable_id(*parts: str) -> str:
    seed = ":".join(parts)
    return hashlib.sha256(seed.encode("utf-8")).hexdigest()


def _canonical_text(value: str) -> str:
    return re.sub(r"\s+", " ", value.strip())


class _LLMExtractionError(RuntimeError):
    pass


async def _extract_semantic_graph_with_llm(
    intent: str,
    *,
    model: str,
    ai_client: AIClient | None = None,
    api_key: str | None = None,
) -> dict[str, Any]:
    del api_key
    client = ai_client or create_ai_client_from_env()
    prompt = (
        "You are extracting semantic graph signals for an IKAM performance harness. "
        "Return strict JSON with keys entities and relations. "
        "entities: list of objects {label, kind, confidence, evidence}. "
        "relations: list of objects {source_label, target_label, kind, confidence, rationale, evidence}. "
        "Constraints: relation kinds must be domain-specific (not generic semantic_link), "
        "include concise rationale per relation, and include evidence snippets as list of short strings. "
        "Use only relationships grounded in provided text. "
        f"Text: {intent}"
    )
    request_payload = {
        "temperature": 0.0,
        "messages": [
            {"role": "system", "content": "You return valid JSON only."},
            {"role": "user", "content": prompt},
        ],
        "response_format": {"type": "json_object"},
    }
    emit_llm_trace(
        provider="openai",
        operation="chat.completions",
        model=model,
        phase="request",
        request_payload=request_payload,
        metadata={"component": "semantic_pipeline", "intent_preview": intent[:120]},
    )
    try:
        response = await client.generate(
            GenerateRequest(
                model=model,
                messages=request_payload["messages"],
                temperature=0.0,
                response_format={"type": "json_object"},
                metadata={"component": "semantic_pipeline", "intent_preview": intent[:120]},
            )
        )
    except Exception as exc:
        emit_llm_trace(
            provider="openai",
            operation="chat.completions",
            model=model,
            phase="error",
            metadata={"component": "semantic_pipeline", "error": str(exc)},
        )
        raise
    content = response.text
    emit_llm_trace(
        provider="openai",
        operation="chat.completions",
        model=model,
        phase="response",
        response_payload={"content": content or ""},
        metadata={"component": "semantic_pipeline"},
    )
    if not content:
        raise _LLMExtractionError("LLM returned empty content")
    try:
        payload = json.loads(content)
    except json.JSONDecodeError as exc:
        raise _LLMExtractionError("LLM returned invalid JSON") from exc
    if not isinstance(payload, dict):
        raise _LLMExtractionError("LLM payload was not an object")
    return payload


def _normalize_entities(raw_entities: Any) -> list[dict[str, Any]]:
    if not isinstance(raw_entities, list):
        return []
    entities: list[dict[str, Any]] = []
    seen_labels: set[str] = set()
    for item in raw_entities:
        if isinstance(item, str):
            label = _canonical_text(item)
            kind = "concept"
            confidence = 0.6
            evidence = []
        elif isinstance(item, dict):
            label = _canonical_text(str(item.get("label") or ""))
            kind = _canonical_text(str(item.get("kind") or "concept"))
            try:
                confidence = float(item.get("confidence", 0.6))
            except (TypeError, ValueError):
                confidence = 0.6
            evidence_raw = item.get("evidence")
            evidence = [
                _canonical_text(str(ev))
                for ev in (evidence_raw if isinstance(evidence_raw, list) else [])
                if str(ev).strip()
            ]
        else:
            continue
        if not label:
            continue
        label_key = label.lower()
        if label_key in seen_labels:
            continue
        seen_labels.add(label_key)
        entities.append(
            {
                "id": _stable_id("entity", label_key),
                "label": label,
                "kind": kind,
                "payload": {"source": "llm", "kind": kind},
                "confidence": max(0.0, min(1.0, confidence)),
                "evidence": evidence,
                "referenced_context": [],
            }
        )
    return entities


def _normalize_relations(raw_relations: Any, entities: list[dict[str, Any]]) -> list[dict[str, Any]]:
    if not isinstance(raw_relations, list) or not entities:
        return []
    entity_by_label = {str(entity["label"]).lower(): entity for entity in entities}
    relations: list[dict[str, Any]] = []
    seen: set[str] = set()
    for item in raw_relations:
        if not isinstance(item, dict):
            continue
        source_label = _canonical_text(str(item.get("source_label") or item.get("source") or ""))
        target_label = _canonical_text(str(item.get("target_label") or item.get("target") or ""))
        kind = _canonical_text(str(item.get("kind") or item.get("predicate") or ""))
        rationale = _canonical_text(str(item.get("rationale") or ""))
        evidence_raw = item.get("evidence")
        evidence = [
            _canonical_text(str(ev))
            for ev in (evidence_raw if isinstance(evidence_raw, list) else [])
            if str(ev).strip()
        ]
        if not source_label or not target_label or not kind:
            continue
        if kind.lower() == "semantic_link":
            continue
        source = entity_by_label.get(source_label.lower())
        target = entity_by_label.get(target_label.lower())
        if source is None or target is None:
            continue
        if not rationale or not evidence:
            continue
        key = _stable_id("relation-key", source["id"], target["id"], kind.lower())
        if key in seen:
            continue
        seen.add(key)
        try:
            confidence = float(item.get("confidence", 0.7))
        except (TypeError, ValueError):
            confidence = 0.7
        relations.append(
            {
                "id": _stable_id("relation", source["id"], target["id"], kind.lower()),
                "kind": kind,
                "source": source["id"],
                "target": target["id"],
                "source_label": source["label"],
                "target_label": target["label"],
                "payload": {"source": "llm", "rationale": rationale},
                "confidence": max(0.0, min(1.0, confidence)),
                "evidence": evidence,
                "rationale": rationale,
                "referenced_context": [],
            }
        )
    return relations


def run_semantic_pipeline(intent: str) -> dict[str, list[dict[str, Any]]]:
    model_name = os.getenv("LLM_MODEL", "").strip()
    if not model_name:
        raise RuntimeError("LLM_MODEL missing: semantic pipeline requires unified LLM config")
    payload = asyncio.run(_extract_semantic_graph_with_llm(intent, model=model_name))
    entities = _normalize_entities(payload.get("entities"))
    relations = _normalize_relations(payload.get("relations"), entities)
    return {"entities": entities, "relations": relations}
