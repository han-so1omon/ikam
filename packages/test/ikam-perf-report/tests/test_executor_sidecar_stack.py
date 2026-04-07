import os
from pathlib import Path
import urllib.request
import json

import pytest

from modelado.db import connection_scope
from modelado.environment_scope import EnvironmentScope
from modelado.operators.core import OperatorEnv
from modelado.plans.engine import PetriNetEngine
from modelado.plans.preload import default_preseed_root, preload_fixtures
from modelado.plans.schema import (
    PetriNetArcEndpoint,
    PetriNetEnvelope,
    PetriNetMarking,
    PetriNetTransition,
)


@pytest.fixture(scope="module")
def db_ready():
    try:
        with connection_scope() as cx:
            cx.execute("SELECT 1")
    except Exception as exc:
        pytest.skip(f"Database not available: {exc}")
    return True


@pytest.fixture(scope="module")
def api_container_stack_ready():
    expected_endpoint = "http://ikam-executor-sidecar:8000"
    if os.getenv("IKAM_CASES_ROOT") != "/app/tests/fixtures/cases":
        pytest.skip("This test must run inside the ikam-perf-report-api container")
    if os.getenv("IKAM_EXECUTOR_SIDECAR_URL") != expected_endpoint:
        pytest.skip("Executor sidecar URL is not configured for the API container")

    health = json.loads(
        urllib.request.urlopen(f"{expected_endpoint}/health", timeout=5).read().decode()
    )
    assert health == {"status": "ok"}
    return expected_endpoint


def test_api_container_process_reaches_executor_sidecar(db_ready, api_container_stack_ready):
    expected_endpoint = api_container_stack_ready
    fixtures_dir = default_preseed_root()

    preload_fixtures(fixtures_dir)

    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute(
                """
                SELECT value->>'endpoint' AS endpoint
                FROM ikam_fragment_store
                WHERE value->>'ir_profile' = 'StructuredDataIR'
                  AND value->>'type' = 'sidecar'
                LIMIT 1
                """,
            )
            executor_row = cur.fetchone()

            cur.execute(
                """
                SELECT value->>'_fragment_id' AS frag_id
                FROM ikam_fragment_store
                WHERE value->>'ir_profile' = 'StructuredDataIR'
                  AND value->>'kind' = 'transition'
                  AND value->>'label' = 'Parse Artifacts'
                LIMIT 1
                """,
            )
            transition_row = cur.fetchone()

    assert executor_row is not None, "Could not find sidecar executor fragment in DB"
    assert executor_row["endpoint"] == expected_endpoint
    assert transition_row is not None, "Could not find parse_artifacts fragment in DB"

    transition = PetriNetTransition(
        transition_id="parse_artifacts",
        label="Parse Artifacts",
        operation_ref="parse_artifacts",
        inputs=[PetriNetArcEndpoint(place_id="artifacts_ready", weight=1)],
        outputs=[PetriNetArcEndpoint(place_id="fragments_lifted", weight=1)],
        metadata={"params": {"raw_bytes": "Hello LlamaIndex!"}},
    )
    envelope = PetriNetEnvelope(
        project_id="test",
        scope_id="test",
        title="test",
        goal="test",
        initial_marking_fragment_id="test-marking",
    )
    env = OperatorEnv(
        env_scope=EnvironmentScope(env_id="test_env", env_type="dev"),
        seed=42,
        renderer_version="1.0",
        policy="strict",
        slots={
            "current_marking": PetriNetMarking(
                tokens={"artifacts_ready": 1},
                meta={"current_fragment": {"test_data": "raw"}},
            )
        },
    )
    engine = PetriNetEngine(
        net_envelope=envelope,
        transitions={"parse_artifacts": transition},
        env=env,
        net_artifact_id="test-net-artifact",
    )

    new_marking, firing = engine.fire(
        transition_id="parse_artifacts",
        marking=env.slots["current_marking"],
        transition_fragment_id=transition_row["frag_id"],
    )

    assert firing.status == "success", f"Firing failed: {firing.error}"
    assert new_marking.meta.get("artifacts_loaded") == 1
    assert "result" in firing.effects
    assert "documents" in firing.effects["result"]
    assert len(firing.effects["result"]["documents"]) == 1
