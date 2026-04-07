import os
import json
import tempfile
from pathlib import Path
from typing import Any, Dict


def _choose_reader(file_name: str) -> tuple[str, str, str]:
    suffix = Path(file_name).suffix.lower()
    if suffix == ".json":
        return ("json_reader", "llama_index.readers.json", "JSONReader.load_data")
    return ("simple_directory_reader", "llama_index.core", "SimpleDirectoryReader.load_data")


def _load_json_documents(*, raw_bytes: bytes, file_name: str) -> list[dict[str, Any]]:
    parsed = json.loads(raw_bytes.decode("utf-8"))
    text = json.dumps(parsed, indent=2, ensure_ascii=False, sort_keys=True)
    return [
        {
            "id": file_name,
            "text": text,
            "metadata": {"file_name": file_name, "reader": "json"},
        }
    ]

def run(payload: Dict[str, Any], context: Dict[str, Any]) -> Any:
    """
    LoadArtifacts operation.
    Expected to receive raw bytes or a file path in the fragment,
    parses it using LlamaIndex's SimpleDirectoryReader, and returns
    structured Document data.
    """
    fragment = payload.get("fragment", {})
    params = payload.get("params", {})
    
    file_path = params.get("file_path")
    raw_bytes = params.get("raw_bytes")
    file_name = str(params.get("file_name") or file_path or "document.txt")
    reader_key, reader_library, reader_method = _choose_reader(file_name)
    
    if not file_path and not raw_bytes:
        raise ValueError("Must provide 'file_path' or 'raw_bytes' in params")

    yield {
        "type": "trace",
        "event_type": "artifact_loading_started",
        "payload": {
            "file_path": file_path,
            "file_name": file_name,
            "has_raw_bytes": bool(raw_bytes),
            "reader_key": reader_key,
            "reader_library": reader_library,
            "reader_method": reader_method,
        }
    }

    docs_out: list[dict[str, Any]] = []

    if reader_key == "json_reader":
        payload_bytes = raw_bytes.encode("utf-8") if isinstance(raw_bytes, str) else raw_bytes
        if not isinstance(payload_bytes, (bytes, bytearray)):
            raise ValueError("JSONReader requires raw bytes")
        docs_out = _load_json_documents(raw_bytes=bytes(payload_bytes), file_name=file_name)
    else:
        documents = []
        from llama_index.core import SimpleDirectoryReader

        if file_path:
            reader = SimpleDirectoryReader(input_files=[file_path])
            documents = reader.load_data()
        elif raw_bytes:
            payload_bytes = raw_bytes.encode("utf-8") if isinstance(raw_bytes, str) else raw_bytes
            suffix = Path(file_name).suffix or ".txt"
            with tempfile.NamedTemporaryFile(delete=False, suffix=suffix) as tmp:
                tmp.write(payload_bytes)
                tmp_path = tmp.name

            try:
                reader = SimpleDirectoryReader(input_files=[tmp_path])
                documents = reader.load_data()
            finally:
                os.remove(tmp_path)

        for doc in documents:
            doc_info = {
                "id": doc.doc_id,
                "text": doc.text,
                "metadata": doc.metadata,
            }
            docs_out.append(doc_info)

    for i, doc in enumerate(docs_out):
        
        yield {
            "type": "trace",
            "event_type": "artifact_loaded",
            "payload": {
                "doc_id": doc["id"],
                "index": i,
                "text_preview": doc["text"][:200] + "..." if len(doc["text"]) > 200 else doc["text"],
                "reader_key": reader_key,
                "reader_library": reader_library,
                "reader_method": reader_method,
            }
        }
            
    # Yield the final result
    yield {
        "type": "result",
        "status": "success",
        "result": {
            "documents": docs_out,
            "reader_key": reader_key,
            "reader_library": reader_library,
            "reader_method": reader_method,
        },
        "context_mutations": {
            "meta_updates": {
                "artifacts_loaded": len(docs_out)
            }
        }
    }
