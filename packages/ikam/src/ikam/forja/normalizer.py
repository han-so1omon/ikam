import json
import re
from typing import List, Any, Protocol
from ikam.fragments import Fragment, CONCEPT_MIME


class LLMResult(Protocol):
    output: str


class AIClient(Protocol):
    async def call_model(self, prompt: str, model: str, temperature: float) -> LLMResult: ...


class NormalizationError(Exception):
    """Raised when normalization fails (LLM error, malformed response)."""


class SemanticNormalizer:
    def __init__(self, ai_client: AIClient):
        self.ai_client = ai_client

    async def normalize(
        self,
        fragment: Fragment,
        *,
        mode: str = "explore-fast",
        policy_version: str = "2026-02-10",
    ) -> List[Fragment]:
        """Normalize a source fragment into concept fragments."""
        content = ""
        if fragment.value and isinstance(fragment.value, dict) and "text" in fragment.value:
            content = fragment.value["text"]
        elif fragment.value and isinstance(fragment.value, str):
            content = fragment.value

        if not content:
            return []

        prompt = f"Extract atomic concepts from: {content}. Return JSON list of strings."
        temperature = 0.0 if mode.startswith("commit-") else 0.2

        try:
            result = await self.ai_client.call_model(
                prompt=prompt, model="gpt-4o-mini", temperature=temperature,
            )
            response_text = result.output
            cleaned_text = response_text.strip()
            if cleaned_text.startswith("```json"):
                cleaned_text = cleaned_text[7:]
            if cleaned_text.endswith("```"):
                cleaned_text = cleaned_text[:-3]
            cleaned_text = cleaned_text.strip()
            concepts = json.loads(cleaned_text)
            if not isinstance(concepts, list):
                concepts = [str(concepts)]
        except json.JSONDecodeError as exc:
            raise NormalizationError(f"LLM returned malformed JSON: {exc}") from exc
        except Exception as exc:
            raise NormalizationError(f"LLM normalization failed: {exc}") from exc

        cleaned: list[str] = []
        for concept in concepts:
            if not isinstance(concept, str):
                continue
            value = " ".join(concept.strip().split())
            if not value:
                continue
            cleaned.append(value)

        if mode.startswith("commit-"):
            deterministic = sorted({token.lower() for token in re.findall(r"\b[a-zA-Z][a-zA-Z0-9_]{3,}\b", content)})
            cleaned = deterministic or sorted(set(item.lower() for item in cleaned))

        fragments = []
        for concept in cleaned:
            frag = Fragment(
                value={"concept": concept, "mode": mode, "policy_version": policy_version},
                mime_type=CONCEPT_MIME,
            )
            fragments.append(frag)

        return fragments
