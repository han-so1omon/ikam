from __future__ import annotations

from pathlib import Path
import sys
import types
from unittest.mock import MagicMock, patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))
sys.path.insert(0, str(ROOT / "packages/ikam/src"))


def test_graph_slice_dispatch_and_delta_apply_flow() -> None:
    from interacciones.schemas import RichPetriArc, RichPetriPlace, RichPetriTransition, RichPetriWorkflow, WorkflowDefinition, WorkflowNode
    from ikam.ir import GRAPH_SLICE_MIME, IKAMGraphSlice
    from modelado.environment_scope import EnvironmentScope
    from modelado.executors import ExecutionDispatcher
    from modelado.executors.bus import ExecutionQueueBus
    from modelado.graph.compiler import GraphCompiler
    from modelado.graph.delta_schema import GRAPH_DELTA_ENVELOPE_SCHEMA_ID
    from modelado.ikam_graph_repository import apply_graph_delta_envelope
    from modelado.operators.core import OperatorEnv

    workflow = RichPetriWorkflow(
        workflow_id="chunk-flow",
        version="v1",
        places=[
            RichPetriPlace(place_id="place:start", label="place:start"),
            RichPetriPlace(place_id="place:end", label="place:end"),
        ],
        transitions=[
            RichPetriTransition(
                transition_id="transition:parse-chunk",
                label="parse-chunk",
                capability="python.chunk_documents",
            )
        ],
        arcs=[
            RichPetriArc(source_kind="place", source_id="place:start", target_kind="transition", target_id="transition:parse-chunk"),
            RichPetriArc(source_kind="transition", source_id="transition:parse-chunk", target_kind="place", target_id="place:end"),
        ],
        source_workflow_definition=WorkflowDefinition(
            workflow_id="chunk-flow",
            version="v1",
            nodes=[
                WorkflowNode(
                    node_id="parse-chunk",
                    kind="dispatch_executor",
                    capability="python.chunk_documents",
                    boundaries={
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
                )
            ],
        ),
    )

    compiled = GraphCompiler().compile(workflow)
    operator = compiled.operators[0]
    transition_fragment_id = operator.fragment_id

    graph_slice = IKAMGraphSlice.model_validate(
        {
            "region": {
                "anchor": {"handle": "document-set", "path": ["documents", 0]},
                "extent": "node",
            },
            "payload": {
                "kind": "document_set",
                "artifact_head_ref": "artifact://case-1",
                "subgraph_ref": "subgraph://run-1-document-set-step-load",
                "document_refs": ["frag-doc-1"],
            },
            "mime_type": GRAPH_SLICE_MIME,
        }
    )

    dispatcher = ExecutionDispatcher()
    queue_bus = ExecutionQueueBus()
    expr_value = {
        "ir_profile": "ExpressionIR",
        "module": "modelado.executors.loaders",
        "entrypoint": "run",
        "ast": {
            "params": operator.ast.params,
        },
    }
    predicate_links = {
        (transition_fragment_id, "executed_by_operator"): transition_fragment_id,
        (transition_fragment_id, "executed_by"): None,
    }

    with patch.object(dispatcher, "get_object_for_predicate", side_effect=lambda subject_id, predicate: predicate_links.get((subject_id, predicate))), patch.object(dispatcher, "get_fragment_value", side_effect=lambda fragment_id: expr_value if fragment_id == transition_fragment_id else None), patch("modelado.executors.compiler.get_execution_context", lambda: None), patch(
        "modelado.executors.compiler.load_executor_declarations",
        lambda: [
            types.SimpleNamespace(
                executor_id="executor://python-primary",
                executor_kind="python-executor",
                capabilities=["python.chunk_documents"],
                transport={"kind": "redpanda", "request_topic": "execution.requests"},
            )
        ],
    ):
        queued = dispatcher.publish_execution_queue_request(
            transition_fragment_id=transition_fragment_id,
            fragment=graph_slice.model_dump(mode="json"),
            params=types.SimpleNamespace(name="dispatch-parse", parameters={"input": "hola"}),
            env=OperatorEnv(
                seed=1,
                renderer_version="test",
                policy="test",
                env_scope=EnvironmentScope(ref="refs/heads/run/env-1"),
            ),
            bus=queue_bus,
        )

    request_topic, request_payload = queue_bus.messages[0]
    queued_topic, queued_payload = queue_bus.messages[1]

    assert request_topic == "execution.requests"
    assert queued_topic == "workflow.events"
    assert queued_payload == queued.model_dump(mode="json")
    assert request_payload["payload"]["context"]["translator_plan"] == {
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
    assert request_payload["payload"]["payload"]["fragment"]["mime_type"] == GRAPH_SLICE_MIME

    delta_envelope = {
        "schema": GRAPH_DELTA_ENVELOPE_SCHEMA_ID,
        "delta": {
            "ops": [
                {
                    "op": "upsert",
                    "anchor": {
                        "handle": request_payload["payload"]["context"]["translator_plan"]["output"][0]["name"].replace("_", "-"),
                    },
                    "value": {
                        "kind": request_payload["payload"]["context"]["translator_plan"]["output"][0]["name"],
                        "source_subgraph_ref": request_payload["payload"]["payload"]["fragment"]["payload"]["subgraph_ref"],
                        "chunk_refs": ["frag-chunk-1"],
                    },
                }
            ]
        },
    }

    cx = MagicMock()
    cx.transaction.return_value.__enter__.return_value = cx
    cx.transaction.return_value.__exit__.return_value = False

    def _append_event(_cx: object, **kwargs: object) -> dict[str, object]:
        properties = kwargs["properties"]
        assert isinstance(properties, dict)
        assert properties["graphDeltaValue"] == {
            "kind": "chunk_extraction_set",
            "source_subgraph_ref": "subgraph://run-1-document-set-step-load",
            "chunk_refs": ["frag-chunk-1"],
        }
        return {
            "id": 1,
            "project_id": "proj-123",
            "op": kwargs["op"],
            "edge_label": kwargs["edge_label"],
            "out_id": kwargs["out_id"],
            "in_id": kwargs["in_id"],
            "properties": properties,
            "t": 1,
            "idempotency_key": kwargs["idempotency_key"],
        }

    with patch("modelado.ikam_graph_repository._require_ikam_write"), patch(
        "modelado.ikam_graph_repository.append_graph_edge_event",
        side_effect=_append_event,
    ):
        apply_result = apply_graph_delta_envelope(
            cx,
            project_id="proj-123",
            envelope=delta_envelope,
        )

    assert apply_result["summary"] == {
        "apply_mode": "atomic",
        "op_count": 1,
        "upsert_count": 1,
        "remove_count": 0,
    }
    assert apply_result["attempted_event_count"] == 1
    assert len(apply_result["appended_events"]) == 1
    assert delta_envelope["delta"]["ops"][0]["value"]["source_subgraph_ref"] == "subgraph://run-1-document-set-step-load"
