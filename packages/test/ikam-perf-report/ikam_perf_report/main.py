import os
from pathlib import Path

from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware

from modelado.oraculo.factory import create_ai_client_from_env
from modelado.intent_classifier import IntentClassifier
from modelado.semantic_engine import SemanticEngine

from ikam_perf_report.benchmarks.case_fixtures import load_registry
from ikam_perf_report.benchmarks.sqi_strategy import register_benchmark_strategies
from ikam_perf_report.api.artifacts import router as artifacts_router
from ikam_perf_report.api.benchmarks import router as benchmarks_router
from ikam_perf_report.api.evaluations import router as evaluations_router
from ikam_perf_report.api.registry import router as registry_router
from ikam_perf_report.api.graph import router as graph_router
from ikam_perf_report.api.history import router as history_router

app = FastAPI()

# Ensure schema exists before any registry operations
from modelado.db import ensure_schema
ensure_schema()

# Register SQI Search Strategies
register_benchmark_strategies()


def _load_local_env_if_available() -> None:
    dotenv_path = Path(__file__).resolve().parent.parent / ".env"
    try:
        from dotenv import load_dotenv  # type: ignore[import-not-found]

        load_dotenv(dotenv_path)
    except Exception:
        if not dotenv_path.exists():
            return
        for raw_line in dotenv_path.read_text(encoding="utf-8").splitlines():
            line = raw_line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, value = line.split("=", 1)
            key = key.strip()
            value = value.strip().strip('"').strip("'")
            if key:
                os.environ.setdefault(key, value)


_load_local_env_if_available()


def _require_semantic_engine() -> None:
    # Hard startup requirement: semantic evaluation infrastructure must be available.
    create_ai_client_from_env()


def _load_llm_model() -> str:
    model = os.getenv("LLM_MODEL", "").strip()
    if not model:
        raise RuntimeError("SemanticEngine required: LLM_MODEL is missing")
    return model


_require_semantic_engine()
load_registry()

llm_model = _load_llm_model()
intent_classifier = IntentClassifier(ai_client=create_ai_client_from_env(), model=llm_model)
semantic_engine = SemanticEngine(intent_classifier=intent_classifier)

app.add_middleware(
    CORSMiddleware,
    allow_origins=[
        "http://localhost:5179",
        "http://127.0.0.1:5179",
        "http://localhost:5180",
        "http://127.0.0.1:5180",
        "http://localhost:5181",
        "http://127.0.0.1:5181",
        "http://ikam-graph-viewer:5179",
    ],
    allow_methods=["*"],
    allow_headers=["*"],
    expose_headers=["Content-Disposition"],
)

app.include_router(graph_router)
app.include_router(benchmarks_router)
app.include_router(artifacts_router)
app.include_router(evaluations_router)
app.include_router(registry_router)
app.include_router(history_router)


@app.get("/health")
def health():
    return {"status": "ok"}
