from __future__ import annotations

import json

from blake3 import blake3

from modelado.environment_scope import EnvironmentScope
from modelado.operators.core import OperatorEnv, OperatorParams
from modelado.operators.load_documents import LoadDocumentsOperator
from modelado.operators.registry import create_default_operator_registry


_DEV_SCOPE = EnvironmentScope(ref="refs/heads/run/test")
_DOCUMENT_MIME = "application/vnd.ikam.loaded-document+json"


def _document_fragment_ref(payload: dict[str, object]) -> str:
    stable_json = json.dumps(
        {"mime_type": _DOCUMENT_MIME, "value": payload},
        sort_keys=True,
        ensure_ascii=False,
    )
    return blake3(stable_json.encode("utf-8")).hexdigest()


def test_load_documents_operator_normalizes_assets_into_documents_and_fragment_maps() -> None:
    operator = LoadDocumentsOperator()
    env = OperatorEnv(seed=42, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    params = OperatorParams(
        name="load.documents",
        parameters={
            "artifact_id": "artifact://bundle",
            "mime_type": "application/pdf",
            "assets": [
                {
                    "artifact_id": "artifact://doc-a",
                    "filename": "doc-a.pdf",
                    "mime_type": "application/pdf",
                    "reader_key": "pdf_reader",
                    "reader_library": "reader.lib",
                    "reader_method": "Reader.load",
                    "status": "success",
                    "documents": [
                        {
                            "id": "doc-1",
                            "text": "Alpha beta",
                            "metadata": {
                                "artifact_id": "artifact://doc-a",
                                "file_name": "doc-a.pdf",
                                "page": 1,
                            },
                        },
                        {
                            "id": "doc-2",
                            "text": "Gamma delta",
                            "metadata": {"filename": "doc-a.pdf"},
                        },
                    ],
                },
                {
                    "artifact_id": "artifact://doc-b",
                    "filename": "slides.pptx",
                    "mime_type": "application/octet-stream",
                    "reader_key": "pptx_reader",
                    "reader_library": "reader.lib",
                    "reader_method": "Reader.load",
                    "status": "unsupported",
                    "error_message": "unsupported mime",
                    "documents": [],
                },
            ],
        },
    )

    result = operator.apply(None, params, env)

    assert result["status"] == "ok"
    assert result["summary"] == {
        "document_count": 2,
        "asset_count": 2,
        "loaded_asset_count": 1,
        "errored_asset_count": 0,
        "unsupported_asset_count": 1,
        "reader_summary": {"pdf_reader": 1, "pptx_reader": 1},
    }

    assert result["documents"] == [
        {
            "id": "doc-1",
            "text": "Alpha beta",
            "metadata": {
                "artifact_id": "artifact://doc-a",
                "file_name": "doc-a.pdf",
                "page": 1,
            },
            "artifact_id": "artifact://doc-a",
            "filename": "doc-a.pdf",
            "mime_type": "application/pdf",
            "asset_index": 0,
            "reader_key": "pdf_reader",
            "reader_method": "Reader.load",
        },
        {
            "id": "doc-2",
            "text": "Gamma delta",
            "metadata": {"filename": "doc-a.pdf"},
            "artifact_id": "artifact://doc-a",
            "filename": "doc-a.pdf",
            "mime_type": "application/pdf",
            "asset_index": 0,
            "reader_key": "pdf_reader",
            "reader_method": "Reader.load",
        },
    ]

    expected_first_ref = _document_fragment_ref(
        {
            "document_id": "doc-1",
            "text": "Alpha beta",
            "metadata": {
                "artifact_id": "artifact://doc-a",
                "file_name": "doc-a.pdf",
                "page": 1,
            },
            "artifact_id": "artifact://doc-a",
            "filename": "doc-a.pdf",
            "mime_type": "application/pdf",
        }
    )
    expected_second_ref = _document_fragment_ref(
        {
            "document_id": "doc-2",
            "text": "Gamma delta",
            "metadata": {"filename": "doc-a.pdf"},
            "artifact_id": "artifact://doc-a",
            "filename": "doc-a.pdf",
            "mime_type": "application/pdf",
        }
    )
    assert result["document_fragment_refs"] == [expected_first_ref, expected_second_ref]
    assert result["fragment_artifact_map"] == {
        expected_first_ref: "artifact://doc-a",
        expected_second_ref: "artifact://doc-a",
    }
    assert result["document_loads"] == [
        {
            "artifact_id": "artifact://doc-a",
            "filename": "doc-a.pdf",
            "mime_type": "application/pdf",
            "reader_key": "pdf_reader",
            "reader_library": "reader.lib",
            "reader_method": "Reader.load",
            "status": "success",
            "document_count": 2,
        },
        {
            "artifact_id": "artifact://doc-b",
            "filename": "slides.pptx",
            "mime_type": "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "reader_key": "pptx_reader",
            "reader_library": "reader.lib",
            "reader_method": "Reader.load",
            "status": "unsupported",
            "document_count": 0,
            "error_message": "unsupported mime",
        },
    ]


def test_load_documents_operator_uses_source_artifact_inputs_when_assets_are_absent() -> None:
    operator = LoadDocumentsOperator()
    env = OperatorEnv(seed=7, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    params = OperatorParams(
        name="load.documents",
        parameters={
            "artifact_id": "artifact://source",
            "filename": "notes.txt",
            "mime_type": "text/plain",
            "reader_key": "plain_text_reader",
            "reader_method": "Reader.load",
            "documents": [
                {
                    "id": "doc-root",
                    "text": "Root text",
                    "metadata": {},
                }
            ],
        },
    )

    result = operator.apply(None, params, env)

    assert result["status"] == "ok"
    assert result["summary"]["asset_count"] == 1
    assert result["summary"]["document_count"] == 1
    assert result["documents"] == [
        {
            "id": "doc-root",
            "text": "Root text",
            "metadata": {},
            "artifact_id": "artifact://source",
            "filename": "notes.txt",
            "mime_type": "text/plain",
            "asset_index": 0,
            "reader_key": "plain_text_reader",
            "reader_method": "Reader.load",
        }
    ]
    assert result["document_loads"] == [
        {
            "artifact_id": "artifact://source",
            "filename": "notes.txt",
            "mime_type": "text/plain",
            "reader_key": "plain_text_reader",
            "reader_library": "llama_index.core",
            "reader_method": "Reader.load",
            "status": "success",
            "document_count": 1,
        }
    ]


def test_load_documents_operator_counts_only_dict_documents() -> None:
    operator = LoadDocumentsOperator()
    env = OperatorEnv(seed=9, renderer_version="1", policy="strict", env_scope=_DEV_SCOPE)
    params = OperatorParams(
        name="load.documents",
        parameters={
            "artifact_id": "artifact://bundle",
            "assets": [
                {
                    "artifact_id": "artifact://first",
                    "filename": "same.md",
                    "mime_type": "text/markdown",
                    "status": "success",
                    "documents": [
                        {
                            "id": "same-doc",
                            "text": "Same text",
                            "metadata": {"filename": "same.md"},
                        },
                        None,
                    ],
                },
            ],
        },
    )

    result = operator.apply(None, params, env)

    assert result["document_loads"][0]["document_count"] == 1
    assert result["summary"]["document_count"] == 1
    assert len(result["document_fragment_refs"]) == 1
    assert result["fragment_artifact_map"] == {
        result["document_fragment_refs"][0]: "artifact://first",
    }


def test_default_registry_registers_load_documents_operator(monkeypatch) -> None:
    created: list[_FakeRegistry] = []

    class _FakeRegistry:
        def __init__(self, _cx: object, _manager: object, namespace: str) -> None:
            self.namespace = namespace
            self.entries: dict[str, object] = {}
            created.append(self)

        def list_keys(self) -> list[str]:
            return sorted(self.entries)

        def register(self, key: str, entry: object) -> None:
            self.entries[key] = entry

        def get(self, key: str) -> object | None:
            return self.entries.get(key)

    monkeypatch.setattr("modelado.operators.registry.OperatorRegistryAdapter", _FakeRegistry)

    registry = create_default_operator_registry(object(), object(), namespace="operators.default.test")

    assert isinstance(registry.get("modelado/operators/load_documents"), LoadDocumentsOperator)
