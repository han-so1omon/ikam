import os
import sys

import pytest
from fastapi.testclient import TestClient


def test_startup_fails_when_semantic_engine_unavailable(monkeypatch):
    monkeypatch.setenv("OPENAI_API_KEY", "")
    monkeypatch.setenv("LLM_MODEL", "")
    sys.modules.pop("ikam_perf_report.main", None)
    with pytest.raises((RuntimeError, ValueError)):
        from ikam_perf_report.main import app

        TestClient(app)
