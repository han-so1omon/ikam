from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def _read(rel_path: str) -> str:
    return (ROOT / rel_path).read_text(encoding="utf-8")


def test_no_legacy_openai_client_wrapper_in_runtime_paths() -> None:
    runner_src = _read("ikam_perf_report/benchmarks/runner.py")
    debug_exec_src = _read("../../ikam/src/ikam/forja/debug_execution.py")

    assert "class _OpenAIAIClient" not in runner_src
    assert "_OpenAIAIClient.from_env()" not in runner_src
    assert "_OpenAIAIClient.from_env()" not in debug_exec_src


def test_semantic_pipeline_uses_unified_client_not_sdk_directly() -> None:
    semantic_src = _read("ikam_perf_report/benchmarks/semantic_pipeline.py")
    assert "from openai import AsyncOpenAI" not in semantic_src
    assert "AsyncOpenAI(" not in semantic_src
    assert "create_ai_client_from_env" in semantic_src
