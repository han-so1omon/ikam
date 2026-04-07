from __future__ import annotations

import sys
from pathlib import Path

import pytest
from pydantic import ValidationError


ROOT = Path(__file__).resolve().parents[1]
SRC = ROOT / "src"
if str(SRC) not in sys.path:
    sys.path.insert(0, str(SRC))


def test_map_generation_request_validates_required_fields() -> None:
    from mcp_ikam.contracts import MapGenerationRequest

    request = MapGenerationRequest.model_validate(
        {
            "artifact_bundle": {
                "corpus_id": "project/corpus",
                "artifacts": [
                    {"artifact_id": "project/doc-1", "file_name": "doc-1.md", "mime_type": "text/markdown"}
                ],
            },
            "map_definition": {
                "goal": "Build a map that maximizes dedup and insight",
                "allowed_profiles": ["modelado/prose-backbone@1"],
                "max_nodes": 24,
                "max_depth": 3,
            },
        }
    )

    assert request.artifact_bundle.corpus_id == "project/corpus"
    assert request.map_definition.max_nodes == 24


def test_map_generation_response_requires_contract_fields() -> None:
    from mcp_ikam.contracts import MapGenerationResponse

    response = MapGenerationResponse.model_validate(
        {
            "map_subgraph": {
                "root_node_id": "map:root",
                "nodes": [
                    {"id": "map:root", "title": "Corpus", "kind": "corpus"},
                    {"id": "map:seg:a", "title": "Segment A", "kind": "segment"},
                ],
                "relationships": [
                    {"type": "map_contains", "source": "map:root", "target": "map:seg:a"}
                ],
            },
            "map_dna": {"fingerprint": "abc123"},
            "segment_anchors": {
                "map:seg:a": [
                    {
                        "artifact_id": "project/doc-1",
                        "locator_type": "section",
                        "locator": "#intro",
                        "confidence": 0.91,
                    }
                ]
            },
            "segment_candidates": [
                {
                    "segment_id": "map:seg:a",
                    "title": "Segment A",
                    "artifact_ids": ["project/doc-1"],
                    "rationale": "high thematic cohesion",
                }
            ],
            "profile_candidates": {"map:seg:a": ["modelado/prose-backbone@1"]},
            "generation_provenance": {
                "provider": "openai",
                "model": "gpt-4o-mini",
                "prompt_version": "v1",
                "temperature": 0.2,
                "seed": 7,
            },
        }
    )

    assert response.map_subgraph["root_node_id"] == "map:root"
    assert response.map_dna.fingerprint == "abc123"


def test_map_generation_response_rejects_missing_provenance() -> None:
    from mcp_ikam.contracts import MapGenerationResponse

    with pytest.raises(ValidationError):
        MapGenerationResponse.model_validate(
            {
                "map_subgraph": {
                    "root_node_id": "map:root",
                    "nodes": [{"id": "map:root", "title": "Corpus", "kind": "corpus"}],
                    "relationships": [],
                },
                "map_dna": {"fingerprint": "abc123"},
                "segment_anchors": {},
                "segment_candidates": [],
                "profile_candidates": {},
            }
        )


def test_map_generation_response_rejects_empty_map_subgraph_root_id() -> None:
    from mcp_ikam.contracts import MapGenerationResponse

    with pytest.raises(ValidationError):
        MapGenerationResponse.model_validate(
            {
                "map_subgraph": {
                    "root_node_id": "",
                    "nodes": [{"id": "map:root", "title": "Corpus", "kind": "corpus"}],
                    "relationships": [],
                },
                "map_dna": {"fingerprint": "abc123"},
                "segment_anchors": {},
                "segment_candidates": [],
                "profile_candidates": {},
                "generation_provenance": {
                    "provider": "openai",
                    "model": "gpt-4o-mini",
                    "prompt_version": "v1",
                    "temperature": 0.2,
                    "seed": 7,
                },
            }
        )


def test_map_generation_request_rejects_unknown_fields() -> None:
    from mcp_ikam.contracts import MapGenerationRequest

    with pytest.raises(ValidationError):
        MapGenerationRequest.model_validate(
            {
                "artifact_bundle": {
                    "corpus_id": "project/corpus",
                    "artifacts": [{"artifact_id": "project/doc-1"}],
                },
                "map_definition": {
                    "goal": "map docs",
                    "allowed_profiles": ["modelado/prose-backbone@1"],
                },
                "extra_field": "not-allowed",
            }
        )


def test_map_generation_request_accepts_context() -> None:
    from mcp_ikam.contracts import MapGenerationRequest

    request = MapGenerationRequest.model_validate(
            {
                "artifact_bundle": {
                    "corpus_id": "project/corpus",
                    "artifacts": [{"artifact_id": "project/doc-1"}],
                },
                "map_definition": {
                    "goal": "map docs",
                    "allowed_profiles": ["modelado/prose-backbone@1"],
                },
                "context": {"project_id": "proj-1", "case_id": "case-1"},
            }
        )

    assert request.context is not None
    assert request.context.project_id == "proj-1"


def test_map_generation_request_accepts_document_fragment_refs() -> None:
    from mcp_ikam.contracts import MapGenerationRequest

    request = MapGenerationRequest.model_validate(
        {
            "artifact_bundle": {
                "corpus_id": "project/corpus",
                "artifacts": [{"artifact_id": "project/doc-1"}],
            },
            "map_definition": {
                "goal": "map docs",
                "allowed_profiles": ["modelado/prose-backbone@1"],
            },
            "document_fragment_refs": ["cas:doc-1", "cas:doc-2"],
        }
    )

    assert request.document_fragment_refs == ["cas:doc-1", "cas:doc-2"]


def test_map_generation_request_rejects_surface_fragments_field() -> None:
    from mcp_ikam.contracts import MapGenerationRequest

    with pytest.raises(ValidationError):
        MapGenerationRequest.model_validate(
            {
                "artifact_bundle": {
                    "corpus_id": "project/corpus",
                    "artifacts": [{"artifact_id": "project/doc-1"}],
                },
                "map_definition": {
                    "goal": "map docs",
                    "allowed_profiles": ["modelado/prose-backbone@1"],
                },
                "surface_fragments": [{"id": "frag-1"}],
            }
        )


def test_response_rejects_unknown_relationship_nodes() -> None:
    from mcp_ikam.contracts import MapGenerationResponse

    with pytest.raises(ValidationError):
        MapGenerationResponse.model_validate(
            {
                "map_subgraph": {
                    "root_node_id": "map:root",
                    "nodes": [{"id": "map:root", "title": "Corpus", "kind": "corpus"}],
                    "relationships": [
                        {"type": "map_contains", "source": "map:root", "target": "map:missing"}
                    ],
                },
                "map_dna": {"fingerprint": "abc123"},
                "segment_anchors": {},
                "segment_candidates": [],
                "profile_candidates": {},
                "generation_provenance": {"provider": "openai", "model": "gpt-4o-mini", "prompt_version": "v1"},
            }
        )


def test_response_rejects_invalid_provenance_values() -> None:
    from mcp_ikam.contracts import MapGenerationResponse

    with pytest.raises(ValidationError):
        MapGenerationResponse.model_validate(
            {
                "map_subgraph": {
                    "root_node_id": "map:root",
                    "nodes": [{"id": "map:root", "title": "Corpus", "kind": "corpus"}],
                    "relationships": [],
                },
                "map_dna": {"fingerprint": "abc123"},
                "segment_anchors": {},
                "segment_candidates": [],
                "profile_candidates": {},
                "generation_provenance": {
                    "provider": "openai",
                    "model": "gpt-4o-mini",
                    "prompt_version": "v1",
                    "temperature": -0.1,
                    "seed": -2,
                },
            }
        )
