from __future__ import annotations

from pathlib import Path
import sys

import pytest


ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/workflow/src"))
sys.path.insert(0, str(ROOT / "packages/interacciones/schemas/src"))
sys.path.insert(0, str(ROOT / "packages/ikam/src"))

from interacciones.workflow.preload_workflows import load_compiled_workflows
from interacciones.schemas import RichPetriWorkflow
from modelado.db import connection_scope
from modelado.plans.preload import default_compiled_preseed_dir
from modelado.plans.preload import build_workflow_compilation_fragments
from modelado.plans.preload import preload_fixtures


@pytest.fixture(scope="module")
def db_ready():
    try:
        with connection_scope() as cx:
            cx.execute("SELECT 1")
    except Exception as exc:
        pytest.skip(f"Database not available: {exc}")
    return True


def test_ingestion_workflow_preseed_compiles_through_rich_petri_and_graph_lowering() -> None:
    workflows = load_compiled_workflows()

    assert [workflow.workflow_id for workflow in workflows] == ["ingestion-early-parse"]
    assert isinstance(workflows[0], RichPetriWorkflow)
    assert workflows[0].source_workflow_storage.mode.value == "default_on"
    assert workflows[0].publish[0].registry == "petri_net_runnables"
    assert workflows[0].publish[0].key == "ingestion-early-parse"

    fragments = build_workflow_compilation_fragments()
    profiles = [getattr(fragment, "profile", None) for fragment in fragments]

    assert "rich_petri_workflow" in profiles
    assert "ikam_executable_graph" in profiles
    assert "ikam_graph_derivation" in profiles

    executable_graph = next(
        fragment
        for fragment in fragments
        if getattr(fragment, "profile", None) == "ikam_executable_graph"
        and fragment.data.get("workflow_id") == "ingestion-early-parse"
    )
    load_documents_operator = next(
        fragment
        for fragment in fragments
        if getattr(getattr(fragment, "ast", None), "params", {}).get("transition_id") == "transition:load-documents"
    )
    chunk_operator = next(
        fragment
        for fragment in fragments
        if getattr(getattr(fragment, "ast", None), "params", {}).get("transition_id") == "transition:parse-chunk"
    )
    entity_relationship_operator = next(
        fragment
        for fragment in fragments
        if getattr(getattr(fragment, "ast", None), "params", {}).get("transition_id") == "transition:parse-entities-and-relationships"
    )
    claim_operator = next(
        fragment
        for fragment in fragments
        if getattr(getattr(fragment, "ast", None), "params", {}).get("transition_id") == "transition:parse-claims"
    )

    assert executable_graph.data["source_workflow_storage_mode"] == "default_on"
    assert executable_graph.data["executor_declaration_ids"] == [
        "executor://agent-env-primary",
        "executor://ml-primary",
        "executor://python-primary",
    ]
    assert executable_graph.data["publish"] == [
        {
            "registry": "petri_net_runnables",
            "key": "ingestion-early-parse",
            "title": "Early Ingestion Parse",
            "goal": "Load documents, chunk them, and extract early semantic structure",
        }
    ]
    assert load_documents_operator.ast.params["capability"] == "python.load_documents"
    assert load_documents_operator.ast.params["eligible_executor_ids"] == ["executor://python-primary"]
    assert chunk_operator.ast.params["capability"] == "python.chunk_documents"
    assert chunk_operator.ast.params["eligible_executor_ids"] == ["executor://python-primary"]
    assert entity_relationship_operator.ast.params["capability"] == "ml.extract_entities_relationships"
    assert entity_relationship_operator.ast.params["eligible_executor_ids"] == ["executor://ml-primary"]
    assert claim_operator.ast.params["capability"] == "ml.extract_claims"
    assert claim_operator.ast.params["eligible_executor_ids"] == ["executor://ml-primary"]


def test_preload_fixtures_persists_ingestion_workflow_and_lowered_graph(db_ready) -> None:
    preload_fixtures(default_compiled_preseed_dir())

    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute(
                """
                SELECT value->>'profile' AS profile,
                       cas_id,
                       value->>'fragment_id' AS fragment_id,
                       value->'data'->>'workflow_id' AS workflow_id
                FROM ikam_fragment_store
                WHERE value->>'profile' IN ('rich_petri_workflow', 'ikam_executable_graph')
                  AND value->'data'->>'workflow_id' = 'ingestion-early-parse'
                """
            )
            workflow_rows = cur.fetchall()
            workflow_profiles = {(row["profile"], row["workflow_id"]) for row in workflow_rows}
            fragment_ids_by_profile = {row["profile"]: row["fragment_id"] for row in workflow_rows}
            executable_graph_cas_ids = {
                row["cas_id"]
                for row in workflow_rows
                if row["profile"] == "ikam_executable_graph" and row["cas_id"]
            }

            cur.execute(
                """
                SELECT value->'statement'->>'subject' AS subject,
                       value->'statement'->>'predicate' AS predicate,
                       value->'statement'->>'object' AS object
                FROM ikam_fragment_store
                WHERE value->>'profile' = 'ikam_graph_derivation'
                  AND value->'statement'->>'predicate' = 'lowered_to_executable_graph'
                  AND value->'statement'->>'subject' = %s
                  AND value->'statement'->>'object' = %s
                LIMIT 1
                """
                ,
                (
                    fragment_ids_by_profile["rich_petri_workflow"],
                    fragment_ids_by_profile["ikam_executable_graph"],
                )
            )
            derivation_row = cur.fetchone()

    assert ("rich_petri_workflow", "ingestion-early-parse") in workflow_profiles
    assert ("ikam_executable_graph", "ingestion-early-parse") in workflow_profiles
    assert derivation_row is not None
    assert derivation_row == {
        "subject": fragment_ids_by_profile["rich_petri_workflow"],
        "predicate": "lowered_to_executable_graph",
        "object": fragment_ids_by_profile["ikam_executable_graph"],
    }

    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute(
                """
                SELECT value->>'version' AS version,
                       value->'entries'->'ingestion-early-parse'->>'fragment_id' AS fragment_id,
                       value->'entries'->'ingestion-early-parse'->>'title' AS title,
                       value->'entries'->'ingestion-early-parse'->>'goal' AS goal,
                       value->'entries'->'ingestion-early-parse'->>'head_fragment_id' AS head_fragment_id
                FROM ikam_fragment_store
                WHERE value->>'profile' = 'registry'
                  AND value->>'registry_name' = 'petri_net_runnables'
                ORDER BY created_at DESC
                LIMIT 1
                """
            )
            registry_row = cur.fetchone()

    assert registry_row is not None
    assert int(registry_row["version"]) >= 1
    assert registry_row["fragment_id"] == fragment_ids_by_profile["ikam_executable_graph"]
    assert registry_row["title"] == "Early Ingestion Parse"
    assert registry_row["goal"] == "Load documents, chunk them, and extract early semantic structure"
    assert registry_row["head_fragment_id"] in executable_graph_cas_ids
