from __future__ import annotations

import asyncio
import json
import threading
from dataclasses import dataclass
from typing import Any, Protocol

from ikam.forja.boundary_planner import PlanValidationError, validate_plan_for_text
from ikam.forja.boundary_planner_models import BoundaryPlan, BoundarySpan


class LLMGenerateClient(Protocol):
    async def generate(self, request: Any) -> Any: ...


@dataclass(frozen=True)
class PlannerPromptConfig:
    version: str = "boundary-planner/v1"


class LLMBoundaryPlanner:
    def __init__(self, *, ai_client: LLMGenerateClient, model: str, prompt: PlannerPromptConfig | None = None) -> None:
        self._ai_client = ai_client
        self._model = model
        self._prompt = prompt or PlannerPromptConfig()

    def plan_text(self, *, text: str, mime_type: str, artifact_id: str) -> BoundaryPlan:
        async def _run() -> BoundaryPlan:
            from modelado.oraculo.ai_client import GenerateRequest

            response = await self._ai_client.generate(
                GenerateRequest(
                    model=self._model,
                    temperature=0.0,
                    response_format={"type": "json_object"},
                    messages=[
                        {
                            "role": "system",
                            "content": (
                                "You produce lossless decomposition boundaries. "
                                "Return strict JSON with {spans:[{start,end,label,confidence,reason}]}. "
                                "Spans must be sorted, non-overlapping, and fully cover the input text."
                            ),
                        },
                        {
                            "role": "user",
                            "content": json.dumps(
                                {
                                    "artifact_id": artifact_id,
                                    "mime_type": mime_type,
                                    "text": text,
                                },
                                ensure_ascii=True,
                            ),
                        },
                    ],
                    metadata={
                        "component": "LLMBoundaryPlanner.plan_text",
                        "prompt_version": self._prompt.version,
                    },
                )
            )
            return parse_boundary_plan_text(
                text=response.text,
                provider=response.provider,
                model=response.model,
                prompt_version=self._prompt.version,
                source_text=text,
            )

        try:
            asyncio.get_running_loop()
        except RuntimeError:
            return asyncio.run(_run())

        output: dict[str, BoundaryPlan] = {}
        errors: list[BaseException] = []

        def _worker() -> None:
            try:
                output["plan"] = asyncio.run(_run())
            except BaseException as exc:  # pragma: no cover
                errors.append(exc)

        thread = threading.Thread(target=_worker, daemon=True)
        thread.start()
        thread.join()
        if errors:
            raise errors[0]
        return output["plan"]


class DeterministicBoundaryPlanner:
    def plan_text(self, *, text: str, mime_type: str, artifact_id: str) -> BoundaryPlan:
        del mime_type, artifact_id
        lines = text.splitlines(keepends=True)
        spans: list[BoundarySpan] = []
        if not lines:
            spans = [BoundarySpan(start=0, end=len(text), label="section_1", confidence=1.0, reason="empty")]
        else:
            start = 0
            for index, line in enumerate(lines, start=1):
                end = start + len(line)
                spans.append(BoundarySpan(start=start, end=end, label=f"section_{index}", confidence=0.5, reason="deterministic-test"))
                start = end
            if start < len(text):
                spans.append(BoundarySpan(start=start, end=len(text), label=f"section_{len(spans)+1}", confidence=0.5, reason="deterministic-tail"))
        return BoundaryPlan(spans=spans, provider="deterministic", model="ikam-deterministic-boundary/v1", prompt_version="boundary-planner/v1")


def parse_boundary_plan_payload(*, payload: dict[str, Any], provider: str, model: str, prompt_version: str) -> BoundaryPlan:
    raw_spans = payload.get("spans")
    if not isinstance(raw_spans, list) or not raw_spans:
        raise PlanValidationError("planner payload missing spans")
    spans: list[BoundarySpan] = []
    for item in raw_spans:
        if not isinstance(item, dict):
            raise PlanValidationError("planner span must be an object")
        spans.append(
            BoundarySpan(
                start=int(item.get("start", -1)),
                end=int(item.get("end", -1)),
                label=str(item.get("label") or "section"),
                confidence=float(item.get("confidence", 1.0)),
                reason=str(item.get("reason") or ""),
            )
        )
    return BoundaryPlan(spans=spans, provider=provider, model=model, prompt_version=prompt_version)


def parse_boundary_plan_text(*, text: str, provider: str, model: str, prompt_version: str, source_text: str) -> BoundaryPlan:
    try:
        payload = json.loads(text)
    except json.JSONDecodeError as exc:
        raise PlanValidationError("planner returned invalid JSON") from exc
    if not isinstance(payload, dict):
        raise PlanValidationError("planner payload must be an object")
    plan = parse_boundary_plan_payload(payload=payload, provider=provider, model=model, prompt_version=prompt_version)
    return validate_plan_for_text(text=source_text, plan=plan)
