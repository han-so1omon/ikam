from __future__ import annotations

import json

from modelado.oraculo.llm_trace import emit_llm_trace


def test_emit_llm_trace_writes_jsonl_record(tmp_path, monkeypatch):
    path = tmp_path / "llm-trace.jsonl"
    monkeypatch.setenv("IKAM_LLM_TRACE_FILE", str(path))
    monkeypatch.setenv("IKAM_TRACE_THREAD_ID", "thread-1")
    monkeypatch.setenv("IKAM_TRACE_ARTIFACT_ID", "artifact-1")

    emit_llm_trace(
        provider="openai",
        operation="chat.completions",
        model="gpt-4o-mini",
        phase="request",
        request_payload={"prompt": "hello"},
        metadata={"component": "unit-test"},
    )

    lines = path.read_text(encoding="utf-8").strip().splitlines()
    assert len(lines) == 1
    record = json.loads(lines[0])
    assert record["provider"] == "openai"
    assert record["operation"] == "chat.completions"
    assert record["model"] == "gpt-4o-mini"
    assert record["phase"] == "request"
    assert record["thread_id"] == "thread-1"
    assert record["artifact_id"] == "artifact-1"
