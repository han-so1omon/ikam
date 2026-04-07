from __future__ import annotations

import asyncio
import hashlib
import json
import threading
from typing import Any

from pydantic import BaseModel
from openai.lib._pydantic import to_strict_json_schema

from modelado.oraculo.ai_client import GenerateRequest
from modelado.oraculo.factory import create_ai_client_from_env

from mcp_ikam.contracts import (
    GenerationProvenance,
    MapDNA,
    MapGenerationRequest,
    MapGenerationResponse,
    SegmentAnchor,
    SegmentCandidate,
    TraceEvent,
)


def _run_async(coro: Any) -> Any:
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return asyncio.run(coro)

    result_holder: dict[str, Any] = {}
    error_holder: dict[str, BaseException] = {}

    def _runner() -> None:
        try:
            result_holder["value"] = asyncio.run(coro)
        except BaseException as exc:  # pragma: no cover - defensive
            error_holder["error"] = exc

    thread = threading.Thread(target=_runner, daemon=True)
    thread.start()
    thread.join()
    if "error" in error_holder:
        raise error_holder["error"]
    return result_holder.get("value")


def _compute_subgraph_dna(map_subgraph: dict[str, Any]) -> MapDNA:
    nodes = map_subgraph.get("nodes")
    rels = map_subgraph.get("relationships")
    if not isinstance(nodes, list):
        nodes = []
    if not isinstance(rels, list):
        rels = []
    node_hashes: list[str] = []
    for node in nodes:
        if not isinstance(node, dict):
            continue
        payload = {
            "id": str(node.get("id", "")),
            "title": str(node.get("title", "")),
            "kind": str(node.get("kind", "")),
        }
        digest = hashlib.sha256(
            json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
        ).hexdigest()[:16]
        node_hashes.append(digest)
    rel_hashes: list[str] = []
    for rel in rels:
        if not isinstance(rel, dict):
            continue
        payload = {
            "type": str(rel.get("type", "")),
            "source": str(rel.get("source", "")),
            "target": str(rel.get("target", "")),
        }
        digest = hashlib.sha256(
            json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
        ).hexdigest()[:16]
        rel_hashes.append(digest)
    all_hashes = node_hashes + rel_hashes
    fingerprint = hashlib.sha256("".join(all_hashes).encode("utf-8")).hexdigest()
    return MapDNA(fingerprint=fingerprint, structural_hashes=all_hashes, version="1")


def _build_fallback_subgraph(request: MapGenerationRequest) -> dict[str, Any]:
    root_id = f"map:{request.artifact_bundle.corpus_id}:root"
    nodes = [{"id": root_id, "title": request.artifact_bundle.corpus_id, "kind": "corpus"}]
    relationships: list[dict[str, str]] = []
    for artifact in request.artifact_bundle.artifacts:
        node_id = f"map:artifact:{artifact.artifact_id}"
        nodes.append(
            {
                "id": node_id,
                "title": artifact.file_name or artifact.artifact_id,
                "kind": "artifact",
            }
        )
        relationships.append({"type": "map_contains", "source": root_id, "target": node_id})
    return {"root_node_id": root_id, "nodes": nodes, "relationships": relationships}


def _normalize_map_subgraph(request: MapGenerationRequest, payload: dict[str, Any]) -> dict[str, Any]:
    map_subgraph = payload.get("map_subgraph")
    if isinstance(map_subgraph, dict):
        return map_subgraph
    return _build_fallback_subgraph(request)


def _normalize_segment_candidates(
    request: MapGenerationRequest,
    map_subgraph: dict[str, Any],
    payload: dict[str, Any],
) -> list[SegmentCandidate]:
    raw = payload.get("segment_candidates")
    if isinstance(raw, list):
        return [SegmentCandidate.model_validate(item) for item in raw]
    candidates: list[SegmentCandidate] = []
    root_id = map_subgraph.get("root_node_id")
    nodes = map_subgraph.get("nodes") if isinstance(map_subgraph.get("nodes"), list) else []
    for node in nodes:
        if not isinstance(node, dict):
            continue
        node_id = str(node.get("id") or "")
        if not node_id or node_id == root_id:
            continue
        candidates.append(
            SegmentCandidate(
                segment_id=node_id,
                title=str(node.get("title") or node_id),
                artifact_ids=[artifact.artifact_id for artifact in request.artifact_bundle.artifacts],
                rationale="derived from map_subgraph nodes",
            )
        )
    return candidates


def _normalize_segment_anchors(payload: dict[str, Any]) -> dict[str, list[SegmentAnchor]]:
    raw = payload.get("segment_anchors")
    anchors: dict[str, list[SegmentAnchor]] = {}
    if isinstance(raw, dict):
        for segment_id, values in raw.items():
            if isinstance(values, list):
                anchors[str(segment_id)] = [SegmentAnchor.model_validate(item) for item in values]
        return anchors
    if isinstance(raw, list):
        for item in raw:
            if not isinstance(item, dict):
                continue
            segment_id = item.get("segment_id")
            values = item.get("anchors")
            if segment_id and isinstance(values, list):
                anchors[str(segment_id)] = [SegmentAnchor.model_validate(v) for v in values]
        return anchors
    return {}
def _normalize_profile_candidates(
    request: MapGenerationRequest,
    candidates: list[SegmentCandidate],
    payload: dict[str, Any],
) -> dict[str, list[str]]:
    raw = payload.get("profile_candidates")
    normalized: dict[str, list[str]] = {}
    if isinstance(raw, dict):
        for segment_id, values in raw.items():
            if isinstance(values, list) and values:
                normalized[str(segment_id)] = [str(v) for v in values]
        if normalized:
            return normalized
    if isinstance(raw, list):
        for item in raw:
            if not isinstance(item, dict):
                continue
            segment_id = item.get("segment_id")
            values = item.get("profiles")
            if segment_id and isinstance(values, list) and values:
                normalized[str(segment_id)] = [str(v) for v in values]
        if normalized:
            return normalized
    default_profiles = [str(v) for v in request.map_definition.allowed_profiles]
    return {candidate.segment_id: default_profiles for candidate in candidates}

class MapNode(BaseModel):
    id: str
    title: str
    kind: str

class MapRelationship(BaseModel):
    type: str
    source: str
    target: str

class MapSubgraph(BaseModel):
    root_node_id: str
    nodes: list[MapNode]
    relationships: list[MapRelationship]

class SegmentAnchorList(BaseModel):
    segment_id: str
    anchors: list[SegmentAnchor]

class ProfileCandidateList(BaseModel):
    segment_id: str
    profiles: list[str]

class LLMMapPlan(BaseModel):
    map_subgraph: MapSubgraph
    segment_candidates: list[SegmentCandidate]
    segment_anchors: list[SegmentAnchorList]
    profile_candidates: list[ProfileCandidateList]

async def _generate_map_plan_with_llm(payload: dict[str, Any]) -> dict[str, Any]:
    client = create_ai_client_from_env()
    prompt = (
        "Generate a structured map plan. "
        "Use compact semantic grouping and avoid verbatim text dumping. "
        "IMPORTANT: Every segment_id MUST be an existing node id defined in the map_subgraph.nodes list! "
        f"Payload: {json.dumps(payload, ensure_ascii=False)}"
    )
    schema = to_strict_json_schema(LLMMapPlan)
    response = await client.generate(
        GenerateRequest(
            messages=[{"role": "user", "content": prompt}],
            model="gpt-4o-mini",
            temperature=0.2,
            response_format={"type": "json_schema", "json_schema": {"name": "LLMMapPlan", "schema": schema, "strict": True}},
        )
    )
    try:
        body = json.loads(response.text)
    except json.JSONDecodeError as exc:
        raise RuntimeError("mcp-ikam llm returned invalid json") from exc
    if not isinstance(body, dict):
        raise RuntimeError("mcp-ikam llm returned non-object payload")
    body["_provider"] = response.provider
    body["_model"] = response.model
    return body


def generate_structural_map(payload: dict[str, Any]) -> dict[str, Any]:
    request = MapGenerationRequest.model_validate(payload)
    trace_events = [
        TraceEvent(phase="request_validated", message="artifact bundle accepted")
    ]

    try:
        trace_events.append(
            TraceEvent(phase="llm_plan_started", message="llm call started", model="gpt-4o-mini")
        )
        llm_payload = _run_async(_generate_map_plan_with_llm(payload))
        trace_events.append(
            TraceEvent(
                phase="llm_plan_returned",
                message="llm response received",
                provider=str(llm_payload.get("_provider") or "unknown"),
                model=str(llm_payload.get("_model") or "unknown"),
            )
        )
    except Exception as exc:
        raise RuntimeError(f"mcp-ikam map generation failed: {exc}") from exc

    try:
        map_subgraph = _normalize_map_subgraph(request, llm_payload)
        trace_events.append(TraceEvent(phase="map_subgraph_normalized", message="map subgraph normalized"))
        map_dna = _compute_subgraph_dna(map_subgraph)
        segment_candidates = _normalize_segment_candidates(request, map_subgraph, llm_payload)
        trace_events.append(
            TraceEvent(
                phase="segment_candidates_normalized",
                message=f"segment candidates normalized count={len(segment_candidates)}",
            )
        )
        segment_anchors = _normalize_segment_anchors(llm_payload)
        profile_candidates = _normalize_profile_candidates(request, segment_candidates, llm_payload)
        trace_events.append(TraceEvent(phase="validation_completed", message="map response validated"))
        response = MapGenerationResponse(
            map_subgraph=map_subgraph,
            map_dna=map_dna,
            segment_anchors=segment_anchors,
            segment_candidates=segment_candidates,
            profile_candidates=profile_candidates,
            generation_provenance=GenerationProvenance(
                provider=str(llm_payload.get("_provider") or "unknown"),
                model=str(llm_payload.get("_model") or "unknown"),
                prompt_version="map-v2",
                temperature=0.2,
                seed=0,
            ),
            trace_events=trace_events,
        )
    except RuntimeError:
        raise
    except Exception as exc:
        raise RuntimeError(f"mcp-ikam map generation failed: {exc}") from exc
    return response.model_dump(mode="json")
