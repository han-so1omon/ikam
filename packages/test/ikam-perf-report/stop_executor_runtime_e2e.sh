#!/bin/sh
set -eu

docker compose -f packages/test/ikam-perf-report/docker-compose.yml stop ikam-python-executor-runtime ikam-ml-executor-runtime kafka-init redpanda
docker compose -f packages/test/ikam-perf-report/docker-compose.yml rm -f ikam-python-executor-runtime ikam-ml-executor-runtime kafka-init redpanda
