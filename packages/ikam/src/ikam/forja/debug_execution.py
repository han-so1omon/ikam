"""Canonical debug execution primitives for compression/re-render pipeline.

This module is the source of truth for canonical step order and deterministic
step execution hooks used by debug controllers.
"""

from __future__ import annotations

from dataclasses import dataclass, field
import os
import sys
import re
import mimetypes
import io
import zipfile
from xml.etree import ElementTree
from types import SimpleNamespace
from typing import Any
from ikam.forja.execution_scope import ExecutionScope
from hashlib import blake2b
from pathlib import Path
from time import perf_counter
from datetime import datetime, timezone


def _trace_log(message: str, *, stderr: bool = False) -> None:
    timestamp = datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")
    print(f"[{timestamp}] {message}", file=sys.stderr if stderr else sys.stdout)

def _deterministic_debug_mode() -> bool:
    return (
        os.getenv("IKAM_PERF_REPORT_TEST_MODE", "0").strip() == "1"
        or bool(os.getenv("PYTEST_CURRENT_TEST"))
    )


def _map_strict_enabled() -> bool:
    return os.getenv("IKAM_MAP_STRICT", "0").strip().lower() in {"1", "true", "yes", "on"}


def _load_mcp_ikam_client_class() -> Any:
    try:
        from mcp_ikam.client import MCPIkamClient

        return MCPIkamClient
    except ImportError:
        for parent in Path(__file__).resolve().parents:
            candidate = parent / "mcp-ikam" / "src"
            if (candidate / "mcp_ikam").exists():
                if str(candidate) not in sys.path:
                    sys.path.insert(0, str(candidate))
                from mcp_ikam.client import MCPIkamClient

                return MCPIkamClient
        raise


def _load_parse_review_runner() -> Any:
    try:
        from mcp_ikam.agent_executor import run_parse_review

        return run_parse_review
    except ImportError:
        for parent in Path(__file__).resolve().parents:
            candidate = parent / "mcp-ikam" / "src"
            interacciones_candidate = parent / "interacciones" / "schemas" / "src"
            if (candidate / "mcp_ikam").exists():
                if str(candidate) not in sys.path:
                    sys.path.insert(0, str(candidate))
                if (interacciones_candidate / "interacciones").exists() and str(interacciones_candidate) not in sys.path:
                    sys.path.insert(0, str(interacciones_candidate))
                from mcp_ikam.agent_executor import run_parse_review

                return run_parse_review
        raise


def _invoke_mcp_map_generation(
    *,
    artifact_id: str,
    assets: list[dict[str, Any]],
    document_fragment_refs: list[str] | None = None,
    available_tools: list[dict[str, Any]] | None = None,
    mapping_mode: str | None = None,
) -> dict[str, Any]:
    MCPIkamClient = _load_mcp_ikam_client_class()
    base_url = os.getenv("IKAM_MCP_MAP_URL", "http://localhost:18081")
    timeout_seconds = float(os.getenv("IKAM_MCP_MAP_TIMEOUT_SECONDS", "90"))
    client = MCPIkamClient(base_url=base_url, timeout_seconds=timeout_seconds)

    artifact_bundle = {"corpus_id": artifact_id, "artifacts": []}
    for asset in assets:
        if not isinstance(asset, dict):
            continue
        aid = str(asset.get("artifact_id") or "").strip()
        if not aid:
            continue
        artifact_bundle["artifacts"].append(
            {
                "artifact_id": aid,
                "file_name": str(asset.get("filename") or aid),
                "mime_type": str(asset.get("mime_type") or ""),
            }
        )
    if not artifact_bundle["artifacts"]:
        artifact_bundle["artifacts"].append(
            {"artifact_id": artifact_id, "file_name": artifact_id, "mime_type": ""}
        )

    payload = {
        "artifact_bundle": artifact_bundle,
        "map_definition": {
            "goal": "Build map subgraph for similarity and lift pipeline",
            "allowed_profiles": [
                "modelado/prose-backbone@1",
                "modelado/reasoning@1",
                "modelado/style-subgraph@1",
            ],
            "max_nodes": 24,
            "max_depth": 3,
        },
        "context": {
            "project_id": artifact_id,
            "case_id": artifact_id,
        },
    }
    if document_fragment_refs is not None:
        payload["document_fragment_refs"] = [str(item) for item in document_fragment_refs if isinstance(item, str) and item]
    if available_tools is not None:
        payload["available_tools"] = available_tools
    if mapping_mode is not None:
        payload["mapping_mode"] = mapping_mode
    return client.generate_structural_map(payload)


def _fragment_text(fragment: Any) -> str:
    if isinstance(fragment, dict):
        for key in ("text", "title", "rationale", "segment_id"):
            value = fragment.get(key)
            if isinstance(value, str) and value.strip():
                return value
        return ""
    value = getattr(fragment, "value", None)
    if isinstance(value, dict):
        text = value.get("text")
        if isinstance(text, str):
            return text
        if {"subject", "predicate", "object"}.issubset(value.keys()):
            return f"{value.get('subject', '')} {value.get('predicate', '')} {value.get('object', '')}".strip()
    if isinstance(value, str):
        return value
    return ""


def _resolve_asset_mime_type(raw_mime_type: str | None, file_name: str | None) -> str:
    """Resolve effective MIME type for ingestion map step.

    If an asset arrives as application/octet-stream, infer concrete MIME from
    filename extension so specialized decomposers (docx/xlsx/pptx) are used.
    """

    declared = (raw_mime_type or "").strip()
    if declared and declared != "application/octet-stream":
        return declared

    candidate_name = (file_name or "").strip()
    if candidate_name:
        if candidate_name.endswith(".docx"):
            return "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        if candidate_name.endswith(".xlsx"):
            return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        if candidate_name.endswith(".pptx"):
            return "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        guessed, _encoding = mimetypes.guess_type(candidate_name)
        if guessed:
            return guessed

    return declared or "application/octet-stream"


def _extract_docx_text(source_bytes: bytes) -> str:
    """Extract plain text from DOCX bytes for structural mapping fallback."""

    with zipfile.ZipFile(io.BytesIO(source_bytes), mode="r") as zf:
        xml_bytes = zf.read("word/document.xml")
    root = ElementTree.fromstring(xml_bytes)
    ns = {"w": "http://schemas.openxmlformats.org/wordprocessingml/2006/main"}
    lines: list[str] = []
    for paragraph in root.findall(".//w:p", ns):
        runs = [node.text or "" for node in paragraph.findall(".//w:t", ns)]
        text = "".join(runs).strip()
        if text:
            lines.append(text)
    return "\n\n".join(lines)


def _asset_log_fields(asset: dict[str, Any], *, fallback_artifact_id: str, fallback_mime_type: str) -> dict[str, str]:
    artifact_id = str(asset.get("artifact_id") or fallback_artifact_id)
    filename = str(asset.get("filename") or artifact_id.rsplit("/", 1)[-1])
    mime_type = _resolve_asset_mime_type(asset.get("mime_type"), filename) or fallback_mime_type
    return {
        "artifact_id": artifact_id,
        "filename": filename,
        "mime_type": mime_type,
    }


def _chunk_stat_line(*, filename: str, segment_count: int, chunk_lengths: list[int]) -> str:
    max_chars = max(chunk_lengths) if chunk_lengths else 0
    total_chars = sum(chunk_lengths)
    return (
        "parse.chunk: asset_chunks "
        f"filename={filename} segments={segment_count} max_chars={max_chars} total_chars={total_chars}"
    )


def _build_documents_for_chunking(
    *,
    state: StepExecutionState,
    assets: list[dict[str, Any]],
    fallback_mime_type: str,
) -> list[dict[str, Any]]:
    existing = state.outputs.get("documents") if isinstance(state.outputs.get("documents"), list) else []
    documents = [item for item in existing if isinstance(item, dict)]
    if documents:
        document_fragment_refs = [
            str(item)
            for item in (state.outputs.get("document_fragment_refs") or [])
            if isinstance(item, str) and item
        ]
        if len(document_fragment_refs) == len(documents):
            enriched_documents: list[dict[str, Any]] = []
            for index, document in enumerate(documents):
                enriched_document = dict(document)
                enriched_document.setdefault("source_document_fragment_id", document_fragment_refs[index])
                enriched_documents.append(enriched_document)
            return enriched_documents
        return documents

    built: list[dict[str, Any]] = []
    if assets:
        for index, asset in enumerate(item for item in assets if isinstance(item, dict)):
            artifact_id = str(asset.get("artifact_id") or state.artifact_id)
            filename = str(asset.get("filename") or artifact_id.rsplit("/", 1)[-1])
            mime_type = _resolve_asset_mime_type(asset.get("mime_type"), filename) or fallback_mime_type
            text = str(asset.get("text") or "")
            if not text.strip():
                continue
            built.append(
                {
                    "id": str(asset.get("document_id") or f"doc-{index}"),
                    "text": text,
                    "metadata": asset.get("metadata") if isinstance(asset.get("metadata"), dict) else {},
                    "artifact_id": artifact_id,
                    "filename": filename,
                    "mime_type": mime_type,
                }
            )
        if built:
            return built

    if state.source_bytes:
        text = state.source_bytes.decode("utf-8", errors="replace")
        return [
            {
                "id": "doc-0",
                "text": text,
                "metadata": {},
                "artifact_id": state.artifact_id,
                "filename": state.artifact_id.rsplit("/", 1)[-1],
                "mime_type": fallback_mime_type,
            }
        ]
    return built


def _join_summary_lines(lines: list[str], *, fallback: str) -> str:
    merged = "\n".join([line.strip() for line in lines if isinstance(line, str) and line.strip()]).strip()
    return merged or fallback


def _build_single_surface_map_payload(
    *, artifact_id: str, surface_fragment_id: str | None, title: str, reason: str | None
) -> tuple[dict[str, Any], list[dict[str, Any]], str, dict[str, str], dict[str, list[str]]]:
    from modelado.plans.mapping import StructuralMap, StructuralMapNode, compute_map_dna

    surface_id = surface_fragment_id or f"{artifact_id}:surface:0"
    root_id = f"map:{artifact_id}:root"
    root = StructuralMapNode(
        id=root_id,
        title=title,
        level=0,
        kind="corpus",
        metadata={"artifact_id": artifact_id, "mode": "fallback"},
        children=[
            StructuralMapNode(
                id=f"map:surface:{surface_id}",
                title="Surface Fragment",
                level=1,
                kind="surface_fragment",
                metadata={
                    "artifact_id": artifact_id,
                    "fragment_id": surface_id,
                    "fallback_reason": reason or "map_generation_failed",
                },
            )
        ],
    )
    smap = StructuralMap(artifact_id=artifact_id, root=root)
    smap.dna = compute_map_dna(smap)
    payload = smap.model_dump(mode="json", by_alias=True)
    outline = [
        {
            "id": root_id,
            "title": title,
            "level": 0,
            "kind": "corpus",
            "parent_id": None,
            "artifact_ids": [artifact_id],
        },
        {
            "id": f"map:surface:{surface_id}",
            "title": "Surface Fragment",
            "level": 1,
            "kind": "surface_fragment",
            "parent_id": root_id,
            "artifact_ids": [artifact_id],
        },
    ]
    surface_node_id = f"map:surface:{surface_id}"
    node_summaries = {
        root_id: _join_summary_lines(
            [f"{title}", f"Fallback structural map generated for {artifact_id}.", f"Reason: {reason or 'map_generation_failed'}"],
            fallback=f"{title} fallback outline.",
        ),
        surface_node_id: "Surface fragment generated under fallback mapping.",
    }
    node_constituents = {
        root_id: [surface_id],
        surface_node_id: [surface_id],
    }
    return payload, outline, root_id, node_summaries, node_constituents


def _synthesize_structural_map_payload(
    *,
    artifact_id: str,
    decomposition_result: Any,
    fragment_artifact_map: dict[str, str],
    artifact_titles: dict[str, str],
) -> tuple[dict[str, Any], list[dict[str, Any]], str, dict[str, str], dict[str, list[str]]]:
    from modelado.plans.mapping import StructuralMap, StructuralMapNode, compute_map_dna

    structural_fragments = list(getattr(decomposition_result, "structural", []))
    fragment_by_id: dict[str, Any] = {}
    for fragment in structural_fragments:
        fid = getattr(fragment, "cas_id", None) or getattr(fragment, "id", None)
        if isinstance(fid, str) and fid:
            fragment_by_id[fid] = fragment

    by_artifact: dict[str, list[str]] = {}
    for fid in fragment_by_id:
        owner = fragment_artifact_map.get(fid, artifact_id)
        by_artifact.setdefault(owner, []).append(fid)
    if not by_artifact:
        by_artifact[artifact_id] = []

    root_id = f"map:{artifact_id}:root"
    root_children: list[StructuralMapNode] = []
    outline_nodes: list[dict[str, Any]] = [
        {
            "id": root_id,
            "title": "Corpus Outline",
            "level": 0,
            "kind": "corpus",
            "parent_id": None,
            "artifact_ids": sorted(by_artifact.keys()),
        }
    ]
    node_summaries: dict[str, str] = {}
    node_constituents: dict[str, list[str]] = {}
    all_surface_texts: list[str] = []
    all_surface_ids: list[str] = []

    for owner_id in sorted(by_artifact.keys()):
        surface_ids = sorted(by_artifact.get(owner_id, []))
        artifact_node_id = f"map:artifact:{owner_id}"
        artifact_title = artifact_titles.get(owner_id, owner_id.rsplit("/", 1)[-1])
        child_nodes: list[StructuralMapNode] = []
        artifact_surface_texts: list[str] = []
        for index, surface_id in enumerate(surface_ids, start=1):
            fragment = fragment_by_id.get(surface_id)
            surface_text = _fragment_text(fragment).strip() if fragment is not None else ""
            text_preview = surface_text[:80]
            node_title = text_preview.strip() or f"Surface {index}"
            surface_node_id = f"map:surface:{surface_id}"
            child_nodes.append(
                StructuralMapNode(
                    id=surface_node_id,
                    title=node_title,
                    level=2,
                    kind="surface_fragment",
                    metadata={"artifact_id": owner_id, "fragment_id": surface_id},
                )
            )
            outline_nodes.append(
                {
                    "id": surface_node_id,
                    "title": node_title,
                    "level": 2,
                    "kind": "surface_fragment",
                    "parent_id": artifact_node_id,
                    "artifact_ids": [owner_id],
                }
            )
            if surface_text:
                artifact_surface_texts.append(surface_text)
                all_surface_texts.append(surface_text)
            all_surface_ids.append(surface_id)
            node_summaries[surface_node_id] = _join_summary_lines(
                [surface_text],
                fallback=f"Surface fragment {index} in {artifact_title}.",
            )
            node_constituents[surface_node_id] = [surface_id]

        root_children.append(
            StructuralMapNode(
                id=artifact_node_id,
                title=artifact_title,
                level=1,
                kind="artifact",
                metadata={"artifact_id": owner_id, "surface_count": len(surface_ids)},
                children=child_nodes,
            )
        )
        outline_nodes.append(
            {
                "id": artifact_node_id,
                "title": artifact_title,
                "level": 1,
                "kind": "artifact",
                "parent_id": root_id,
                "artifact_ids": [owner_id],
            }
        )
        node_summaries[artifact_node_id] = _join_summary_lines(
            [
                f"{artifact_title}",
                f"Artifact {owner_id} contributes {len(surface_ids)} mapped surface fragment(s).",
                *artifact_surface_texts[:3],
            ],
            fallback=f"{artifact_title} mapped into {len(surface_ids)} surface fragment(s).",
        )
        node_constituents[artifact_node_id] = list(surface_ids)

    root = StructuralMapNode(
        id=root_id,
        title="Corpus Outline",
        level=0,
        kind="corpus",
        metadata={"artifact_count": len(by_artifact)},
        children=root_children,
    )
    smap = StructuralMap(artifact_id=artifact_id, root=root)
    smap.dna = compute_map_dna(smap)
    payload = smap.model_dump(mode="json", by_alias=True)
    node_summaries[root_id] = _join_summary_lines(
        [
            "Corpus Outline",
            f"Corpus contains {len(by_artifact)} artifact node(s) and {len(all_surface_ids)} mapped surface fragment(s).",
            *all_surface_texts[:5],
        ],
        fallback=f"Corpus contains {len(by_artifact)} artifact node(s).",
    )
    node_constituents[root_id] = list(all_surface_ids)
    return payload, outline_nodes, root_id, node_summaries, node_constituents


def _deterministic_vector(text: str, *, dim: int = 768) -> list[float]:
    if not text.strip():
        return [0.0] * dim
    tokens = [token for token in re.findall(r"[a-z0-9_]+", text.lower()) if token]
    if not tokens:
        return [0.0] * dim
    vector = [0.0] * dim
    for token in tokens:
        digest = blake2b(token.encode("utf-8"), digest_size=8).digest()
        bucket = int.from_bytes(digest[:4], "big") % dim
        sign = 1.0 if (digest[4] % 2 == 0) else -1.0
        weight = 1.0 + (digest[5] / 255.0)
        vector[bucket] += sign * weight
    return vector


def _extract_planner_metadata(fragments: list[Any]) -> dict[str, Any]:
    for fragment in fragments:
        value = getattr(fragment, "value", None)
        if not isinstance(value, dict):
            continue
        meta = value.get("meta")
        if not isinstance(meta, dict):
            continue
        provider = meta.get("planner_provider")
        model = meta.get("planner_model")
        if provider or model:
            return {
                "planner_provider": provider,
                "planner_model": model,
                "planner_prompt_version": meta.get("planner_prompt_version"),
                "planner_confidence": meta.get("planner_confidence"),
                "planner_section_label": meta.get("section_label"),
            }
    return {
        "planner_provider": None,
        "planner_model": None,
        "planner_prompt_version": None,
        "planner_confidence": None,
        "planner_section_label": None,
    }




@dataclass
class StepExecutionState:
    source_bytes: bytes
    mime_type: str
    artifact_id: str
    assets: list[dict[str, Any]] = field(default_factory=list)
    outputs: dict[str, Any] = field(default_factory=dict)


def _load_single_asset_for_debug_step(*, asset: dict[str, Any], asset_index: int) -> dict[str, Any]:
    from modelado.executors.loaders import run

    artifact_id = str(asset.get("artifact_id") or f"asset-{asset_index}")
    filename = str(asset.get("filename") or artifact_id.rsplit("/", 1)[-1])
    mime_type = _resolve_asset_mime_type(str(asset.get("mime_type") or "application/octet-stream"), filename)
    payload_bytes = asset.get("payload") if isinstance(asset, dict) else None
    if not isinstance(payload_bytes, (bytes, bytearray)):
        payload_bytes = bytes(str(payload_bytes or ""), encoding="utf-8")

    suffix = Path(filename).suffix.lower()
    if suffix == ".json":
        reader_defaults = {
            "reader_key": "json_reader",
            "reader_library": "llama_index.readers.json",
            "reader_method": "JSONReader.load_data",
        }
    else:
        reader_defaults = {
            "reader_key": "simple_directory_reader",
            "reader_library": "llama_index.core",
            "reader_method": "SimpleDirectoryReader.load_data",
        }

    result: dict[str, Any] | None = None
    try:
        for item in run(
            {
                "fragment": {},
                "params": {
                    "raw_bytes": bytes(payload_bytes),
                    "file_name": filename,
                    "mime_type": mime_type,
                    "artifact_id": artifact_id,
                },
            },
            {},
        ):
            if item.get("type") == "result":
                result = dict(item.get("result", {}))
                break
    except Exception as exc:
        return {
            **reader_defaults,
            "status": "error",
            "documents": [],
            "error_message": str(exc),
        }
    if result is None:
        raise RuntimeError(f"load.documents did not return a result for {filename}")
    result.setdefault("reader_key", reader_defaults["reader_key"])
    result.setdefault("reader_library", reader_defaults["reader_library"])
    result.setdefault("reader_method", reader_defaults["reader_method"])
    result.setdefault("status", "success")
    return result




def next_step_name(current_step_name: str, scope: ExecutionScope) -> str:
    dynamic_steps = scope.get_dynamic_execution_steps()
    try:
        idx = dynamic_steps.index(current_step_name)
    except ValueError as exc:
        raise ValueError(f"Unknown pipeline step: {current_step_name}") from exc
    return dynamic_steps[min(idx + 1, len(dynamic_steps) - 1)]


class InjectedVerifyFailure(RuntimeError):
    """Raised when the verify handler detects an injected failure flag."""

    def __init__(self, drift_at: str) -> None:
        self.drift_at = drift_at
        self.injection_used = True
        super().__init__(f"injected_verify_fail: drift_at={drift_at}")


def _append_edges_and_project(
    state: StepExecutionState,
    new_edges: list[dict[str, str]],
    new_nodes: list[dict[str, str]],
) -> None:
    """Append edges to accumulated list and rebuild graph_projection.

    Each step that creates relationships calls this to incrementally build
    the graph.  The projection is always current after the call returns.
    """
    edges = state.outputs.setdefault("edges", [])
    edges.extend(new_edges)

    # Merge new nodes into existing set (deduplicate by id)
    prev_projection = state.outputs.get("graph_projection", {"nodes": [], "edges": []})
    node_id_set = {n["id"] for n in prev_projection["nodes"]}
    merged_nodes = list(prev_projection["nodes"])
    for node in new_nodes:
        if node["id"] not in node_id_set:
            merged_nodes.append(node)
            node_id_set.add(node["id"])

    state.outputs["graph_projection"] = {
        "nodes": merged_nodes,
        "edges": list(edges),  # snapshot of all accumulated edges
        "node_count": len(merged_nodes),
        "edge_count": len(edges),
    }


def _scope_step_execution_metadata(scope: ExecutionScope | None, step_name: str) -> dict[str, Any]:
    if scope is None:
        return {}
    getter = getattr(scope, "get_step_execution_metadata", None)
    if not callable(getter):
        return {}
    metadata = getter(step_name)
    return dict(metadata) if isinstance(metadata, dict) else {}


def _executor_identity_from_step_metadata(step_metadata: dict[str, Any], *, fallback_id: str, fallback_kind: str) -> dict[str, str]:
    executor_id = str(step_metadata.get("executor_id") or fallback_id)
    executor_kind = str(step_metadata.get("executor_kind") or fallback_kind)
    return {
        "executor_id": executor_id,
        "executor_kind": executor_kind,
    }


def _resolve_ingestion_operator_branch(step_name: str, step_metadata: dict[str, Any]) -> str | None:
    operator_id = str(step_metadata.get("operator_id") or "")
    if not operator_id:
        return None

    branch_by_operator = {
        "modelado/operators/load_documents": "load_documents",
        "modelado/operators/chunking": "chunking",
        "modelado/operators/entities_and_relationships": "entities_and_relationships",
        "modelado/operators/claims": "claims",
    }
    return branch_by_operator.get(operator_id)


async def _execute_ingestion_load_documents(
    step_name: str,
    state: StepExecutionState,
    scope: ExecutionScope | None,
    step_metadata: dict[str, Any],
) -> dict[str, Any]:
    if step_name != "parse_artifacts":
        raise RuntimeError(f"unsupported operator for {step_name}: {step_metadata.get('operator_id')}")
    from ikam.forja.cas import cas_fragment as _cas_fragment

    assets = [asset for asset in state.assets if isinstance(asset, dict)]
    if not assets:
        assets = [
            {
                "artifact_id": state.artifact_id,
                "filename": state.artifact_id.rsplit("/", 1)[-1],
                "mime_type": state.mime_type,
                "payload": state.source_bytes,
            }
        ]
    _trace_log(f"load.documents: phase=asset_intake finished assets={len(assets)}")
    for asset in assets:
        filename = str(asset.get("filename") or state.artifact_id.rsplit("/", 1)[-1])
        artifact_id = str(asset.get("artifact_id") or state.artifact_id)
        mime_type = _resolve_asset_mime_type(str(asset.get("mime_type") or state.mime_type), filename)
        _trace_log(
            "load.documents: asset "
            f"filename={filename} mime={mime_type} artifact_id={artifact_id}"
        )

    normalized_documents: list[dict[str, Any]] = []
    document_fragments: list[Any] = []
    document_fragment_refs: list[str] = []
    document_fragment_artifact_map: dict[str, str] = {}
    document_loads: list[dict[str, Any]] = []
    global_index = 0
    loaded_asset_count = 0
    errored_asset_count = 0
    unsupported_asset_count = 0
    reader_summary: dict[str, int] = {}
    for asset_index, asset in enumerate(assets):
        asset_result = _load_single_asset_for_debug_step(asset=asset, asset_index=asset_index)
        filename = str(asset.get("filename") or state.artifact_id.rsplit("/", 1)[-1])
        artifact_id = str(asset.get("artifact_id") or state.artifact_id)
        mime_type = _resolve_asset_mime_type(str(asset.get("mime_type") or state.mime_type), filename)
        reader_key = str(asset_result.get("reader_key") or "simple_directory_reader")
        reader_library = str(asset_result.get("reader_library") or "llama_index.core")
        reader_method = str(asset_result.get("reader_method") or "SimpleDirectoryReader.load_data")
        status = str(asset_result.get("status") or "success")
        error_message = str(asset_result.get("error_message") or "").strip()
        documents = asset_result.get("documents") if isinstance(asset_result.get("documents"), list) else []
        _trace_log(
            "load.documents: asset_reader "
            f"filename={filename} mime={mime_type} artifact_id={artifact_id} reader_key={reader_key} "
            f"reader_library={reader_library} reader_method={reader_method}"
        )
        reader_summary[reader_key] = reader_summary.get(reader_key, 0) + 1
        if status == "unsupported":
            unsupported_asset_count += 1
        elif status == "error":
            errored_asset_count += 1
        elif status == "success":
            loaded_asset_count += 1
        load_record = {
            "artifact_id": artifact_id,
            "filename": filename,
            "mime_type": mime_type,
            "reader_key": reader_key,
            "reader_library": reader_library,
            "reader_method": reader_method,
            "status": status,
            "document_count": len(documents),
        }
        if error_message:
            load_record["error_message"] = error_message
        document_loads.append(load_record)
        if status != "success":
            status_line = (
                "load.documents: asset_status "
                f"filename={filename} artifact_id={artifact_id} status={status} document_count={len(documents)}"
            )
            if error_message:
                status_line = f"{status_line} error={error_message}"
            _trace_log(status_line, stderr=True)
            continue

        for document in documents:
            if not isinstance(document, dict):
                continue
            doc_id = str(document.get("id") or f"doc-{global_index}")
            text = str(document.get("text") or "")
            metadata = document.get("metadata") if isinstance(document.get("metadata"), dict) else {}
            doc_filename = str(metadata.get("file_name") or metadata.get("filename") or filename)
            doc_artifact_id = str(metadata.get("artifact_id") or artifact_id)
            preview = " ".join(text.split())[:120]
            metadata_keys = ",".join(sorted(str(key) for key in metadata.keys())) or "none"
            normalized_documents.append(
                {
                    "id": doc_id,
                    "text": text,
                    "metadata": metadata,
                    "artifact_id": doc_artifact_id,
                    "filename": doc_filename,
                    "mime_type": mime_type,
                    "asset_index": asset_index,
                    "reader_key": reader_key,
                    "reader_method": reader_method,
                }
            )
            document_fragment = _cas_fragment(
                {
                    "document_id": doc_id,
                    "text": text,
                    "metadata": metadata,
                    "artifact_id": doc_artifact_id,
                    "filename": doc_filename,
                    "mime_type": mime_type,
                },
                "application/vnd.ikam.loaded-document+json",
            )
            fragment_ref = getattr(document_fragment, "cas_id", None)
            if isinstance(fragment_ref, str) and fragment_ref:
                document_fragments.append(document_fragment)
                document_fragment_refs.append(fragment_ref)
                document_fragment_artifact_map[fragment_ref] = doc_artifact_id
            _trace_log(
                "load.documents: document "
                f"index={global_index} doc_id={doc_id} artifact_id={doc_artifact_id} filename={doc_filename} "
                f"chars={len(text)} metadata_keys={metadata_keys} preview={preview}"
            )
            global_index += 1

    operation_telemetry = {
        "executor_id": "executor://python-primary",
        "executor_kind": "python-executor",
        "operation_name": "load.documents",
        "strategy": "artifact_loading",
        "operation_library": "multi_reader_dispatch",
        "operation_method": "per_asset_dispatch",
        "reader_dispatch_strategy": "per_asset",
        "document_count": len(normalized_documents),
        "asset_count": len(assets),
        "loaded_asset_count": loaded_asset_count,
        "errored_asset_count": errored_asset_count,
        "unsupported_asset_count": unsupported_asset_count,
        "reader_summary": reader_summary,
    }
    state.outputs["documents"] = normalized_documents
    state.outputs["document_fragments"] = document_fragments
    state.outputs["document_fragment_refs"] = document_fragment_refs
    state.outputs["document_fragment_artifact_map"] = document_fragment_artifact_map
    state.outputs["document_loads"] = document_loads
    state.outputs["operation_telemetry"] = operation_telemetry
    _trace_log(
        "load.documents: summary "
        f"documents={len(normalized_documents)} assets={len(assets)} loaded_assets={loaded_asset_count} "
        f"errored_assets={errored_asset_count} unsupported_assets={unsupported_asset_count}"
    )
    return {
        "executor": "ikam.forja.debug_execution",
        "status": "ok",
        "documents": normalized_documents,
    }


async def _execute_ingestion_chunking(
    step_name: str,
    state: StepExecutionState,
    scope: ExecutionScope | None,
    step_metadata: dict[str, Any],
) -> dict[str, Any]:
    map_status = "ok"
    map_error_reason: str | None = None
    result = SimpleNamespace(structural=[], root_fragments=[], canonical=None)
    new_edges: list[dict[str, Any]] = []
    new_nodes: list[dict[str, Any]] = []
    state.outputs["fragment_artifact_map"] = {}
    state.outputs["asset_decomposition_statuses"] = []
    state.outputs["decomposition_artifact_metrics"] = []
    state.outputs["reconstruction_verdicts"] = []

    assets = [asset for asset in state.assets if isinstance(asset, dict)]
    assets_from_manifest = bool(assets)
    if not assets:
        assets = [
            {
                "artifact_id": state.artifact_id,
                "filename": state.artifact_id.rsplit("/", 1)[-1],
                "mime_type": state.mime_type,
            }
        ]
    for asset in assets:
        aid = str(asset.get("artifact_id") or state.artifact_id)
        new_nodes.append({"id": aid, "type": "artifact"})

    artifact_titles: dict[str, str] = {}
    for asset in assets:
        if not isinstance(asset, dict):
            continue
        aid = str(asset.get("artifact_id") or "")
        if not aid:
            continue
        artifact_titles[aid] = str(asset.get("filename") or aid)
    if not artifact_titles:
        artifact_titles[state.artifact_id] = state.artifact_id.rsplit("/", 1)[-1]

    mapping_mode = state.outputs.get("mapping_mode")
    document_fragment_refs = [
        str(item)
        for item in (state.outputs.get("document_fragment_refs") or [])
        if isinstance(item, str) and item
    ]
    available_tools = []
    if scope:
        available_tools = scope.get_available_tools()
    total_started = perf_counter()
    _trace_log(f"parse.chunk: phase=asset_intake finished assets={len(assets)}")
    asset_log_rows = [
        _asset_log_fields(
            asset,
            fallback_artifact_id=state.artifact_id,
            fallback_mime_type=state.mime_type,
        )
        for asset in assets
    ]
    asset_source = "manifest" if assets_from_manifest else "primary"
    for row in asset_log_rows:
        _trace_log(
            "parse.chunk: asset "
            f"filename={row['filename']} mime={row['mime_type']} artifact_id={row['artifact_id']} source={asset_source}"
        )

    from modelado.environment_scope import EnvironmentScope as ModeladoEnvironmentScope
    from modelado.operators.chunking import ChunkOperator
    from modelado.operators.core import OperatorEnv, OperatorParams
    from modelado.oraculo.factory import create_ai_client_from_env

    documents = _build_documents_for_chunking(
        state=state,
        assets=assets,
        fallback_mime_type=state.mime_type,
    )
    llm_client = None
    try:
        llm_client = create_ai_client_from_env()
    except Exception:
        llm_client = None

    _trace_log(f"parse.chunk: mapping_mode={mapping_mode or 'semantic_relations_only'}")
    _trace_log(
        "parse.chunk: branch=python_native_chunking "
        "planner_external=false chunk_execution_local=true"
    )
    _trace_log(
        "parse.chunk: framework branch=python_native_chunking framework=modelado "
        "operation_library=modelado.operators operation_method=ChunkOperator.apply"
    )
    _trace_log(
        f"parse.chunk: phase=chunking started documents={len(documents)} assets={len(assets)}"
    )
    chunking_started = perf_counter()
    chunk_result = ChunkOperator().apply(
        None,
        OperatorParams(
            name="parse.chunk",
            parameters={
                "artifact_id": state.artifact_id,
                "documents": documents,
            },
        ),
        OperatorEnv(
            seed=0,
            renderer_version="debug",
            policy="strict",
            env_scope=ModeladoEnvironmentScope(ref=f"refs/heads/run/{state.artifact_id}"),
            llm=llm_client,
        ),
    )
    chunking_duration_ms = max(1, int((perf_counter() - chunking_started) * 1000))

    chunks = [item for item in chunk_result.get("chunks", []) if isinstance(item, dict)]
    chunk_fragment_refs = [item for item in chunk_result.get("fragment_ids", []) if isinstance(item, str) and item]
    fragment_artifact_map = chunk_result.get("fragment_artifact_map") if isinstance(chunk_result.get("fragment_artifact_map"), dict) else {}
    document_stats = [item for item in chunk_result.get("document_stats", []) if isinstance(item, dict)]
    document_chunk_sets = [item for item in chunk_result.get("document_chunk_sets", []) if isinstance(item, dict)]
    hydrated_chunk_extraction_set = (
        chunk_result.get("chunk_extraction_set") if isinstance(chunk_result.get("chunk_extraction_set"), dict) else {}
    )
    summary = chunk_result.get("summary") if isinstance(chunk_result.get("summary"), dict) else {}
    llm_boundary_hint = summary.get("llm_boundary_hint")
    operation_telemetry = {
        "executor_id": "executor://python-primary",
        "executor_kind": "python-executor",
        "operation_name": "parse.chunk",
        "branch": "python_native_chunking",
        "status": "success",
        "strategy": "grounded_document_chunking",
        "operation_library": "modelado.operators",
        "operation_method": "ChunkOperator.apply",
        "planner_external": False,
        "chunk_execution_local": True,
        "document_input_mode": "document_set",
        "document_ref_count": len(document_fragment_refs),
        "document_count": len(documents),
        "chunk_count": len(chunks),
        "duration_ms": chunking_duration_ms,
    }
    if isinstance(llm_boundary_hint, str) and llm_boundary_hint.strip():
        operation_telemetry["llm_boundary_hint"] = llm_boundary_hint
    state.outputs["operation_telemetry"] = operation_telemetry
    state.outputs["documents"] = documents
    state.outputs["chunks"] = chunks
    state.outputs["fragment_ids"] = chunk_fragment_refs
    state.outputs["fragment_artifact_map"] = fragment_artifact_map
    state.outputs["document_chunk_sets"] = document_chunk_sets
    state.outputs["chunk_extraction_set"] = hydrated_chunk_extraction_set

    compat_structural: list[Any] = []
    artifact_chunk_lengths: dict[str, list[int]] = {}
    new_edges = []
    new_nodes = []
    seen_artifacts: set[str] = set()
    for row in document_stats:
        artifact_id = str(row.get("artifact_id") or state.artifact_id)
        if artifact_id in seen_artifacts:
            continue
        seen_artifacts.add(artifact_id)
        new_nodes.append({"id": artifact_id, "type": "artifact"})

    for chunk in chunks:
        fragment_id = str(chunk.get("fragment_id") or chunk.get("chunk_id") or "")
        artifact_id = str(chunk.get("artifact_id") or state.artifact_id)
        text = str(chunk.get("text") or "")
        artifact_chunk_lengths.setdefault(artifact_id, []).append(len(text))
        compat_structural.append(
            SimpleNamespace(
                cas_id=fragment_id,
                id=fragment_id,
                mime_type="application/vnd.ikam.chunk-extraction+json",
                value={
                    "text": text,
                    "segment_id": str(chunk.get("chunk_id") or fragment_id),
                    "document_id": str(chunk.get("document_id") or ""),
                },
            )
        )
        new_nodes.append({"id": fragment_id, "type": "surface_fragment"})
        new_edges.append(
            {
                "source": artifact_id,
                "target": fragment_id,
                "predicate": "contains",
                "step": "map.conceptual.lift.surface_fragments",
            }
        )

    from ikam.forja.cas import cas_fragment as _cas_fragment
    import base64 as _b64

    source_fragment = _cas_fragment(
        {"bytes_b64": _b64.b64encode(state.source_bytes).decode()},
        state.mime_type,
    )
    root_fragments: list[Any] = list(compat_structural)
    if getattr(source_fragment, "cas_id", None):
        root_fragments.append(source_fragment)
        new_nodes.append({"id": source_fragment.cas_id, "type": "source_bytes"})
        new_edges.append(
            {
                "source": source_fragment.cas_id,
                "target": state.artifact_id,
                "predicate": "original-bytes-of",
                "step": "map.conceptual.lift.surface_fragments",
            }
        )

    status_rows: list[dict[str, Any]] = []
    diagnostic_rows: list[dict[str, Any]] = []
    for asset in assets:
        aid = str(asset.get("artifact_id") or state.artifact_id)
        filename = str(asset.get("filename") or aid)
        resolved_mime = _resolve_asset_mime_type(asset.get("mime_type"), filename)
        lengths = artifact_chunk_lengths.get(aid, [])
        _trace_log(
            _chunk_stat_line(
                filename=filename,
                segment_count=len(lengths),
                chunk_lengths=lengths,
            )
        )
        status_rows.append(
            {
                "artifact_id": aid,
                "mime_type": resolved_mime,
                "status": "success",
                "reason": "python_native_chunking",
            }
        )
        diagnostic_rows.append(
            {
                "artifact_id": aid,
                "mime_type": resolved_mime,
                "policy_version": "v1",
                "structural_coverage_ratio": 1.0,
                "status": "grounded",
                "planner_provider": None,
                "planner_model": None,
                "planner_prompt_version": None,
                "planner_confidence": 1.0,
                "planner_section_label": "chunking",
                "decomposition_status": "chunked",
                "decomposition_reason": "python_native_chunking",
                "boundary_count": len(lengths),
                "max_chunk_chars": max(lengths) if lengths else 0,
                "chunk_distribution": lengths,
            }
        )

    state.outputs["decomposition"] = SimpleNamespace(
        structural=compat_structural,
        root_fragments=root_fragments,
        canonical=None,
    )
    state.outputs["asset_decomposition_statuses"] = status_rows
    state.outputs["decomposition_artifact_metrics"] = diagnostic_rows
    state.outputs["reconstruction_verdicts"] = []
    _trace_log(
        f"parse.chunk: phase=chunking finished chunk_count={len(chunks)} duration_ms={chunking_duration_ms}"
    )
    _trace_log(f"parse.chunk: chunk_count={len(chunks)}")
    _trace_log(
        f"parse.chunk: summary assets={len(assets)} documents={len(documents)} chunks={len(chunks)} total_duration_ms={max(1, int((perf_counter() - total_started) * 1000))}"
    )
    _append_edges_and_project(state, new_edges, new_nodes)

    return {
        "executor": "ikam.forja.debug_execution",
        "root_fragments": len(root_fragments),
        "has_canonical": False,
        "details": {
            "root_fragment_ids": [
                getattr(item, "cas_id", None)
                for item in root_fragments
                if getattr(item, "cas_id", None)
            ],
            "canonical_mime_type": None,
            "asset_count": len(assets),
            "operation_telemetry": operation_telemetry,
        },
    }


async def _execute_ingestion_entities_and_relationships(
    step_name: str,
    state: StepExecutionState,
    scope: ExecutionScope | None,
    step_metadata: dict[str, Any],
) -> dict[str, Any]:
    executor_identity = _executor_identity_from_step_metadata(
        step_metadata,
        fallback_id="executor://ml-primary",
        fallback_kind="ml-executor",
    )
    from modelado.environment_scope import EnvironmentScope as ModeladoEnvironmentScope
    from modelado.operators import EntitiesAndRelationshipsOperator
    from modelado.operators.core import OperatorEnv, OperatorParams

    inputs = state.outputs.get("inputs") if isinstance(state.outputs.get("inputs"), dict) else {}
    chunk_extraction_set_ref = str(inputs.get("chunk_extraction_set_ref") or "")
    hydrated_chunk_extraction_set = (
        state.outputs.get("chunk_extraction_set") if isinstance(state.outputs.get("chunk_extraction_set"), dict) else {}
    )

    chunk_rows = state.outputs.get("chunks") if isinstance(state.outputs.get("chunks"), list) else []
    chunk_fragment_ids = [item for item in (state.outputs.get("fragment_ids") or []) if isinstance(item, str) and item]

    chunk_extractions: list[dict[str, Any]] = []
    if hydrated_chunk_extraction_set:
        raw_chunk_extractions = hydrated_chunk_extraction_set.get("chunk_extractions")
        if isinstance(raw_chunk_extractions, list):
            chunk_extractions = [item for item in raw_chunk_extractions if isinstance(item, dict)]
    if chunk_rows:
        for index, chunk in enumerate(chunk_rows):
            if not isinstance(chunk, dict):
                continue
            fragment_id = str(chunk.get("fragment_id") or chunk.get("chunk_id") or "")
            if not fragment_id and index < len(chunk_fragment_ids):
                fragment_id = chunk_fragment_ids[index]
            chunk_extractions.append(
                {
                    "cas_id": fragment_id,
                    "mime_type": "application/vnd.ikam.chunk+json",
                    "value": {
                        "chunk_id": str(chunk.get("chunk_id") or fragment_id),
                        "document_id": str(chunk.get("document_id") or ""),
                        "artifact_id": str(chunk.get("artifact_id") or state.artifact_id),
                        "filename": str(chunk.get("filename") or state.artifact_id.rsplit("/", 1)[-1]),
                        "text": str(chunk.get("text") or ""),
                        "span": chunk.get("span") if isinstance(chunk.get("span"), dict) else None,
                        "order": chunk.get("order"),
                    },
                }
            )

    if not chunk_extractions:
        decomposition = state.outputs.get("decomposition")
        structural = list(getattr(decomposition, "structural", [])) if decomposition is not None else []
        for index, fragment in enumerate(structural):
            fragment_id = str(getattr(fragment, "cas_id", None) or getattr(fragment, "id", None) or "")
            value = getattr(fragment, "value", None) if isinstance(getattr(fragment, "value", None), dict) else {}
            chunk_extractions.append(
                {
                    "cas_id": fragment_id,
                    "mime_type": str(getattr(fragment, "mime_type", "application/vnd.ikam.chunk+json")),
                    "value": {
                        "chunk_id": str(value.get("chunk_id") or value.get("segment_id") or fragment_id),
                        "document_id": str(value.get("document_id") or ""),
                        "artifact_id": str(value.get("artifact_id") or state.artifact_id),
                        "filename": str(value.get("filename") or state.artifact_id.rsplit("/", 1)[-1]),
                        "text": str(value.get("text") or ""),
                        "span": value.get("span") if isinstance(value.get("span"), dict) else None,
                        "order": value.get("order"),
                    },
                }
            )

    if not chunk_extractions:
        raise RuntimeError("entities_and_relationships requires chunk extractions")

    result = EntitiesAndRelationshipsOperator().apply(
        None,
        OperatorParams(
            name=step_name,
            parameters={
                "chunk_extraction_set": {
                    "kind": "chunk_extraction_set",
                    "source_subgraph_ref": str(
                        hydrated_chunk_extraction_set.get("source_subgraph_ref")
                        or inputs.get("document_set_ref")
                        or ""
                    ),
                    "subgraph_ref": chunk_extraction_set_ref,
                    "extraction_refs": [
                        str(item.get("cas_id") or "")
                        for item in chunk_extractions
                        if isinstance(item, dict) and str(item.get("cas_id") or "")
                    ],
                    "chunk_extractions": chunk_extractions,
                }
            },
        ),
        OperatorEnv(
            seed=0,
            renderer_version="debug",
            policy="strict",
            env_scope=ModeladoEnvironmentScope(ref=f"refs/heads/run/{state.artifact_id}"),
            llm=None,
        ),
    )

    state.outputs["fragment_ids"] = [item for item in result.get("fragment_ids", []) if isinstance(item, str) and item]
    state.outputs["fragment_artifact_map"] = (
        result.get("fragment_artifact_map") if isinstance(result.get("fragment_artifact_map"), dict) else {}
    )
    state.outputs["entity_relationships"] = (
        result.get("entity_relationships") if isinstance(result.get("entity_relationships"), list) else []
    )
    state.outputs["entity_relationship_set"] = (
        result.get("entity_relationship_set") if isinstance(result.get("entity_relationship_set"), dict) else {}
    )
    state.outputs["summary"] = result.get("summary") if isinstance(result.get("summary"), dict) else {}
    state.outputs["operation_telemetry"] = {
        **executor_identity,
        "operation_name": step_name,
        "operator_id": str(step_metadata.get("operator_id") or "modelado/operators/entities_and_relationships"),
        "source_kind": result.get("source_kind") or "chunk_extraction_set",
        "entity_relationship_count": len(state.outputs["fragment_ids"]),
    }

    return {
        "executor": "ikam.forja.debug_execution",
        "status": result.get("status") or "ok",
        "entity_relationship_count": len(state.outputs["fragment_ids"]),
        "details": {
            "source_kind": result.get("source_kind"),
            "fragment_ids": list(state.outputs["fragment_ids"]),
            "summary": state.outputs["summary"],
        },
    }


async def _execute_ingestion_claims(
    step_name: str,
    state: StepExecutionState,
    scope: ExecutionScope | None,
    step_metadata: dict[str, Any],
) -> dict[str, Any]:
    executor_identity = _executor_identity_from_step_metadata(
        step_metadata,
        fallback_id="executor://ml-primary",
        fallback_kind="ml-executor",
    )
    from modelado.environment_scope import EnvironmentScope as ModeladoEnvironmentScope
    from modelado.operators.claims import ClaimsOperator
    from modelado.operators.core import OperatorEnv, OperatorParams

    inputs = state.outputs.get("inputs") if isinstance(state.outputs.get("inputs"), dict) else {}
    entity_relationship_set_ref = str(inputs.get("entity_relationship_set_ref") or "")
    hydrated_entity_relationship_set = (
        state.outputs.get("entity_relationship_set") if isinstance(state.outputs.get("entity_relationship_set"), dict) else {}
    )
    entity_relationships = (
        hydrated_entity_relationship_set.get("entity_relationships")
        if isinstance(hydrated_entity_relationship_set.get("entity_relationships"), list)
        else []
    )
    if not entity_relationships:
        raise RuntimeError("claims requires entity relationship set")

    result = ClaimsOperator().apply(
        None,
        OperatorParams(
            name=step_name,
            parameters={
                "entity_relationship_set": {
                    "kind": "entity_relationship_set",
                    "source_subgraph_ref": str(
                        hydrated_entity_relationship_set.get("source_subgraph_ref") or ""
                    ),
                    "subgraph_ref": entity_relationship_set_ref,
                    "entity_relationship_refs": [
                        str(item.get("cas_id") or "")
                        for item in entity_relationships
                        if isinstance(item, dict) and str(item.get("cas_id") or "")
                    ],
                    "entity_relationships": entity_relationships,
                }
            },
        ),
        OperatorEnv(
            seed=0,
            renderer_version="debug",
            policy="strict",
            env_scope=ModeladoEnvironmentScope(ref=f"refs/heads/run/{state.artifact_id}"),
            llm=None,
        ),
    )

    state.outputs["fragment_ids"] = [item for item in result.get("fragment_ids", []) if isinstance(item, str) and item]
    state.outputs["fragment_artifact_map"] = (
        result.get("fragment_artifact_map") if isinstance(result.get("fragment_artifact_map"), dict) else {}
    )
    state.outputs["claims"] = result.get("claims") if isinstance(result.get("claims"), list) else []
    state.outputs["claim_set"] = result.get("claim_set") if isinstance(result.get("claim_set"), dict) else {}
    state.outputs["summary"] = result.get("summary") if isinstance(result.get("summary"), dict) else {}
    state.outputs["operation_telemetry"] = {
        **executor_identity,
        "operation_name": step_name,
        "operator_id": str(step_metadata.get("operator_id") or "modelado/operators/claims"),
        "source_kind": result.get("source_kind") or "entity_relationship_set",
        "claim_count": len(state.outputs["fragment_ids"]),
    }

    return {
        "executor": "ikam.forja.debug_execution",
        "status": result.get("status") or "ok",
        "claim_count": len(state.outputs["fragment_ids"]),
        "details": {
            "source_kind": result.get("source_kind"),
            "fragment_ids": list(state.outputs["fragment_ids"]),
            "summary": state.outputs["summary"],
        },
    }


def _get_ingestion_branch_handler(branch: str) -> Any:
    registry = {
        "load_documents": _execute_ingestion_load_documents,
        "chunking": _execute_ingestion_chunking,
        "entities_and_relationships": _execute_ingestion_entities_and_relationships,
        "claims": _execute_ingestion_claims,
    }
    handler = registry.get(branch)
    if handler is None:
        raise RuntimeError(f"Unknown ingestion branch: {branch}")
    return handler


async def execute_step(step_name: str, state: StepExecutionState, scope: ExecutionScope | None = None) -> dict[str, Any]:
    step_metadata = _scope_step_execution_metadata(scope, step_name)
    ingestion_branch = _resolve_ingestion_operator_branch(step_name, step_metadata)

    if step_name == "init.initialize":
        if not state.source_bytes:
            raise RuntimeError("prepare_case requires non-empty source bytes")
        return {"executor": "ikam.forja.debug_execution", "source_bytes": len(state.source_bytes)}

    if ingestion_branch is not None:
        handler = _get_ingestion_branch_handler(ingestion_branch)
        return await handler(step_name, state, scope, step_metadata)

    if step_name == "map.conceptual.lift.summarize":
        return {
            "executor": "ikam.forja.debug_execution",
            "pass_through": True,
            "details": {"step": step_name}
        }

    if step_name == "map.conceptual.embed.discovery_index":
        decomposition = state.outputs.get("decomposition")
        map_segments = state.outputs.get("map_segment_candidates")
        if decomposition is None and not isinstance(map_segments, list):
            raise RuntimeError("embed_mapped requires map output")
        import numpy as np

        deterministic_mode = _deterministic_debug_mode()
        embedder = None
        if not deterministic_mode:
            from modelado.fragment_embedder import get_shared_embedder

            embedder = get_shared_embedder()
        surface_embeddings: dict[str, list[float]] = {}

        surface_items: list[Any] = []
        if isinstance(map_segments, list) and map_segments:
            for segment in map_segments:
                if isinstance(segment, dict):
                    surface_items.append(segment)
        elif decomposition is not None:
            surface_items.extend(list(getattr(decomposition, "structural", [])))

        # Embed all mapped surface items
        for frag in surface_items:
            frag_id = getattr(frag, "cas_id", None) or getattr(frag, "id", None)
            if not isinstance(frag_id, str) and isinstance(frag, dict):
                frag_id = str(frag.get("segment_id") or "")
            if frag_id and frag_id not in surface_embeddings:
                if deterministic_mode:
                    vector = _deterministic_vector(_fragment_text(frag))
                else:
                    if isinstance(frag, dict):
                        vector = _deterministic_vector(_fragment_text(frag))
                    else:
                        vector = await embedder.embed(frag)
                surface_embeddings[frag_id] = vector

        # Build intra-decomposition similarity clusters
        CLUSTER_THRESHOLD = 0.38 if deterministic_mode else 0.7
        frag_ids = list(surface_embeddings.keys())
        clusters: list[dict] = []

        if len(frag_ids) >= 2:
            # Compute pairwise cosine similarities
            vecs = np.array([surface_embeddings[fid] for fid in frag_ids])
            norms = np.linalg.norm(vecs, axis=1, keepdims=True)
            norms = np.where(norms == 0, 1.0, norms)
            normalized = vecs / norms
            sim_matrix = normalized @ normalized.T

            # Greedy clustering: assign each fragment to first cluster it fits
            assigned: set[int] = set()
            for i in range(len(frag_ids)):
                if i in assigned:
                    continue
                members = [frag_ids[i]]
                member_indices = [i]
                for j in range(i + 1, len(frag_ids)):
                    if j in assigned:
                        continue
                    if sim_matrix[i, j] >= CLUSTER_THRESHOLD:
                        members.append(frag_ids[j])
                        member_indices.append(j)
                        assigned.add(j)
                assigned.add(i)

                # Compute average intra-cluster similarity
                if len(member_indices) > 1:
                    sims = []
                    for a in range(len(member_indices)):
                        for b in range(a + 1, len(member_indices)):
                            sims.append(float(sim_matrix[member_indices[a], member_indices[b]]))
                    avg_sim = sum(sims) / len(sims) if sims else 1.0
                else:
                    avg_sim = 1.0

                clusters.append({
                    "members": members,
                    "centroid_id": frag_ids[i],
                    "avg_similarity": avg_sim,
                })
        else:
            # Single fragment or none: one trivial cluster per fragment
            for fid in frag_ids:
                clusters.append({
                    "members": [fid],
                    "centroid_id": fid,
                    "avg_similarity": 1.0,
                })

        state.outputs["surface_embeddings"] = surface_embeddings
        state.outputs["surface_clusters"] = clusters

        return {
            "executor": "ikam.forja.debug_execution",
            "embedding_count": len(surface_embeddings),
            "cluster_count": len(clusters),
            "details": {
                "embedded_ids": list(surface_embeddings.keys()),
                "clusters": [
                    {"members": c["members"], "centroid_id": c["centroid_id"], "avg_similarity": c["avg_similarity"]}
                    for c in clusters
                ],
                "embedding_mode": "deterministic" if deterministic_mode else "model",
                "cluster_threshold": CLUSTER_THRESHOLD,
            },
        }

    if step_name == "map.conceptual.normalize.discovery":
        decomposition = state.outputs.get("decomposition")
        map_segments = state.outputs.get("map_segment_candidates")
        if decomposition is None and not isinstance(map_segments, list):
            raise RuntimeError("lift requires map segment candidates")

        deterministic_mode = _deterministic_debug_mode()
        ir_fragments = []
        lifted_from_map: dict[str, str] = {}

        # Read surface clusters from embed_mapped (if available)
        surface_clusters = state.outputs.get("surface_clusters")
        clusters_available = isinstance(surface_clusters, list) and len(surface_clusters) > 0

        # Build lookup: cas_id → cluster for O(1) access per fragment
        frag_to_cluster: dict[str, dict] = {}
        if clusters_available:
            for cluster in surface_clusters:
                for member_id in cluster.get("members", []):
                    frag_to_cluster[member_id] = cluster

        from ikam.forja.cas import cas_fragment as _cas_fragment
        from ikam.ir.mime_types import CLAIM_IR

        lifter = None
        if not deterministic_mode:
            from modelado.lifter import ClaimLifter
            from modelado.oraculo.unified_bridge import UnifiedCallModelClient

            lifter = ClaimLifter(ai_client=UnifiedCallModelClient.from_env())

        surface_items: list[Any] = []
        if isinstance(map_segments, list) and map_segments:
            for segment in map_segments:
                if isinstance(segment, dict):
                    surface_items.append(segment)
        elif decomposition is not None:
            surface_items.extend(list(getattr(decomposition, "structural", [])))

        surface_lookup = {
            str(getattr(item, "cas_id", None) or getattr(item, "id", None) or (item.get("segment_id") if isinstance(item, dict) else "")): item
            for item in surface_items
        }

        for surface_frag in surface_items:
            source_id = getattr(surface_frag, "cas_id", None)
            if not isinstance(source_id, str) and isinstance(surface_frag, dict):
                source_id = str(surface_frag.get("segment_id") or "")
            if not source_id:
                continue

            # Build cluster context for this fragment
            cluster_ctx = None
            if clusters_available and source_id in frag_to_cluster:
                cluster = frag_to_cluster[source_id]
                # Collect sibling texts (excluding self) for LLM context
                sibling_texts = []
                for sib_id in cluster.get("members", []):
                    if sib_id == source_id:
                        continue
                    sibling = surface_lookup.get(str(sib_id))
                    text = _fragment_text(sibling) if sibling is not None else ""
                    if isinstance(text, str) and text.strip():
                        sibling_texts.append(text.strip())
                cluster_ctx = {
                    "cluster_members": cluster.get("members", []),
                    "centroid_id": cluster.get("centroid_id", ""),
                    "member_texts": sibling_texts,
                }

            if deterministic_mode:
                text = _fragment_text(surface_frag)
                preview = text.replace("\n", " ").strip()[:180]
                claims = [
                    _cas_fragment(
                        {
                            "subject": source_id,
                            "predicate": "states",
                            "object": preview or "empty-fragment",
                        },
                        CLAIM_IR,
                    )
                ]
            else:
                if isinstance(surface_frag, dict):
                    text = _fragment_text(surface_frag)
                    preview = text.replace("\n", " ").strip()[:180]
                    claims = [
                        _cas_fragment(
                            {
                                "subject": source_id,
                                "predicate": "states",
                                "object": preview or "empty-fragment",
                            },
                            CLAIM_IR,
                        )
                    ]
                else:
                    claims = await lifter.lift(surface_frag, cluster_context=cluster_ctx)
            for claim_frag in claims:
                ir_fragments.append(claim_frag)
                if claim_frag.cas_id:
                    lifted_from_map[claim_frag.cas_id] = source_id

        state.outputs["ir_fragments"] = ir_fragments
        state.outputs["lifted_from_map"] = lifted_from_map
        # Keep legacy key for backward compat during migration
        state.outputs["lifted"] = [
            {"id": f.cas_id, "mime_type": f.mime_type} for f in ir_fragments
        ]

        # Build one ReconstructionProgram per surface→IR mapping (transform strategy)
        from ikam.ir.reconstruction import (
            ReconstructionProgram,
            CompositionStep,
            program_to_fragment,
        )

        # Invert lifted_from_map: surface_cas_id → [ir_cas_ids]
        surface_to_ir: dict[str, list[str]] = {}
        for ir_id, surface_id in lifted_from_map.items():
            surface_to_ir.setdefault(surface_id, []).append(ir_id)

        lift_programs = []
        for surface_id, ir_ids in surface_to_ir.items():
            program = ReconstructionProgram(
                steps=[
                    CompositionStep(
                        strategy="transform",
                        inputs={
                            "source_cas_id": surface_id,
                            "ir_cas_ids": ir_ids,
                        },
                    )
                ],
                output_mime_type=state.mime_type or "application/octet-stream",
            )
            lift_programs.append(program_to_fragment(program))

        state.outputs["lift_reconstruction_programs"] = lift_programs

        # Emit ir_fragment→surface_fragment "lifted-from" edges
        new_edges = []
        new_nodes = []
        for ir_id, surface_id in lifted_from_map.items():
            if ir_id and surface_id:
                new_edges.append({
                    "source": ir_id,
                    "target": surface_id,
                    "predicate": "lifted-from",
                    "step": "map.conceptual.normalize.discovery",
                })
                new_nodes.append({"id": ir_id, "type": "ir_fragment"})
        _append_edges_and_project(state, new_edges, new_nodes)

        return {
            "executor": "ikam.forja.debug_execution",
            "lifted_count": len(ir_fragments),
            "details": {
                "lifted_ids": [str(f.cas_id) for f in ir_fragments if f.cas_id],
                "lift_program_count": len(lift_programs),
                "cluster_context_used": clusters_available,
                "cluster_count": len(surface_clusters) if clusters_available else 0,
                "lift_mode": "deterministic" if deterministic_mode else "model",
            },
        }

    if step_name == "map.reconstructable.embed":
        ir_fragments = state.outputs.get("ir_fragments")
        decomposition = state.outputs.get("decomposition")
        map_segments = state.outputs.get("map_segment_candidates")
        if not isinstance(ir_fragments, list):
            raise RuntimeError("embed_lifted requires ir_fragments output")
        if decomposition is None and not isinstance(map_segments, list):
            raise RuntimeError("embed_lifted requires map segment candidates")

        deterministic_mode = _deterministic_debug_mode()
        embedder = None
        if not deterministic_mode:
            from modelado.fragment_embedder import get_shared_embedder

            embedder = get_shared_embedder()
        embeddings: dict[str, list[float]] = {}

        # Reuse surface embeddings from embed_mapped if available
        surface_embeddings = state.outputs.get("surface_embeddings")
        if isinstance(surface_embeddings, dict):
            embeddings.update(surface_embeddings)

        # Embed all IR fragments
        for frag in ir_fragments:
            frag_id = getattr(frag, "cas_id", None)
            if frag_id and frag_id not in embeddings:
                if deterministic_mode:
                    vector = _deterministic_vector(_fragment_text(frag))
                else:
                    vector = await embedder.embed(frag)
                embeddings[frag_id] = vector

        # Embed any surface fragments not already embedded
        surface_items: list[Any] = []
        if isinstance(map_segments, list) and map_segments:
            for segment in map_segments:
                if isinstance(segment, dict):
                    surface_items.append(segment)
        elif decomposition is not None:
            surface_items.extend(list(getattr(decomposition, "structural", [])))

        for frag in surface_items:
            frag_id = getattr(frag, "cas_id", None)
            if not isinstance(frag_id, str) and isinstance(frag, dict):
                frag_id = str(frag.get("segment_id") or "")
            if frag_id and frag_id not in embeddings:
                if deterministic_mode:
                    vector = _deterministic_vector(_fragment_text(frag))
                else:
                    if isinstance(frag, dict):
                        vector = _deterministic_vector(_fragment_text(frag))
                    else:
                        vector = await embedder.embed(frag)
                embeddings[frag_id] = vector

        state.outputs["embeddings"] = embeddings
        all_keys = list(embeddings.keys())
        return {
            "executor": "ikam.forja.debug_execution",
            "embedding_count": len(embeddings),
            "details": {
                "embedding_keys": all_keys,
                "embedding_mode": "deterministic" if deterministic_mode else "model",
            },
        }

    if step_name == "map.reconstructable.search.dependency_resolution":
        embeddings = state.outputs.get("embeddings")
        if not isinstance(embeddings, dict):
            raise RuntimeError("candidate_search requires embeddings (dict[str, list[float]])")

        from modelado.db import connection_scope
        from modelado.environment_scope import EnvironmentScope

        # Derive operation_id from artifact_id for scoping DB rows
        op_id = state.artifact_id.split(":")[-1] if ":" in state.artifact_id else state.artifact_id
        project_id = state.artifact_id.split(":")[0] if ":" in state.artifact_id else "default"
        ref = EnvironmentScope(ref="refs/heads/run/default").ref
        deterministic_mode = _deterministic_debug_mode()
        embedding_model = "ikam-deterministic-debug-embedder/v1"
        if not deterministic_mode:
            from modelado.fragment_embedder import get_shared_embedder

            embedding_model = get_shared_embedder().model_name

        SIMILARITY_THRESHOLD = 0.85

        with connection_scope() as cx:
            # 1) Store each fragment embedding in ikam_fragment_store
            for cas_id, vec in embeddings.items():
                vec_literal = "[" + ",".join(str(v) for v in vec) + "]"
                cx.execute(
                    """
                    INSERT INTO ikam_fragment_store
                        (cas_id, ref, operation_id, project_id, embedding, embedding_model)
                    VALUES (%s, %s, %s, %s, %s::vector, %s)
                    ON CONFLICT (cas_id, ref, COALESCE(operation_id, '')) DO UPDATE
                        SET embedding = EXCLUDED.embedding,
                            embedding_model = EXCLUDED.embedding_model
                    """,
                    (cas_id, ref, op_id, project_id, vec_literal, embedding_model),
                )
            cx.commit()

            # 2) For each fragment, query HNSW for nearest neighbors
            ids = list(embeddings.keys())
            seen_pairs: set[tuple[str, str]] = set()
            candidates: list[dict[str, Any]] = []

            for source_id in ids:
                vec_literal = "[" + ",".join(str(v) for v in embeddings[source_id]) + "]"
                with cx.cursor() as cur:
                    cur.execute(
                        """
                        SELECT cas_id, 1 - (embedding <=> %s::vector) AS similarity
                        FROM ikam_fragment_store
                        WHERE operation_id = %s
                          AND cas_id != %s
                          AND embedding IS NOT NULL
                        ORDER BY embedding <=> %s::vector
                        LIMIT %s
                        """,
                        (vec_literal, op_id, source_id, vec_literal, len(ids)),
                    )
                    for row in cur.fetchall():
                        target_id = row["cas_id"]
                        similarity = float(row["similarity"])
                        if similarity < SIMILARITY_THRESHOLD:
                            continue
                        pair = tuple(sorted([source_id, target_id]))
                        if pair in seen_pairs:
                            continue
                        seen_pairs.add(pair)
                        candidates.append({
                            "source_id": source_id,
                            "target_id": target_id,
                            "similarity": similarity,
                            "tier": "embedding",
                        })

        state.outputs["candidates"] = candidates
        return {
            "executor": "ikam.forja.debug_execution",
            "candidate_count": len(candidates),
            "details": {
                "embedding_model": embedding_model,
                "candidate_pairs": [
                    {"source_id": c["source_id"], "target_id": c["target_id"], "similarity": c["similarity"]}
                    for c in candidates
                ],
            },
        }

    if step_name == "map.reconstructable.normalize":
        ir_fragments = state.outputs.get("ir_fragments")
        if not isinstance(ir_fragments, list):
            raise RuntimeError("normalize requires ir_fragments output")
        decomposition = state.outputs.get("decomposition")
        map_segments = state.outputs.get("map_segment_candidates")
        if decomposition is None and not isinstance(map_segments, list):
            raise RuntimeError("normalize requires map segment candidates")

        from ikam.forja.cas import cas_fragment as _cas_fragment
        from ikam.ir.reconstruction import (
            ReconstructionProgram,
            CompositionStep,
            program_to_fragment,
        )
        from ikam.fragments import Fragment as _Fragment, CONCEPT_MIME

        deterministic_mode = _deterministic_debug_mode()
        normalizer = None
        if not deterministic_mode:
            from ikam.forja.normalizer import SemanticNormalizer
            from modelado.oraculo.unified_bridge import UnifiedCallModelClient

            normalizer = SemanticNormalizer(ai_client=UnifiedCallModelClient.from_env())
        candidates = state.outputs.get("candidates", [])
        duplicate_targets = {c["target_id"] for c in candidates if "target_id" in c}

        # --- Semantic normalization of IR fragments ---
        all_normalized: list = []
        normalized_from_map: dict[str, str] = {}  # normalized CAS ID → IR CAS ID
        for ir_frag in ir_fragments:
            frag_id = getattr(ir_frag, "cas_id", None)
            if frag_id and frag_id in duplicate_targets:
                continue

            text_content = ""
            val = getattr(ir_frag, "value", None)
            if isinstance(val, dict):
                parts = []
                for key in ("subject", "predicate", "object"):
                    if key in val:
                        parts.append(str(val[key]))
                text_content = " ".join(parts) if parts else ""
            elif isinstance(val, str):
                text_content = val

            if not text_content:
                continue

            text_frag = _Fragment(
                cas_id=frag_id,
                value={"text": text_content},
                mime_type=getattr(ir_frag, "mime_type", None),
            )
            if deterministic_mode:
                concept_label = text_content.lower().strip()[:120]
                concepts = [
                    _Fragment(
                        value={"concept": concept_label, "source": frag_id},
                        mime_type=CONCEPT_MIME,
                    )
                ]
            else:
                concepts = await normalizer.normalize(text_frag)
            for concept_frag in concepts:
                cas_concepts = _cas_fragment(
                    concept_frag.value, concept_frag.mime_type or CONCEPT_MIME,
                )
                all_normalized.append(cas_concepts)
                if cas_concepts.cas_id and frag_id:
                    normalized_from_map[cas_concepts.cas_id] = frag_id

        # --- Build ONE concatenate reconstruction program from mapped segments ---
        structural_ids: list[str] = []
        if isinstance(map_segments, list) and map_segments:
            for segment in map_segments:
                if isinstance(segment, dict):
                    segment_id = segment.get("segment_id")
                    if isinstance(segment_id, str) and segment_id:
                        structural_ids.append(segment_id)
        elif decomposition is not None:
            structural_ids.extend(
                [f.cas_id for f in getattr(decomposition, "structural", []) if getattr(f, "cas_id", None)]
            )
        structural_ids = list(dict.fromkeys(structural_ids))
        program = ReconstructionProgram(
            steps=[
                CompositionStep(
                    strategy="concatenate",
                    inputs={"fragment_ids": structural_ids},
                )
            ],
            output_mime_type=state.mime_type or "application/octet-stream",
        )
        all_programs = [program_to_fragment(program)]

        # Build per-fragment normalize reconstruction programs (transform strategy)
        # Invert normalized_from_map: ir_cas_id → [normalized_cas_ids]
        ir_to_normalized: dict[str, list[str]] = {}
        for norm_id, ir_id in normalized_from_map.items():
            ir_to_normalized.setdefault(ir_id, []).append(norm_id)

        normalize_programs = []
        for ir_id, norm_ids in ir_to_normalized.items():
            norm_program = ReconstructionProgram(
                steps=[
                    CompositionStep(
                        strategy="transform",
                        inputs={
                            "ir_cas_id": ir_id,
                            "normalized_cas_ids": norm_ids,
                        },
                    )
                ],
                output_mime_type=state.mime_type or "application/octet-stream",
            )
            normalize_programs.append(program_to_fragment(norm_program))

        state.outputs["normalized_fragments"] = all_normalized
        state.outputs["reconstruction_programs"] = all_programs
        state.outputs["normalize_reconstruction_programs"] = normalize_programs
        state.outputs["normalized"] = all_normalized
        state.outputs["normalized_from_map"] = normalized_from_map

        # Emit normalized_fragment→ir_fragment "normalized-by" edges
        new_edges = []
        new_nodes = []
        for norm_id, ir_id in normalized_from_map.items():
            if norm_id and ir_id:
                new_edges.append({
                    "source": norm_id,
                    "target": ir_id,
                    "predicate": "normalized-by",
                    "step": "map.reconstructable.normalize",
                })
                new_nodes.append({"id": norm_id, "type": "normalized_fragment"})

        # Emit composed-by edges: reconstruction_program → normalized_fragment
        for ir_id, norm_ids in ir_to_normalized.items():
            # Find the normalize program for this IR fragment
            matching_programs = [
                p for p in normalize_programs
                if getattr(p, "value", None) is not None
                and isinstance(p.value, dict)
                and any(
                    s.get("inputs", {}).get("ir_cas_id") == ir_id
                    for s in p.value.get("steps", [])
                )
            ]
            for prog in matching_programs:
                for n_id in norm_ids:
                    new_edges.append({
                        "source": prog.cas_id,
                        "target": n_id,
                        "predicate": "composed-by",
                        "step": "map.reconstructable.normalize",
                    })
                    if prog.cas_id:
                        new_nodes.append({"id": prog.cas_id, "type": "fragment"})

        _append_edges_and_project(state, new_edges, new_nodes)

        return {
            "executor": "ikam.forja.debug_execution",
            "normalized_count": len(all_normalized),
            "program_count": len(all_programs),
            "details": {
                "normalized_ids": [str(f.cas_id) for f in all_normalized if f.cas_id],
                "program_ids": [str(f.cas_id) for f in all_programs if f.cas_id],
                "normalize_program_count": len(normalize_programs),
                "normalize_mode": "deterministic" if deterministic_mode else "model",
            },
        }

    if step_name == "map.reconstructable.compose.reconstruction_programs":
        normalized_fragments = state.outputs.get("normalized_fragments")
        reconstruction_programs = state.outputs.get("reconstruction_programs")
        lift_programs = state.outputs.get("lift_reconstruction_programs", [])
        normalize_programs = state.outputs.get("normalize_reconstruction_programs", [])
        ir_fragments = state.outputs.get("ir_fragments")
        decomposition = state.outputs.get("decomposition")
        map_segments = state.outputs.get("map_segment_candidates")
        if not isinstance(ir_fragments, list):
            raise RuntimeError("compose_proposal requires ir_fragments")
        if decomposition is None and not isinstance(map_segments, list):
            raise RuntimeError("compose_proposal requires map segment candidates")

        # Collect CAS IDs from each fragment group
        ir_ids = [f.cas_id for f in ir_fragments if getattr(f, "cas_id", None)]
        normalized_ids = [f.cas_id for f in (normalized_fragments or []) if getattr(f, "cas_id", None)]
        program_ids = [f.cas_id for f in (reconstruction_programs or []) if getattr(f, "cas_id", None)]
        lift_program_ids = [f.cas_id for f in lift_programs if getattr(f, "cas_id", None)]
        normalize_program_ids = [f.cas_id for f in normalize_programs if getattr(f, "cas_id", None)]
        surface_ids: list[str] = []
        if isinstance(map_segments, list) and map_segments:
            for segment in map_segments:
                if isinstance(segment, dict):
                    segment_id = segment.get("segment_id")
                    if isinstance(segment_id, str) and segment_id:
                        surface_ids.append(segment_id)
        elif decomposition is not None:
            surface_ids.extend(
                [f.cas_id for f in decomposition.structural if getattr(f, "cas_id", None)]
            )
        surface_ids = list(dict.fromkeys(surface_ids))

        # Path A (normalized) when reconstruction programs exist, Path B (surface_only) otherwise
        commit_mode = "normalized" if program_ids else "surface_only"
        total_fragments = (
            len(ir_ids) + len(normalized_ids) + len(program_ids)
            + len(lift_program_ids) + len(normalize_program_ids) + len(surface_ids)
        )

        proposal = {
            "commit_mode": commit_mode,
            "ir_fragment_ids": ir_ids,
            "normalized_fragment_ids": normalized_ids,
            "program_ids": program_ids,
            "lift_program_ids": lift_program_ids,
            "normalize_program_ids": normalize_program_ids,
            "surface_fragment_ids": surface_ids,
        }
        state.outputs["proposal"] = proposal
        return {
            "executor": "ikam.forja.debug_execution",
            "proposal_ready": True,
            "details": {
                "commit_mode": commit_mode,
                "fragment_count": total_fragments,
                "ir_count": len(ir_ids),
                "normalized_count": len(normalized_ids),
                "program_count": len(program_ids),
                "lift_program_count": len(lift_program_ids),
                "normalize_program_count": len(normalize_program_ids),
                "surface_count": len(surface_ids),
            },
        }

    if step_name == "map.conceptual.verify.discovery_gate":
        # Check for injected failure flag (one-shot: consume on read)
        injection = state.outputs.pop("_injected_verify_fail", None)
        if injection is not None:
            raise InjectedVerifyFailure(drift_at=injection.get("drift_at", "map"))

        decomposition = state.outputs.get("decomposition")
        if decomposition is None:
            raise RuntimeError("verify requires decomposition output")
        programs = state.outputs.get("reconstruction_programs")
        if not isinstance(programs, list) or len(programs) == 0:
            raise RuntimeError("verify requires reconstruction_programs output")

        from ikam.forja.verifier import ByteIdentityVerifier, DriftSpec
        from ikam.forja.cas import cas_fragment as _verify_cas
        from ikam.ir.reconstruction import ReconstructionProgram, render_program
        import base64 as _b64

        # Build fragment store from decomposition structural fragments
        fragment_store = {}
        for frag in getattr(decomposition, "structural", []):
            if getattr(frag, "cas_id", None):
                fragment_store[frag.cas_id] = frag

        # Execute each reconstruction program and concatenate results.
        # When mapped segments are semantic-only (no raw bytes), fall back to
        # source bytes so verification can still emit provenance edges.
        rendered_parts = []
        try:
            for prog_frag in programs:
                prog_data = getattr(prog_frag, "value", {})
                program = ReconstructionProgram.model_validate(prog_data)
                rendered_parts.append(render_program(program, fragment_store))
            reconstructed_bytes = b"".join(rendered_parts)
            if not reconstructed_bytes and state.source_bytes:
                reconstructed_bytes = state.source_bytes
        except Exception:
            reconstructed_bytes = state.source_bytes

        # Build an "original" Fragment with bytes_b64 for the verifier protocol
        original_frag = _verify_cas(
            {"bytes_b64": _b64.b64encode(state.source_bytes).decode()},
            state.mime_type,
        )

        drift_spec = DriftSpec(metric="byte-identity", tolerance=0.0)
        verifier = ByteIdentityVerifier()
        vr_frag = verifier.verify(original_frag, reconstructed_bytes, drift_spec)

        passed = vr_frag.value["passed"]
        measured_drift = vr_frag.value["measured_drift"]

        # Record full reconstruction chain for audit/debug traceability
        lift_progs = state.outputs.get("lift_reconstruction_programs", [])
        norm_progs = state.outputs.get("normalize_reconstruction_programs", [])
        reconstruction_chain = {
            "lift_programs": [f.cas_id for f in lift_progs if getattr(f, "cas_id", None)],
            "normalize_programs": [f.cas_id for f in norm_progs if getattr(f, "cas_id", None)],
            "concatenate_programs": [f.cas_id for f in programs if getattr(f, "cas_id", None)],
        }
        has_chain = bool(reconstruction_chain["lift_programs"] or reconstruction_chain["normalize_programs"])

        verification_output = {"passed": passed, "measured_drift": measured_drift}
        if has_chain:
            verification_output["reconstruction_chain"] = reconstruction_chain

        state.outputs["verification"] = verification_output
        state.outputs["verification_result_fragment"] = vr_frag

        # Emit verification_result→artifact "verified-by" edge
        vr_id = getattr(vr_frag, "cas_id", None)
        if vr_id:
            new_edges = [{
                "source": vr_id,
                "target": state.artifact_id,
                "predicate": "verified-by",
                "step": "map.conceptual.verify.discovery_gate",
            }]
            new_nodes = [{"id": vr_id, "type": "verification_result"}]
            _append_edges_and_project(state, new_edges, new_nodes)

        return {
            "executor": "ikam.forja.debug_execution",
            "passed": passed,
            "details": {
                "metric": "byte-identity",
                "measured_drift": measured_drift,
                "diff_summary": vr_frag.value.get("diff_summary"),
                "reconstruction_chain_recorded": has_chain,
            },
        }

    if step_name == "map.conceptual.commit.semantic_only":
        verification = state.outputs.get("verification")
        proposal = state.outputs.get("proposal")
        if not isinstance(verification, dict) or not isinstance(proposal, dict):
            raise RuntimeError("promote_commit requires verification and proposal outputs")

        passed = verification.get("passed", False)

        if passed:
            # Path A: normalized — commit IR fragments, concepts, programs, verification result
            committed_ids = []
            committed_ids.extend(proposal.get("ir_fragment_ids", []))
            committed_ids.extend(proposal.get("normalized_fragment_ids", []))
            committed_ids.extend(proposal.get("program_ids", []))
            vr_frag = state.outputs.get("verification_result_fragment")
            if vr_frag is not None and getattr(vr_frag, "cas_id", None):
                committed_ids.append(vr_frag.cas_id)
            commit_mode = "normalized"
        else:
            # Path B: surface_only — commit surface fragments directly
            committed_ids = list(proposal.get("surface_fragment_ids", []))
            commit_mode = "surface_only"

        state.outputs["commit"] = {
            "mode": commit_mode,
            "target_ref": "refs/heads/main",
            "promoted_fragment_ids": committed_ids,
            "committed_fragment_ids": committed_ids,
        }
        return {
            "executor": "ikam.forja.debug_execution",
            "commit_mode": commit_mode,
            "details": {
                "verification_passed": passed,
                "committed_count": len(committed_ids),
            },
        }

    if step_name == "map.reconstructable.build_subgraph.reconstruction":
        decomposition = state.outputs.get("decomposition")
        map_segments = state.outputs.get("map_segment_candidates")
        if decomposition is None and not isinstance(map_segments, list):
            raise RuntimeError("project_graph requires map segment candidates")

        accumulated_edges = state.outputs.get("edges")

        if accumulated_edges is not None:
            # --- Assemble from accumulated edges (normal pipeline path) ---
            # Use the current graph_projection built incrementally by prior steps
            prev_projection = state.outputs.get("graph_projection", {"nodes": [], "edges": []})
            nodes = list(prev_projection["nodes"])
            node_id_set = {n["id"] for n in nodes}

            # Ensure artifact node exists
            art_id = state.artifact_id
            if art_id not in node_id_set:
                nodes.append({"id": art_id, "type": "artifact"})
                node_id_set.add(art_id)

            # Add any committed fragment nodes not yet in the projection
            commit = state.outputs.get("commit", {})
            for fid in commit.get("committed_fragment_ids", []):
                if fid and fid not in node_id_set:
                    nodes.append({"id": fid, "type": "fragment"})
                    node_id_set.add(fid)

            edges = list(accumulated_edges)
        else:
            # --- Backward compat: build from scratch (tests calling project_graph directly) ---
            commit = state.outputs.get("commit", {})
            lifted_from_map = state.outputs.get("lifted_from_map", {})
            normalized_from_map = state.outputs.get("normalized_from_map", {})
            ir_fragments = state.outputs.get("ir_fragments", [])
            normalized_fragments = state.outputs.get("normalized_fragments", [])

            committed_ids = set(commit.get("committed_fragment_ids", []))

            nodes: list[dict[str, Any]] = []
            node_id_set: set[str] = set()

            art_id = state.artifact_id
            nodes.append({"id": art_id, "type": "artifact"})
            node_id_set.add(art_id)

            if decomposition is not None:
                for frag in decomposition.structural:
                    fid = getattr(frag, "cas_id", None)
                    if fid and fid not in node_id_set:
                        nodes.append({"id": fid, "type": "surface_fragment"})
                        node_id_set.add(fid)
            if isinstance(map_segments, list):
                for segment in map_segments:
                    if not isinstance(segment, dict):
                        continue
                    fid = segment.get("segment_id")
                    if isinstance(fid, str) and fid and fid not in node_id_set:
                        nodes.append({"id": fid, "type": "surface_fragment"})
                        node_id_set.add(fid)

            for frag in ir_fragments:
                fid = getattr(frag, "cas_id", None)
                if fid and fid not in node_id_set:
                    nodes.append({"id": fid, "type": "ir_fragment"})
                    node_id_set.add(fid)

            for frag in normalized_fragments:
                fid = getattr(frag, "cas_id", None)
                if fid and fid not in node_id_set:
                    nodes.append({"id": fid, "type": "normalized_fragment"})
                    node_id_set.add(fid)

            for fid in committed_ids:
                if fid and fid not in node_id_set:
                    nodes.append({"id": fid, "type": "fragment"})
                    node_id_set.add(fid)

            edges: list[dict[str, str]] = []

            for fid in committed_ids:
                if fid:
                    edges.append({"source": art_id, "target": fid, "predicate": "contains"})

            for ir_id, surface_id in lifted_from_map.items():
                if ir_id and surface_id:
                    edges.append({"source": ir_id, "target": surface_id, "predicate": "lifted-from"})

            for norm_id, ir_id in normalized_from_map.items():
                if norm_id and ir_id:
                    edges.append({"source": norm_id, "target": ir_id, "predicate": "normalized-by"})

            vr_frag = state.outputs.get("verification_result_fragment")
            if vr_frag is not None:
                vr_id = getattr(vr_frag, "cas_id", None)
                if vr_id:
                    if vr_id not in node_id_set:
                        nodes.append({"id": vr_id, "type": "verification_result"})
                        node_id_set.add(vr_id)
                    edges.append({"source": vr_id, "target": art_id, "predicate": "verified-by"})

        graph_projection = {
            "nodes": nodes,
            "edges": edges,
            "node_count": len(nodes),
            "edge_count": len(edges),
        }
        state.outputs["graph_projection"] = graph_projection

        # Store predicate vocabulary as a CAS-addressed Fragment
        from ikam.forja.predicate_vocabulary import build_default_vocabulary
        state.outputs["predicate_vocabulary"] = build_default_vocabulary()

        return {
            "executor": "ikam.forja.debug_execution",
            "details": {
                "node_count": len(nodes),
                "edge_count": len(edges),
            },
        }

    raise RuntimeError(f"No executor configured for step: {step_name}")
