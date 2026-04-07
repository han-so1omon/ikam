from __future__ import annotations

import json
import mimetypes
from typing import Any

from blake3 import blake3

from .core import Operator, OperatorEnv, OperatorParams, ProvenanceRecord, record_provenance


DOCUMENT_MIME = "application/vnd.ikam.loaded-document+json"


def _resolve_asset_mime_type(raw_mime_type: str | None, file_name: str | None) -> str:
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


def _document_fragment_ref(value: dict[str, Any]) -> str:
    stable_json = json.dumps(
        {"mime_type": DOCUMENT_MIME, "value": value},
        sort_keys=True,
        ensure_ascii=False,
    )
    return blake3(stable_json.encode("utf-8")).hexdigest()


class LoadDocumentsOperator(Operator):
    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> dict[str, Any]:
        del fragment, env
        raw = params.parameters
        artifact_id = str(raw.get("artifact_id") or "")
        default_filename = str(raw.get("filename") or artifact_id.rsplit("/", 1)[-1] or artifact_id)
        default_mime = _resolve_asset_mime_type(raw.get("mime_type"), default_filename)

        assets = raw.get("assets") if isinstance(raw.get("assets"), list) else None
        if not assets:
            assets = [{
                "artifact_id": artifact_id,
                "filename": default_filename,
                "mime_type": default_mime,
                "reader_key": raw.get("reader_key"),
                "reader_library": raw.get("reader_library"),
                "reader_method": raw.get("reader_method"),
                "status": raw.get("status"),
                "error_message": raw.get("error_message"),
                "documents": raw.get("documents"),
            }]

        normalized_documents: list[dict[str, Any]] = []
        document_fragment_refs: list[str] = []
        fragment_artifact_map: dict[str, str] = {}
        document_loads: list[dict[str, Any]] = []
        loaded_asset_count = 0
        errored_asset_count = 0
        unsupported_asset_count = 0
        reader_summary: dict[str, int] = {}

        for asset_index, asset in enumerate(item for item in assets if isinstance(item, dict)):
            asset_artifact_id = str(asset.get("artifact_id") or artifact_id)
            filename = str(asset.get("filename") or default_filename)
            mime_type = _resolve_asset_mime_type(asset.get("mime_type"), filename) or default_mime
            reader_key = str(asset.get("reader_key") or "simple_directory_reader")
            reader_library = str(asset.get("reader_library") or "llama_index.core")
            reader_method = str(asset.get("reader_method") or "SimpleDirectoryReader.load_data")
            status = str(asset.get("status") or "success")
            error_message = str(asset.get("error_message") or "").strip()
            documents = asset.get("documents") if isinstance(asset.get("documents"), list) else []
            normalized_asset_documents = [document for document in documents if isinstance(document, dict)]

            reader_summary[reader_key] = reader_summary.get(reader_key, 0) + 1
            if status == "unsupported":
                unsupported_asset_count += 1
            elif status == "error":
                errored_asset_count += 1
            elif status == "success":
                loaded_asset_count += 1

            load_record = {
                "artifact_id": asset_artifact_id,
                "filename": filename,
                "mime_type": mime_type,
                "reader_key": reader_key,
                "reader_library": reader_library,
                "reader_method": reader_method,
                "status": status,
                "document_count": len(normalized_asset_documents),
            }
            if error_message:
                load_record["error_message"] = error_message
            document_loads.append(load_record)

            if status != "success":
                continue

            for document in normalized_asset_documents:
                doc_id = str(document.get("id") or f"doc-{len(normalized_documents)}")
                text = str(document.get("text") or "")
                metadata = document.get("metadata") if isinstance(document.get("metadata"), dict) else {}
                doc_filename = str(metadata.get("file_name") or metadata.get("filename") or filename)
                doc_artifact_id = str(metadata.get("artifact_id") or asset_artifact_id)
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
                fragment_value = {
                    "document_id": doc_id,
                    "text": text,
                    "metadata": metadata,
                    "artifact_id": doc_artifact_id,
                    "filename": doc_filename,
                    "mime_type": mime_type,
                }
                fragment_ref = _document_fragment_ref(fragment_value)
                document_fragment_refs.append(fragment_ref)
                fragment_artifact_map.setdefault(fragment_ref, doc_artifact_id)

        summary = {
            "document_count": len(normalized_documents),
            "asset_count": len([item for item in assets if isinstance(item, dict)]),
            "loaded_asset_count": loaded_asset_count,
            "errored_asset_count": errored_asset_count,
            "unsupported_asset_count": unsupported_asset_count,
            "reader_summary": reader_summary,
        }
        return {
            "status": "ok",
            "documents": normalized_documents,
            "document_fragment_refs": document_fragment_refs,
            "fragment_artifact_map": fragment_artifact_map,
            "document_loads": document_loads,
            "summary": summary,
        }

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)
