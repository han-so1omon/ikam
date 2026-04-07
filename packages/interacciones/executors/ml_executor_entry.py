from __future__ import annotations

import importlib
import os
from typing import Any, Callable

from shell import build_ml_executor_runtime


def run_ml_executor(
    *,
    handlers: dict[str, Callable[[dict[str, Any]], dict[str, Any]]],
    executor_id: str,
    consumer_group_id: str,
    max_iterations: int | None = None,
    should_continue: Callable[[int], bool] | None = None,
    topics: Any = None,
    consumer_factory: Callable[[str], Any] | None = None,
    producer_factory: Callable[[], Any] | None = None,
    build_runtime: Callable[..., Any] = build_ml_executor_runtime,
) -> int:
    runtime = build_runtime(
        executor_id=executor_id,
        handlers=handlers,
        consumer_group_id=consumer_group_id,
        topics=topics,
        consumer_factory=consumer_factory,
        producer_factory=producer_factory,
    )
    return runtime.run_forever(max_iterations=max_iterations, should_continue=should_continue)


def main(
    *,
    env: dict[str, str] | None = None,
    load_handlers: Callable[[str, str], dict[str, Callable[[dict[str, Any]], dict[str, Any]]]] | None = None,
    run_executor: Callable[..., int] = run_ml_executor,
) -> int:
    resolved_env = env or os.environ
    loader = load_handlers or _load_handlers
    handlers = loader(
        resolved_env["IKAM_EXECUTOR_HANDLERS_MODULE"],
        resolved_env.get("IKAM_EXECUTOR_HANDLERS_ATTR", "HANDLERS"),
    )
    raw_max_iterations = resolved_env.get("IKAM_EXECUTOR_MAX_ITERATIONS")
    max_iterations = int(raw_max_iterations) if raw_max_iterations else None
    return run_executor(
        handlers=handlers,
        executor_id=resolved_env.get("IKAM_EXECUTOR_ID", "executor://ml-primary"),
        consumer_group_id=resolved_env.get("IKAM_EXECUTOR_CONSUMER_GROUP_ID", "ml-executor-group"),
        max_iterations=max_iterations,
    )


def _load_handlers(
    module_name: str,
    attr_name: str,
) -> dict[str, Callable[[dict[str, Any]], dict[str, Any]]]:
    module = importlib.import_module(module_name)
    return getattr(module, attr_name)


if __name__ == "__main__":
    raise SystemExit(main())
