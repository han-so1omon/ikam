from pathlib import Path

import pytest
import yaml


def _load_compose() -> dict:
    repo_root = Path(__file__).resolve().parents[4]
    compose_path = repo_root / "packages" / "test" / "ikam-perf-report" / "docker-compose.yml"
    if not compose_path.exists():
        pytest.skip("compose file not mounted in this test environment")
    return yaml.safe_load(compose_path.read_text())


def test_ikam_perf_report_profile_exists():
    compose = _load_compose()
    services = compose.get("services", {})
    assert "ikam-perf-report-api" in services
    assert "instruction-parser" not in services


def test_perf_report_stack_wires_executor_sidecar():
    compose = _load_compose()
    services = compose.get("services", {})

    assert "ikam-executor-sidecar" in services

    api_service = services["ikam-perf-report-api"]
    api_environment = api_service.get("environment", [])
    api_command = api_service.get("command", "")
    assert "IKAM_EXECUTOR_SIDECAR_URL=http://ikam-executor-sidecar:8000" in api_environment
    assert "pip install -e '/app/packages/interacciones[kafka]'" in api_command
    assert "PYTHONPATH=/app/packages/interacciones/schemas/src:/app/packages/interacciones/graph/src:/app/packages/interacciones/workflow/src" in api_command

    depends_on = api_service.get("depends_on", {})
    assert depends_on.get("ikam-executor-sidecar", {}).get("condition") == "service_healthy"


def test_perf_report_stack_bootstraps_schema_and_preseed_via_init_service():
    compose = _load_compose()
    services = compose.get("services", {})

    assert "ikam-perf-report-init" in services

    init_service = services["ikam-perf-report-init"]
    init_command = init_service.get("command", "")
    init_depends_on = init_service.get("depends_on", {})

    assert "pip install -e '/app/packages/ikam[chunking]'" in init_command
    assert "pip install -e '/app/packages/interacciones[kafka]'" in init_command
    assert "pip install -e '/app/packages/modelado[documents,postgres]'" in init_command
    assert "pip install -e /app/packages/test/ikam-perf-report" in init_command
    assert "PYTHONPATH=/app/packages/ikam/src:/app/packages/modelado/src:/app/packages/interacciones/schemas/src:/app/packages/interacciones/graph/src:/app/packages/interacciones/workflow/src:/app/packages/test/ikam-perf-report" in init_command
    assert "ensure_schema" in init_command
    assert "preload_fixtures(default_preseed_root())" in init_command
    assert init_depends_on.get("ikam-perf-report-postgres", {}).get("condition") == "service_healthy"

    api_depends_on = services["ikam-perf-report-api"].get("depends_on", {})
    assert api_depends_on.get("ikam-perf-report-init", {}).get("condition") == "service_completed_successfully"


def test_perf_report_stack_wires_parallel_python_executor_runtime():
    compose = _load_compose()
    services = compose.get("services", {})

    assert "ikam-python-executor-runtime" in services

    runtime_service = services["ikam-python-executor-runtime"]
    command = runtime_service.get("command", "")
    environment = runtime_service.get("environment", [])

    assert "python3 -m python_executor_entry" in command
    assert "pip install -e '/app/packages/interacciones[kafka]'" in command
    assert "PYTHONPATH=/app/packages/interacciones/executors:/app/packages/interacciones/schemas/src:/app/packages/interacciones/graph/src:/app/packages/test/ikam-perf-report" in command
    assert "IKAM_EXECUTOR_HANDLERS_MODULE=ikam_perf_report.executor_handlers" in environment
    assert "IKAM_EXECUTOR_HANDLERS_ATTR=PYTHON_EXECUTOR_HANDLERS" in environment
    assert "IKAM_EXECUTOR_ID=executor://python-primary" in environment
    assert "IKAM_EXECUTOR_CONSUMER_GROUP_ID=python-executor-group" in environment


def test_perf_report_stack_wires_parallel_ml_executor_runtime():
    compose = _load_compose()
    services = compose.get("services", {})

    assert "ikam-ml-executor-runtime" in services

    runtime_service = services["ikam-ml-executor-runtime"]
    command = runtime_service.get("command", "")
    environment = runtime_service.get("environment", [])

    assert "python3 -m ml_executor_entry" in command
    assert "pip install -e '/app/packages/interacciones[kafka]'" in command
    assert "PYTHONPATH=/app/packages/interacciones/executors:/app/packages/interacciones/schemas/src:/app/packages/interacciones/graph/src:/app/packages/test/ikam-perf-report" in command
    assert "IKAM_EXECUTOR_HANDLERS_MODULE=ikam_perf_report.executor_handlers" in environment
    assert "IKAM_EXECUTOR_HANDLERS_ATTR=ML_EXECUTOR_HANDLERS" in environment
    assert "IKAM_EXECUTOR_ID=executor://ml-primary" in environment
    assert "IKAM_EXECUTOR_CONSUMER_GROUP_ID=ml-executor-group" in environment


def test_perf_report_stack_wires_broker_for_parallel_executor_runtimes():
    compose = _load_compose()
    services = compose.get("services", {})

    assert "redpanda" in services

    redpanda_service = services["redpanda"]
    redpanda_command = " ".join(redpanda_service.get("command", []))
    assert "redpanda start" in redpanda_command
    assert "--kafka-addr=PLAINTEXT://0.0.0.0:9092,OUTSIDE://0.0.0.0:19092" in redpanda_command
    assert "--advertise-kafka-addr=PLAINTEXT://redpanda:9092,OUTSIDE://127.0.0.1:19092" in redpanda_command

    for service_name in ("ikam-python-executor-runtime", "ikam-ml-executor-runtime"):
        runtime_service = services[service_name]
        environment = runtime_service.get("environment", [])
        depends_on = runtime_service.get("depends_on", {})

        assert "KAFKA_BOOTSTRAP_SERVERS=redpanda:9092" in environment
        assert depends_on.get("redpanda", {}).get("condition") == "service_healthy"


def test_perf_report_stack_adds_healthchecks_for_parallel_executor_runtimes():
    compose = _load_compose()
    services = compose.get("services", {})

    for service_name, module_name in (
        ("ikam-python-executor-runtime", "python_executor_entry"),
        ("ikam-ml-executor-runtime", "ml_executor_entry"),
    ):
        healthcheck = services[service_name].get("healthcheck", {})
        test_cmd = healthcheck.get("test", [])
        assert test_cmd[0] == "CMD-SHELL"
        assert "PYTHONPATH=/app/packages/interacciones/executors:/app/packages/interacciones/schemas/src:/app/packages/interacciones/graph/src:/app/packages/test/ikam-perf-report" in test_cmd[1]
        assert "import importlib; importlib.import_module" in test_cmd[1]
        assert module_name in test_cmd[1]


def test_perf_report_stack_initializes_executor_runtime_topics():
    compose = _load_compose()
    services = compose.get("services", {})

    assert "kafka-init" in services

    kafka_init = services["kafka-init"]
    command = kafka_init.get("command", "")
    command_text = " ".join(command) if isinstance(command, list) else command
    depends_on = kafka_init.get("depends_on", {})

    assert "execution.requests" in command_text
    assert "workflow.events" in command_text
    assert "execution.progress" in command_text
    assert "execution.results" in command_text
    assert depends_on.get("redpanda", {}).get("condition") == "service_healthy"

    for service_name in ("ikam-python-executor-runtime", "ikam-ml-executor-runtime"):
        service_depends_on = services[service_name].get("depends_on", {})
        assert service_depends_on.get("kafka-init", {}).get("condition") == "service_completed_successfully"
