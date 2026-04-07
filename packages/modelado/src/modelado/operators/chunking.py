from __future__ import annotations

import json
from typing import Any

from blake3 import blake3

from modelado.oraculo.ai_client import GenerateRequest

from .core import Operator, OperatorEnv, OperatorParams, ProvenanceRecord, _run_async_safely, record_provenance


CHUNK_MIME = "application/vnd.ikam.chunk-extraction+json"
DOCUMENT_CHUNK_SET_MIME = "application/vnd.ikam.document-chunk-set+json"


def _chunk_fragment_ref(value: dict[str, Any]) -> str:
    stable_json = json.dumps(
        {"mime_type": CHUNK_MIME, "value": value},
        sort_keys=True,
        ensure_ascii=False,
    )
    return blake3(stable_json.encode("utf-8")).hexdigest()


def _document_chunk_set_fragment_ref(value: dict[str, Any]) -> str:
    stable_json = json.dumps(
        {"mime_type": DOCUMENT_CHUNK_SET_MIME, "value": value},
        sort_keys=True,
        ensure_ascii=False,
    )
    return blake3(stable_json.encode("utf-8")).hexdigest()


def _split_text_into_chunks(text: str) -> list[tuple[str, int, int]]:
    if not text.strip():
        return []

    chunks: list[tuple[str, int, int]] = []
    cursor = 0
    for block in text.split("\n\n"):
        chunk_text = block.strip()
        if not chunk_text:
            cursor += len(block) + 2
            continue
        start = text.find(chunk_text, cursor)
        if start < 0:
            start = cursor
        end = start + len(chunk_text)
        chunks.append((chunk_text, start, end))
        cursor = end
    return chunks or [(text, 0, len(text))]


def _locate_chunk_span(text: str, chunk_text: str, cursor: int) -> tuple[int, int]:
    start = text.find(chunk_text, cursor)
    if start < 0:
        start = text.find(chunk_text)
    if start < 0:
        start = cursor
    end = start + len(chunk_text)
    return start, end


def _llm_chunk_spans(text: str, env: OperatorEnv) -> list[tuple[str, int, int]] | None:
    if not env.llm or not text.strip():
        return None
    try:
        response = _run_async_safely(
            env.llm.generate(
                GenerateRequest(
                    messages=[
                        {
                            "role": "system",
                            "content": (
                                "Partition the document into grounded chunks. "
                                "Return JSON with key 'chunks' containing an ordered list of exact substrings from the source text. "
                                "Do not rewrite or summarize."
                            ),
                        },
                        {"role": "user", "content": text},
                    ],
                    model="gpt-4o-mini",
                    temperature=0.0,
                    response_format={"type": "json_object"},
                )
            )
        )
        payload = json.loads(response.text)
        raw_chunks = payload.get("chunks") if isinstance(payload, dict) else None
        if not isinstance(raw_chunks, list):
            return None
        spans: list[tuple[str, int, int]] = []
        cursor = 0
        for raw_chunk in raw_chunks:
            chunk_text = str(raw_chunk or "").strip()
            if not chunk_text:
                continue
            start, end = _locate_chunk_span(text, chunk_text, cursor)
            spans.append((chunk_text, start, end))
            cursor = end
        return spans or None
    except Exception:
        return None


class ChunkOperator(Operator):
    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> dict[str, Any]:
        del fragment
        raw = params.parameters
        documents = raw.get("documents") if isinstance(raw.get("documents"), list) else []
        source_subgraph_ref = str(raw.get("source_subgraph_ref") or "")
        subgraph_ref = str(raw.get("subgraph_ref") or "")

        normalized_documents = [item for item in documents if isinstance(item, dict)]
        chunk_rows: list[dict[str, Any]] = []
        chunk_fragment_refs: list[str] = []
        chunk_fragment_artifact_map: dict[str, str] = {}
        document_stats: list[dict[str, Any]] = []
        extractions: list[dict[str, Any]] = []
        document_chunk_sets: list[dict[str, Any]] = []
        hydrated_document_chunk_sets: list[dict[str, Any]] = []
        derivation_edges: list[dict[str, str]] = []

        for document in normalized_documents:
            document_id = str(document.get("id") or f"doc-{len(document_stats)}")
            artifact_id = str(document.get("artifact_id") or raw.get("artifact_id") or "")
            source_document_fragment_id = str(document.get("source_document_fragment_id") or "")
            filename = str(document.get("filename") or artifact_id.rsplit("/", 1)[-1] or document_id)
            mime_type = str(document.get("mime_type") or "text/plain")
            text = str(document.get("text") or "")

            spans = _llm_chunk_spans(text, env) or _split_text_into_chunks(text)
            document_chunk_refs: list[str] = []
            for order, (chunk_text, start, end) in enumerate(spans):
                chunk_value = {
                    "chunk_id": f"{document_id}:chunk:{order}",
                    "document_id": document_id,
                    "source_document_fragment_id": source_document_fragment_id,
                    "artifact_id": artifact_id,
                    "filename": filename,
                    "mime_type": mime_type,
                    "text": chunk_text,
                    "span": {"start": start, "end": end},
                    "order": order,
                }
                fragment_ref = _chunk_fragment_ref(chunk_value)
                chunk_value["fragment_id"] = fragment_ref
                chunk_rows.append(chunk_value)
                chunk_fragment_refs.append(fragment_ref)
                document_chunk_refs.append(fragment_ref)
                extractions.append(
                    {
                        "cas_id": fragment_ref,
                        "mime_type": CHUNK_MIME,
                        "value": dict(chunk_value),
                    }
                )
                if artifact_id:
                    chunk_fragment_artifact_map[fragment_ref] = artifact_id
                if source_document_fragment_id:
                    derivation_edges.append(
                        {
                            "from": f"fragment:{fragment_ref}",
                            "to": f"fragment:{source_document_fragment_id}",
                            "edge_label": "knowledge:derives",
                        }
                    )

            document_chunk_set_value = {
                "kind": "document_chunk_set",
                "document_id": document_id,
                "source_document_fragment_id": source_document_fragment_id,
                "artifact_id": artifact_id,
                "filename": filename,
                "chunk_refs": document_chunk_refs,
            }
            document_chunk_set_fragment_id = _document_chunk_set_fragment_ref(document_chunk_set_value)
            document_chunk_set_value["fragment_id"] = document_chunk_set_fragment_id
            document_chunk_sets.append(document_chunk_set_value)
            hydrated_document_chunk_sets.append(
                {
                    "cas_id": document_chunk_set_fragment_id,
                    "mime_type": DOCUMENT_CHUNK_SET_MIME,
                    "value": dict(document_chunk_set_value),
                }
            )
            document_stats.append(
                {
                    "document_id": document_id,
                    "source_document_fragment_id": source_document_fragment_id,
                    "artifact_id": artifact_id,
                    "filename": filename,
                    "chunk_count": len(spans),
                    "char_count": len(text),
                }
            )

        return {
            "status": "ok",
            "source_kind": "document_set",
            "chunks": chunk_rows,
            "fragment_ids": chunk_fragment_refs,
            "fragment_artifact_map": chunk_fragment_artifact_map,
            "document_stats": document_stats,
            "document_chunk_sets": hydrated_document_chunk_sets,
            "chunk_extraction_set": {
                "kind": "chunk_extraction_set",
                "source_subgraph_ref": source_subgraph_ref,
                "subgraph_ref": subgraph_ref,
                "extraction_refs": chunk_fragment_refs,
                "extractions": extractions,
                "chunk_extractions": extractions,
                "document_chunk_sets": document_chunk_sets,
                "edges": derivation_edges,
            },
            "summary": {
                "document_count": len(normalized_documents),
                "chunk_count": len(chunk_rows),
                "artifact_count": len({row["artifact_id"] for row in document_stats if row.get("artifact_id")}),
            },
        }

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)
