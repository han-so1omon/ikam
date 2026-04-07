from __future__ import annotations

from typing import Any


def _load_artifacts_handler(payload: dict[str, Any]) -> dict[str, Any]:
    from modelado.executors.loaders import run

    result: dict[str, Any] | None = None
    for item in run({"fragment": {}, "params": payload}, {}):
        if item.get("type") == "result":
            result = dict(item.get("result", {}))
            break
    if result is None:
        raise RuntimeError("load_artifacts handler did not return a result")
    return result


PYTHON_EXECUTOR_HANDLERS = {
    "python.load_documents": _load_artifacts_handler,
}


def _ml_embed_handler(payload: dict[str, Any]) -> dict[str, Any]:
    text = str(payload.get("input", ""))
    return {"embedding": [float(len(text)), 1.0]}


ML_EXECUTOR_HANDLERS = {
    "ml.embed": _ml_embed_handler,
}
