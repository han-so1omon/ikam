from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))

from interacciones.schemas import RichPetriTransition  # noqa: E402
from modelado.executors.transition_validation import build_runtime_transition_validation, build_transition_validation  # noqa: E402


def _validator_specs(*validators: dict[str, object]) -> list[dict[str, object]]:
    transition = RichPetriTransition.model_validate(
        {
            "transition_id": "transition-1",
            "label": "Transition 1",
            "capability": "python.transition_1",
            "validators": list(validators),
        }
    )
    return [validator.model_dump(mode="json") for validator in transition.validators]


def test_build_transition_validation_validates_load_documents_handoff() -> None:
    specs = _validator_specs(
        {
            "name": "input-url",
            "direction": "input",
            "kind": "type",
            "selector": "input.url",
            "target": "value",
            "config": {"schema": {"type": "object", "title": "url", "required": ["kind", "location"]}},
        },
        {
            "name": "output-document-set",
            "direction": "output",
            "kind": "type",
            "selector": "output.document_set",
            "target": "value",
            "config": {
                "schema": {
                    "type": "object",
                    "title": "document_set",
                    "required": ["kind", "artifact_head_ref", "subgraph_ref", "document_refs"],
                }
            },
        },
    )

    payload = build_transition_validation(
        validators=specs,
        runtime_inputs={
            "url": [
                {
                    "value": {
                        "kind": "url",
                        "location": "/repo/tests/fixtures/cases/s-local-retail-v01",
                        "mime_type": "text/markdown",
                    }
                }
            ]
        },
        runtime_outputs={
            "document_set": [
                {
                    "value": {
                        "kind": "document_set",
                        "artifact_head_ref": "artifact://s-local-retail-v01",
                        "subgraph_ref": "subgraph://run-1-document-set-step-1",
                        "document_refs": ["frag-doc-1"],
                    }
                }
            ]
        },
    )

    assert [spec["name"] for spec in payload["specs"]] == ["input-url", "output-document-set"]
    assert payload["resolved_inputs"]["url"][0]["value"]["location"].endswith("/tests/fixtures/cases/s-local-retail-v01")
    assert payload["resolved_outputs"]["document_set"][0]["value"]["kind"] == "document_set"
    assert payload["resolved_inputs"]["url"][0]["inspection"] == {
        "value_kind": "url",
        "summary": "url /repo/tests/fixtures/cases/s-local-retail-v01",
        "refs": [],
        "content": {
            "kind": "url",
            "location": "/repo/tests/fixtures/cases/s-local-retail-v01",
            "mime_type": "text/markdown",
        },
        "resolved_refs": [],
    }
    assert payload["resolved_inputs"]["url"][0]["inspection_stub"] == {
        "id": "artifact:/repo/tests/fixtures/cases/s-local-retail-v01",
        "kind": "artifact",
        "ir_kind": "url",
        "label": "/repo/tests/fixtures/cases/s-local-retail-v01",
        "summary": "url /repo/tests/fixtures/cases/s-local-retail-v01",
        "inspection_ref": "inspect://artifact//repo/tests/fixtures/cases/s-local-retail-v01",
    }
    assert payload["resolved_outputs"]["document_set"][0]["inspection"] == {
        "value_kind": "document_set",
        "summary": "document_set 1 ref",
        "refs": ["frag-doc-1"],
        "content": {
            "kind": "document_set",
            "artifact_head_ref": "artifact://s-local-retail-v01",
            "subgraph_ref": "subgraph://run-1-document-set-step-1",
            "document_refs": ["frag-doc-1"],
        },
        "resolved_refs": [],
    }
    assert payload["resolved_outputs"]["document_set"][0]["inspection_stub"] == {
        "id": "subgraph:subgraph://run-1-document-set-step-1",
        "kind": "subgraph",
        "ir_kind": "document_set",
        "label": "document_set",
        "summary": "document_set 1 ref",
        "inspection_ref": "inspect://subgraph/subgraph://run-1-document-set-step-1",
    }
    results = {item["name"]: item for item in payload["results"]}
    assert results["input-url"]["status"] == "passed"
    assert results["output-document-set"]["status"] == "passed"


def test_build_transition_validation_validates_parse_claims_handoff() -> None:
    specs = _validator_specs(
        {
            "name": "input-entity-relationship-set",
            "direction": "input",
            "kind": "type",
            "selector": "input.entity_relationship_set",
            "target": "value",
            "config": {
                "schema": {
                    "type": "object",
                    "title": "entity_relationship_set",
                    "required": ["kind", "source_subgraph_ref", "subgraph_ref", "entity_relationship_refs"],
                }
            },
        },
        {
            "name": "output-claim-set",
            "direction": "output",
            "kind": "type",
            "selector": "output.claim_set",
            "target": "value",
            "config": {
                "schema": {
                    "type": "object",
                    "title": "claim_set",
                    "required": ["kind", "source_subgraph_ref", "subgraph_ref", "claim_refs"],
                }
            },
        },
    )

    payload = build_transition_validation(
        validators=specs,
        runtime_inputs={
            "entity_relationship_set": [
                {
                    "value": {
                        "kind": "entity_relationship_set",
                        "source_subgraph_ref": "subgraph://run-1-chunk-extraction-set-step-entities",
                        "subgraph_ref": "subgraph://run-1-entity-relationship-set-step-entities",
                        "entity_relationship_refs": ["frag-ir-1"],
                    }
                }
            ]
        },
        runtime_outputs={
            "claim_set": [
                {
                    "value": {
                        "kind": "claim_set",
                        "source_subgraph_ref": "subgraph://run-1-entity-relationship-set-step-entities",
                        "subgraph_ref": "subgraph://run-1-claim-set-step-claims",
                        "claim_refs": ["frag-claim-1"],
                    }
                }
            ]
        },
    )

    assert [spec["name"] for spec in payload["specs"]] == ["input-entity-relationship-set", "output-claim-set"]
    assert payload["resolved_inputs"]["entity_relationship_set"][0]["value"]["kind"] == "entity_relationship_set"
    assert payload["resolved_outputs"]["claim_set"][0]["value"]["kind"] == "claim_set"
    assert payload["resolved_inputs"]["entity_relationship_set"][0]["inspection"] == {
        "value_kind": "entity_relationship_set",
        "summary": "entity_relationship_set 1 ref",
        "refs": ["frag-ir-1"],
        "content": {
            "kind": "entity_relationship_set",
            "source_subgraph_ref": "subgraph://run-1-chunk-extraction-set-step-entities",
            "subgraph_ref": "subgraph://run-1-entity-relationship-set-step-entities",
            "entity_relationship_refs": ["frag-ir-1"],
        },
        "resolved_refs": [],
    }
    assert payload["resolved_outputs"]["claim_set"][0]["inspection"] == {
        "value_kind": "claim_set",
        "summary": "claim_set 1 ref",
        "refs": ["frag-claim-1"],
        "content": {
            "kind": "claim_set",
            "source_subgraph_ref": "subgraph://run-1-entity-relationship-set-step-entities",
            "subgraph_ref": "subgraph://run-1-claim-set-step-claims",
            "claim_refs": ["frag-claim-1"],
        },
        "resolved_refs": [],
    }
    results = {item["name"]: item for item in payload["results"]}
    assert results["input-entity-relationship-set"]["status"] == "passed"
    assert results["output-claim-set"]["status"] == "passed"


def test_build_transition_validation_filters_unrelated_runtime_inputs_by_selector() -> None:
    specs = _validator_specs(
        {
            "name": "input-document-set",
            "direction": "input",
            "kind": "type",
            "selector": "input.document_set",
            "target": "value",
            "config": {
                "schema": {
                    "type": "object",
                    "title": "document_set",
                    "required": ["kind", "artifact_head_ref", "subgraph_ref", "document_refs"],
                }
            },
        }
    )

    payload = build_transition_validation(
        validators=specs,
        runtime_inputs={
            "url": [{"value": {"kind": "url", "location": "/repo/tests/fixtures/cases/s-local-retail-v01"}}],
            "document_set": [{"value": {"kind": "document_set", "artifact_head_ref": "artifact://case", "subgraph_ref": "subgraph://run-document-set-step-load", "document_refs": ["frag-doc-1"]}}],
        },
        runtime_outputs={},
    )

    assert list(payload["resolved_inputs"].keys()) == ["document_set"]


def test_build_transition_validation_inspects_fragment_backed_entries() -> None:
    specs = _validator_specs(
        {
            "name": "output-document-fragment-refs",
            "direction": "output",
            "kind": "type",
            "selector": "output.document_fragment_refs",
            "target": "value",
            "config": {
                "schema": {
                    "type": "object",
                    "title": "loaded_document",
                    "required": ["document_id", "filename", "text"],
                }
            },
        }
    )

    payload = build_transition_validation(
        validators=specs,
        runtime_inputs={},
        runtime_outputs={
            "document_fragment_refs": [
                {
                    "fragment_id": "frag-loaded-document-1",
                    "cas_id": "frag-loaded-document-1",
                    "mime_type": "application/vnd.ikam.loaded-document+json",
                    "value": {
                        "document_id": "doc-1",
                        "filename": "brief.md",
                        "text": "Invoice 123 is ready",
                    },
                }
            ]
        },
    )

    assert payload["resolved_outputs"]["document_fragment_refs"][0]["inspection"] == {
        "value_kind": "loaded_document",
        "summary": "loaded_document doc-1",
        "refs": ["frag-loaded-document-1"],
        "content": {
            "document_id": "doc-1",
            "filename": "brief.md",
            "text": "Invoice 123 is ready",
        },
        "resolved_refs": [
            {
                "fragment_id": "frag-loaded-document-1",
                "cas_id": "frag-loaded-document-1",
                "mime_type": "application/vnd.ikam.loaded-document+json",
                "name": "brief.md",
                "inspection_ref": "inspect://fragment/frag-loaded-document-1",
                "value": {
                    "document_id": "doc-1",
                    "filename": "brief.md",
                    "text": "Invoice 123 is ready",
                },
            }
        ],
    }
    assert payload["resolved_outputs"]["document_fragment_refs"][0]["inspection_stub"] == {
        "id": "fragment:frag-loaded-document-1",
        "kind": "fragment",
        "ir_kind": "loaded_document",
        "label": "doc-1",
        "summary": "loaded_document doc-1",
        "inspection_ref": "inspect://fragment/frag-loaded-document-1",
    }


def test_build_runtime_transition_validation_emits_canonical_locator_refs() -> None:
    specs = _validator_specs(
        {
            "name": "output-document-set",
            "direction": "output",
            "kind": "type",
            "selector": "output.document_set",
            "target": "value",
            "config": {
                "schema": {
                    "type": "object",
                    "title": "document_set",
                    "required": ["kind", "artifact_head_ref", "subgraph_ref", "document_refs"],
                }
            },
        }
    )

    payload = build_runtime_transition_validation(
        validators=specs,
        artifact_id="artifact-case-1",
        mime_type="text/markdown",
        fixture_path="/repo/tests/fixtures/cases/s-local-retail-v01",
        run_id="run-1",
        step_id="step-load",
        step_outputs={"document_fragment_refs": ["frag-doc-1"]},
        environment_fragments=[],
    )

    assert payload is not None
    value = payload["resolved_outputs"]["document_set"][0]["value"]
    assert value["artifact_head_ref"] == "artifact://artifact-case-1"
    assert value["subgraph_ref"] == "subgraph://run-1-document-set-step-load"
