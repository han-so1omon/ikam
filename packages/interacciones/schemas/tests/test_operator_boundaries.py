"""Contract tests for authored operator boundary declarations."""

import pytest
from pydantic import ValidationError


def test_operator_boundary_declarations_round_trip() -> None:
    from interacciones.schemas import OperatorBoundaries

    boundaries = OperatorBoundaries.model_validate(
        {
            "input": [
                {
                    "name": "document_set",
                    "mime_type": "application/ikam-document-set+v1+json",
                }
            ],
            "output": [
                {
                    "name": "chunk_extraction_set",
                    "mime_type": "application/ikam-chunk-extraction-set+v1+json",
                }
            ],
        }
    )

    assert boundaries.model_dump(mode="json") == {
        "input": [
            {
                "name": "document_set",
                "mime_type": "application/ikam-document-set+v1+json",
            }
        ],
        "output": [
            {
                "name": "chunk_extraction_set",
                "mime_type": "application/ikam-chunk-extraction-set+v1+json",
            }
        ],
    }


def test_operator_boundary_spec_requires_name_and_mime_type() -> None:
    from interacciones.schemas import OperatorBoundarySpec

    with pytest.raises(ValidationError):
        OperatorBoundarySpec.model_validate({"mime_type": "application/ikam-document-set+v1+json"})

    with pytest.raises(ValidationError):
        OperatorBoundarySpec.model_validate({"name": "document_set"})


def test_workflow_node_accepts_authored_boundaries_on_dispatch_nodes() -> None:
    from interacciones.schemas import WorkflowNode

    node = WorkflowNode.model_validate(
        {
            "node_id": "parse-chunk",
            "kind": "dispatch_executor",
            "capability": "python.chunk_documents",
            "boundaries": {
                "input": [
                    {
                        "name": "document_set",
                        "mime_type": "application/ikam-document-set+v1+json",
                    }
                ],
                "output": [
                    {
                        "name": "chunk_extraction_set",
                        "mime_type": "application/ikam-chunk-extraction-set+v1+json",
                    }
                ],
            },
        }
    )

    dumped = node.model_dump(mode="json")

    assert dumped["boundaries"] == {
        "input": [
            {
                "name": "document_set",
                "mime_type": "application/ikam-document-set+v1+json",
            }
        ],
        "output": [
            {
                "name": "chunk_extraction_set",
                "mime_type": "application/ikam-chunk-extraction-set+v1+json",
            }
        ],
    }


def test_non_dispatch_nodes_reject_authored_boundaries() -> None:
    from interacciones.schemas import WorkflowNode

    with pytest.raises(ValidationError):
        WorkflowNode.model_validate(
            {
                "node_id": "wait",
                "kind": "wait_for_result",
                "boundaries": {
                    "input": [
                        {
                            "name": "document_set",
                            "mime_type": "application/ikam-document-set+v1+json",
                        }
                    ]
                },
            }
        )

    with pytest.raises(ValidationError):
        WorkflowNode.model_validate(
            {
                "node_id": "wait-empty",
                "kind": "wait_for_result",
                "boundaries": {},
            }
        )
