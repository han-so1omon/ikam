"""Contract tests for richer Petri workflow schemas."""

import pytest
from pydantic import ValidationError

from interacciones.schemas import (
    ResolutionMode,
    RichPetriArc,
    RichPetriTransition,
    RichPetriWorkflow,
    SourceWorkflowStorageMode,
    SourceWorkflowStoragePolicy,
    TracePersistenceMode,
)


def test_rich_petri_workflow_round_trips_with_orchestration_intent() -> None:
    workflow = RichPetriWorkflow.model_validate(
        {
            "workflow_id": "ingestion-early-steps",
            "version": "2026-03-06",
            "places": [
                {"place_id": "place:start", "label": "Start"},
                {"place_id": "place:done", "label": "Done"},
            ],
            "transitions": [
                {
                    "transition_id": "dispatch-normalize",
                    "label": "Dispatch normalize",
                    "capability": "python.transform",
                    "policy": {"cost_tier": "standard"},
                    "constraints": {"locality": "local"},
                    "trace_policy": {"mode": "per_step"},
                }
            ],
            "arcs": [
                {
                    "source_kind": "place",
                    "source_id": "place:start",
                    "target_kind": "transition",
                    "target_id": "dispatch-normalize",
                },
                {
                    "source_kind": "transition",
                    "source_id": "dispatch-normalize",
                    "target_kind": "place",
                    "target_id": "place:done",
                },
            ],
        }
    )

    dumped = workflow.model_dump(mode="json")

    assert dumped["transitions"][0]["capability"] == "python.transform"
    assert dumped["transitions"][0]["trace_policy"] == {"mode": "per_step"}
    assert dumped["source_workflow_storage"] == {
        "mode": "default_on",
        "reconstructable_from_lowered_graph": True,
    }


def test_rich_petri_transition_supports_direct_executor_override() -> None:
    transition = RichPetriTransition(
        transition_id="dispatch-embed",
        label="Dispatch embed",
        capability="ml.embed",
        resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
        direct_executor_ref="executor://ml-primary",
    )

    dumped = transition.model_dump(mode="json")

    assert dumped["resolution_mode"] == "direct_executor_ref"
    assert dumped["direct_executor_ref"] == "executor://ml-primary"


def test_rich_petri_transition_supports_operator_declared_validators() -> None:
    transition = RichPetriTransition.model_validate(
        {
            "transition_id": "dispatch-validate",
            "label": "Dispatch validate",
            "capability": "python.validate",
            "validators": [
                {
                    "name": "input-url",
                    "direction": "input",
                    "kind": "type",
                    "selector": "input.url",
                    "target": "value",
                    "config": {
                        "schema": {
                            "type": "object",
                            "title": "url",
                            "required": ["kind", "location"],
                        }
                    },
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
            ],
        }
    )

    dumped = transition.model_dump(mode="json")

    assert dumped["validators"] == [
        {
            "name": "input-url",
            "direction": "input",
            "kind": "type",
            "selector": "input.url",
            "target": "value",
            "config": {
                "schema": {
                    "type": "object",
                    "title": "url",
                    "required": ["kind", "location"],
                }
            },
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
    ]


def test_rich_petri_transition_rejects_unsupported_validator_direction() -> None:
    with pytest.raises(ValidationError):
        RichPetriTransition.model_validate(
            {
                "transition_id": "dispatch-validate",
                "label": "Dispatch validate",
                "capability": "python.validate",
                "validators": [
                    {
                        "name": "input-url",
                        "direction": "sideways",
                        "kind": "type",
                        "selector": "input.url",
                        "target": "value",
                        "config": {
                            "schema": {
                                "type": "object",
                                "title": "url",
                                "required": ["kind", "location"],
                            }
                        },
                    }
                ],
            }
        )


def test_rich_petri_transition_rejects_unsupported_validator_kind() -> None:
    with pytest.raises(ValidationError):
        RichPetriTransition.model_validate(
            {
                "transition_id": "dispatch-validate",
                "label": "Dispatch validate",
                "capability": "python.validate",
                "validators": [
                    {
                        "name": "input-url",
                        "direction": "input",
                        "kind": "shape",
                        "selector": "input.url",
                        "target": "value",
                        "config": {
                            "schema": {
                                "type": "object",
                                "title": "url",
                                "required": ["kind", "location"],
                            }
                        },
                    }
                ],
            }
        )


def test_rich_petri_transition_rejects_type_validator_without_inline_schema() -> None:
    with pytest.raises(ValidationError):
        RichPetriTransition.model_validate(
            {
                "transition_id": "dispatch-validate",
                "label": "Dispatch validate",
                "capability": "python.validate",
                "validators": [
                    {
                        "name": "input-url",
                        "direction": "input",
                        "kind": "type",
                        "selector": "input.url",
                        "target": "value",
                        "config": {},
                    }
                ],
            }
        )


def test_rich_petri_transition_rejects_regex_validator_without_pattern() -> None:
    with pytest.raises(ValidationError):
        RichPetriTransition.model_validate(
            {
                "transition_id": "dispatch-validate",
                "label": "Dispatch validate",
                "capability": "python.validate",
                "validators": [
                    {
                        "name": "ensure-output-format",
                        "direction": "output",
                        "kind": "regex",
                        "selector": "result.identifier",
                        "target": "value.text",
                        "config": {},
                    }
                ],
            }
        )


def test_rich_petri_transition_rejects_validator_with_extra_fields() -> None:
    with pytest.raises(ValidationError):
        RichPetriTransition.model_validate(
            {
                "transition_id": "dispatch-validate",
                "label": "Dispatch validate",
                "capability": "python.validate",
                "validators": [
                    {
                        "name": "ensure-output-format",
                        "direction": "output",
                        "kind": "regex",
                        "selector": "result.identifier",
                        "target": "value.text",
                        "config": {"pattern": "^[a-z0-9-]+$"},
                        "unexpected": True,
                    }
                ],
            }
        )


def test_rich_petri_transition_requires_capability_for_direct_executor_override() -> None:
    with pytest.raises(ValidationError):
        RichPetriTransition(
            transition_id="dispatch-embed",
            label="Dispatch embed",
            resolution_mode=ResolutionMode.DIRECT_EXECUTOR_REF,
            direct_executor_ref="executor://ml-primary",
        )


def test_source_workflow_storage_policy_defaults_to_persisted_and_reconstructable() -> None:
    policy = SourceWorkflowStoragePolicy()

    assert policy.mode is SourceWorkflowStorageMode.DEFAULT_ON
    assert policy.reconstructable_from_lowered_graph is True


def test_source_workflow_storage_policy_supports_explicit_omit_mode() -> None:
    policy = SourceWorkflowStoragePolicy(mode=SourceWorkflowStorageMode.OMIT)

    assert policy.model_dump(mode="json") == {
        "mode": "omit",
        "reconstructable_from_lowered_graph": True,
    }


def test_source_workflow_storage_policy_supports_optional_mode() -> None:
    policy = SourceWorkflowStoragePolicy(mode=SourceWorkflowStorageMode.OPTIONAL)

    assert policy.model_dump(mode="json") == {
        "mode": "optional",
        "reconstructable_from_lowered_graph": True,
    }


def test_source_workflow_storage_policy_requires_reconstructability_when_not_persisted() -> None:
    with pytest.raises(ValidationError):
        SourceWorkflowStoragePolicy(
            mode=SourceWorkflowStorageMode.OMIT,
            reconstructable_from_lowered_graph=False,
        )


def test_source_workflow_storage_policy_requires_reconstructability_for_optional_mode() -> None:
    with pytest.raises(ValidationError):
        SourceWorkflowStoragePolicy(
            mode=SourceWorkflowStorageMode.OPTIONAL,
            reconstructable_from_lowered_graph=False,
        )


def test_rich_petri_workflow_rejects_arcs_to_unknown_nodes() -> None:
    with pytest.raises(ValidationError):
        RichPetriWorkflow.model_validate(
            {
                "workflow_id": "broken-flow",
                "version": "v1",
                "places": [{"place_id": "place:start", "label": "Start"}],
                "transitions": [
                    {
                        "transition_id": "dispatch",
                        "label": "Dispatch",
                        "capability": "python.fetch",
                        "trace_policy": {"mode": TracePersistenceMode.PER_STEP.value},
                    }
                ],
                "arcs": [
                    {
                        "source_kind": "place",
                        "source_id": "place:start",
                        "target_kind": "transition",
                        "target_id": "missing",
                    }
                ],
            }
        )


def test_rich_petri_workflow_rejects_duplicate_transition_ids() -> None:
    with pytest.raises(ValidationError):
        RichPetriWorkflow.model_validate(
            {
                "workflow_id": "broken-flow",
                "version": "v1",
                "places": [
                    {"place_id": "place:start", "label": "Start"},
                    {"place_id": "place:done", "label": "Done"},
                ],
                "transitions": [
                    {
                        "transition_id": "dispatch",
                        "label": "Dispatch",
                        "capability": "python.fetch",
                    },
                    {
                        "transition_id": "dispatch",
                        "label": "Dispatch again",
                        "capability": "python.transform",
                    },
                ],
                "arcs": [
                    {
                        "source_kind": "place",
                        "source_id": "place:start",
                        "target_kind": "transition",
                        "target_id": "dispatch",
                    },
                    {
                        "source_kind": "transition",
                        "source_id": "dispatch",
                        "target_kind": "place",
                        "target_id": "place:done",
                    },
                ],
            }
        )


def test_rich_petri_transition_allows_approval_hint_without_capability() -> None:
    transition = RichPetriTransition(
        transition_id="approval-only",
        label="Approval only",
        approval_hint={"channel": "human-review"},
    )

    dumped = transition.model_dump(mode="json")

    assert dumped["approval_hint"] == {"channel": "human-review"}
    assert dumped["capability"] is None


def test_rich_petri_transition_rejects_semantically_empty_transition() -> None:
    with pytest.raises(ValidationError):
        RichPetriTransition(
            transition_id="dispatch-unknown",
            label="Dispatch unknown",
        )


def test_rich_petri_transition_rejects_blank_capability() -> None:
    with pytest.raises(ValidationError):
        RichPetriTransition(
            transition_id="dispatch-blank",
            label="Dispatch blank",
            capability="",
        )


def test_rich_petri_transition_rejects_blank_direct_executor_ref() -> None:
    with pytest.raises(ValidationError):
        RichPetriTransition(
            transition_id="dispatch-direct",
            label="Dispatch direct",
            direct_executor_ref="",
            capability="ml.embed",
        )


def test_rich_petri_transition_allows_structural_checkpoint_without_capability() -> None:
    transition = RichPetriTransition(
        transition_id="checkpoint-review",
        label="Checkpoint review",
        checkpoint_hint={"checkpoint_key": "review-ready"},
    )

    dumped = transition.model_dump(mode="json")

    assert dumped["capability"] is None
    assert dumped["checkpoint_hint"] == {"checkpoint_key": "review-ready"}


def test_rich_petri_transition_allows_structural_approval_without_capability() -> None:
    transition = RichPetriTransition(
        transition_id="approval-review",
        label="Approval review",
        approval_hint={"channel": "human-review"},
    )

    dumped = transition.model_dump(mode="json")

    assert dumped["capability"] is None
    assert dumped["approval_hint"] == {"channel": "human-review"}


def test_rich_petri_transition_allows_approval_hint_on_executable_transition() -> None:
    transition = RichPetriTransition(
        transition_id="approval-direct",
        label="Approval direct",
        capability="ml.embed",
        approval_hint={"channel": "human-review"},
    )

    dumped = transition.model_dump(mode="json")

    assert dumped["capability"] == "ml.embed"
    assert dumped["approval_hint"] == {"channel": "human-review"}


def test_structural_transitions_can_carry_both_approval_and_checkpoint_hints() -> None:
    transition = RichPetriTransition(
        transition_id="approval-checkpoint",
        label="Approval checkpoint",
        approval_hint={"channel": "human-review"},
        checkpoint_hint={"checkpoint_key": "review-ready"},
    )

    dumped = transition.model_dump(mode="json")

    assert dumped["approval_hint"] == {"channel": "human-review"}
    assert dumped["checkpoint_hint"] == {"checkpoint_key": "review-ready"}


def test_rich_petri_transition_rejects_operation_ref_field() -> None:
    with pytest.raises(ValidationError):
        RichPetriTransition.model_validate(
            {
                "transition_id": "dispatch-op",
                "label": "Dispatch op",
                "capability": "python.transform",
                "operation_ref": "expr://normalized",
            }
        )


def test_rich_petri_arc_rejects_same_kind_connections() -> None:
    with pytest.raises(ValidationError):
        RichPetriArc(
            source_kind="place",
            source_id="place:start",
            target_kind="place",
            target_id="place:done",
        )
