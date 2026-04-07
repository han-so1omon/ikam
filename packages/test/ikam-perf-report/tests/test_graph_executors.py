import os
import time
import json
import socket
import pytest
import threading
import subprocess
import urllib.request
import urllib.error

from modelado.db import connection_scope
from modelado.executors.compiler import ExecutionDispatcher
from modelado.plans.preload import default_preseed_root, preload_fixtures
from modelado.plans.engine import PetriNetEngine
from modelado.plans.schema import PetriNetMarking, PetriNetEnvelope, PetriNetTransition, PetriNetArcEndpoint
from modelado.operators.core import OperatorEnv
from modelado.environment_scope import EnvironmentScope
from modelado.registry.graph_registry import GraphRegistry

@pytest.fixture(scope="module")
def db_ready():
    """Ensure database connection is available before running tests."""
    # This just ensures we don't crash immediately if the DB is down,
    # or it acts as a gatekeeper.
    try:
        with connection_scope() as cx:
            cx.execute("SELECT 1")
    except Exception as e:
        pytest.skip(f"Database not available: {e}")
    return True

@pytest.fixture(scope="module")
def sidecar_server():
    """Runs the Executor Sidecar in a background subprocess."""
    import sys
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]

    proc = subprocess.Popen(
        [sys.executable, "-m", "uvicorn", "modelado.executors.sidecar:app", "--host", "127.0.0.1", "--port", str(port)]
    )
    
    # Wait for it to be ready
    ready = False
    for _ in range(10):
        try:
            response = urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=5)
            if response.status == 200:
                ready = True
                break
        except urllib.error.URLError:
            pass
        time.sleep(0.5)

    if not ready:
        proc.terminate()
        proc.wait()
        raise RuntimeError("Executor sidecar test server failed to become healthy")
         
    yield f"http://127.0.0.1:{port}"
    
    proc.terminate()
    proc.wait()

def test_graph_executor_e2e(db_ready, sidecar_server, monkeypatch):
    """
    E2E test for the graph-compiled Executor sidecar.
    
    1. Preloads the fixtures.
    2. Overrides the executor registry endpoint to the test sidecar server port.
    3. Loads the PetriNet via graph registries.
    4. Evaluates a transition on the PetriNet Engine.
    """
    pytest.importorskip("llama_index.core")

    # 1. Run Preload (inserts the IR fragments into DB)
    compiled_dir = default_preseed_root()
    if not compiled_dir.exists():
        pytest.skip("Consolidated preseed root does not exist.")

    preload_fixtures(compiled_dir)
    
    # Override executor endpoints to point to our sidecar_server instance
    # The DB will have an executor with endpoint 'http://localhost:8000', we change it to 8001
    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute(
                """
                UPDATE ikam_fragment_store 
                SET value = jsonb_set(value::jsonb, '{endpoint}', %s::jsonb)
                WHERE value->>'ir_profile' = 'StructuredDataIR' 
                  AND value->>'type' = 'sidecar'
                """,
                (json.dumps(sidecar_server),)
            )
            cx.commit()

    # 2. Extract transitions directly (simulation of get_petri_net)
    transitions = {
        "parse_artifacts": PetriNetTransition(
            transition_id="parse_artifacts",
            label="Parse Artifacts",
            operation_ref="parse_artifacts",
            inputs=[PetriNetArcEndpoint(place_id="artifacts_ready", weight=1)],
            outputs=[PetriNetArcEndpoint(place_id="fragments_lifted", weight=1)],
            metadata={"params": {"raw_bytes": "Hello LlamaIndex!"}}
        )
    }

    envelope = PetriNetEnvelope(
        project_id="test",
        scope_id="test",
        title="test",
        goal="test",
        initial_marking_fragment_id="test-marking"
    )
    
    # 3. Setup the Execution Engine
    env_scope = EnvironmentScope(
        env_id="test_env",
        env_type="dev"
    )
    
    env = OperatorEnv(
        env_scope=env_scope,
        seed=42,
        renderer_version="1.0",
        policy="strict",
        slots={"current_marking": PetriNetMarking(tokens={"artifacts_ready": 1}, meta={"current_fragment": {"test_data": "raw"}})}
    )
    
    engine = PetriNetEngine(
        net_envelope=envelope,
        transitions=transitions,
        env=env,
        net_artifact_id="test-net-artifact"
    )

    # We need to find the transition_fragment_id for "parse_artifacts"
    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute(
                """
                SELECT value->>'_fragment_id' as frag_id FROM ikam_fragment_store 
                WHERE value->>'ir_profile' = 'StructuredDataIR' 
                  AND value->>'kind' = 'transition'
                  AND value->>'label' = 'Parse Artifacts'
                LIMIT 1
                """
            )
            row = cur.fetchone()
            assert row is not None, "Could not find parse_artifacts fragment in DB"
            transition_fragment_id = row["frag_id"]

    # 4. Fire the transition
    initial_marking = env.slots["current_marking"]
    
    new_marking, firing = engine.fire(
        transition_id="parse_artifacts",
        marking=initial_marking,
        transition_fragment_id=transition_fragment_id
    )

    # 5. Assertions
    assert firing.status == "success", f"Firing failed: {firing.error}"
    assert new_marking.tokens.get("artifacts_ready") is None, "Token should be consumed"
    assert new_marking.tokens.get("fragments_lifted") == 1, "Token should be produced"
    
    assert new_marking.meta.get("artifacts_loaded") == 1, "Context mutations should be applied"
    assert "result" in firing.effects
    assert "documents" in firing.effects["result"]
    assert len(firing.effects["result"]["documents"]) == 1
    assert "Hello LlamaIndex!" in firing.effects["result"]["documents"][0]["text"]


def test_dispatcher_uses_env_driven_sidecar_endpoint(db_ready, monkeypatch):
    monkeypatch.setenv(
        "IKAM_EXECUTOR_SIDECAR_URL",
        "http://127.0.0.1:9999",
    )

    fixtures_dir = default_preseed_root()
    preload_fixtures(fixtures_dir)

    with connection_scope() as cx:
        with cx.cursor() as cur:
            cur.execute(
                """
                SELECT value->>'_fragment_id' AS fragment_id
                FROM ikam_fragment_store
                WHERE value->>'ir_profile' = 'StructuredDataIR'
                  AND value->>'type' = 'sidecar'
                LIMIT 1
                """
            )
            row = cur.fetchone()

    assert row is not None
    value = ExecutionDispatcher().get_fragment_value(row["fragment_id"])

    assert value is not None
    assert value["endpoint"] == "http://127.0.0.1:9999"


def test_executor_sidecar_health_endpoint(sidecar_server):
    response = urllib.request.urlopen(f"{sidecar_server}/health", timeout=5)

    assert response.status == 200
    assert json.loads(response.read().decode()) == {"status": "ok"}
