from __future__ import annotations

import json
from urllib import request as urllib_request
from urllib.error import HTTPError, URLError
from typing import Any, Callable


class MCPIkamClient:
    def __init__(self, base_url: str, timeout_seconds: float = 30.0) -> None:
        if not isinstance(base_url, str) or not base_url.strip():
            raise ValueError("mcp-ikam base_url is required")
        self.base_url = base_url.rstrip("/")
        self.timeout_seconds = timeout_seconds

    def generate_structural_map(
        self,
        payload: dict[str, Any],
        on_trace_event: Callable[[dict[str, Any]], None] | None = None,
    ) -> dict[str, Any]:
        body = json.dumps({"name": "generate_structural_map", "payload": payload}).encode("utf-8")
        req = urllib_request.Request(
            f"{self.base_url}/v1/tools/call",
            data=body,
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib_request.urlopen(req, timeout=self.timeout_seconds) as resp:
                payload_json = json.loads(resp.read().decode("utf-8"))
        except TimeoutError as exc:
            raise RuntimeError(f"mcp-ikam timeout after {self.timeout_seconds:.0f}s") from exc
        except HTTPError as exc:
            detail = ""
            try:
                body = exc.read().decode("utf-8", errors="replace").strip()
                if body:
                    detail = body
            except Exception:
                detail = ""
            if detail:
                raise RuntimeError(f"mcp-ikam http error: {exc.code} ({detail})") from exc
            raise RuntimeError(f"mcp-ikam http error: {exc.code}") from exc
        except URLError as exc:
            reason = str(exc.reason)
            if "timed out" in reason.lower():
                raise RuntimeError(f"mcp-ikam timeout after {self.timeout_seconds:.0f}s") from exc
            raise RuntimeError(f"mcp-ikam transport error: {reason}") from exc
        except json.JSONDecodeError as exc:
            raise RuntimeError("mcp-ikam response invalid json") from exc
        result = payload_json.get("result")
        if not isinstance(result, dict):
            raise RuntimeError("mcp-ikam response missing result object")
        trace_events = result.get("trace_events")
        if callable(on_trace_event) and isinstance(trace_events, list):
            for item in trace_events:
                if isinstance(item, dict):
                    on_trace_event(item)
        return result
