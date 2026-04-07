#!/bin/sh
set -eu

docker compose -f packages/test/ikam-perf-report/docker-compose.yml up -d --wait --force-recreate redpanda kafka-init ikam-python-executor-runtime ikam-ml-executor-runtime

ENABLE_EXECUTOR_RUNTIME_KAFKA_E2E_TESTS=1 \
KAFKA_BOOTSTRAP_SERVERS=127.0.0.1:19092 \
uv run pytest packages/test/ikam-perf-report/tests/test_executor_runtime_broker_e2e.py
