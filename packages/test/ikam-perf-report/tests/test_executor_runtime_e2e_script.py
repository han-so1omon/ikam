from pathlib import Path


def test_executor_runtime_e2e_script_wires_required_services_and_env() -> None:
    script_path = Path(__file__).resolve().parents[1] / "run_executor_runtime_e2e.sh"

    assert script_path.exists()

    content = script_path.read_text(encoding="utf-8")

    assert "ENABLE_EXECUTOR_RUNTIME_KAFKA_E2E_TESTS=1" in content
    assert "KAFKA_BOOTSTRAP_SERVERS=127.0.0.1:19092" in content
    assert "docker compose -f packages/test/ikam-perf-report/docker-compose.yml up -d --wait --force-recreate redpanda kafka-init ikam-python-executor-runtime ikam-ml-executor-runtime" in content
    assert "uv run pytest packages/test/ikam-perf-report/tests/test_executor_runtime_broker_e2e.py" in content


def test_executor_runtime_e2e_cleanup_script_stops_required_services() -> None:
    script_path = Path(__file__).resolve().parents[1] / "stop_executor_runtime_e2e.sh"

    assert script_path.exists()

    content = script_path.read_text(encoding="utf-8")

    assert "docker compose -f packages/test/ikam-perf-report/docker-compose.yml stop ikam-python-executor-runtime ikam-ml-executor-runtime kafka-init redpanda" in content
    assert "docker compose -f packages/test/ikam-perf-report/docker-compose.yml rm -f ikam-python-executor-runtime ikam-ml-executor-runtime kafka-init redpanda" in content
