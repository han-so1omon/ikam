from __future__ import annotations

import json
import sys
from typing import Any

from mcp_ikam.router import call_tool, list_tools


def _response(id_value: Any, result: Any = None, error: str | None = None, error_code: int = -32000) -> dict[str, Any]:
    payload: dict[str, Any] = {"jsonrpc": "2.0", "id": id_value}
    if error is None:
        payload["result"] = result
    else:
        payload["error"] = {"code": error_code, "message": error}
    return payload


def handle_request(request: dict[str, Any]) -> dict[str, Any]:
    method = request.get("method")
    params = request.get("params") or {}
    id_value = request.get("id")
    if method == "list_tools":
        return _response(id_value, result=list_tools())
    if method == "health":
        return _response(id_value, result={"status": "ok"})
    if method == "call_tool":
        name = params.get("name")
        payload = params.get("payload", {})
        if not isinstance(name, str):
            return _response(id_value, error="call_tool requires params.name", error_code=-32602)
        if not isinstance(payload, dict):
            return _response(id_value, error="call_tool requires params.payload as object", error_code=-32602)
        try:
            return _response(id_value, result=call_tool(name, payload))
        except ValueError as exc:
            return _response(id_value, error=str(exc), error_code=-32602)
        except RuntimeError as exc:
            return _response(id_value, error=str(exc), error_code=-32000)
        except Exception as exc:  # pragma: no cover - transport safety
            return _response(id_value, error=str(exc), error_code=-32000)
    return _response(id_value, error=f"Unknown method: {method}", error_code=-32601)


def main() -> None:
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            sys.stdout.write(json.dumps(_response(None, error="Invalid JSON", error_code=-32700)) + "\n")
            sys.stdout.flush()
            continue
        sys.stdout.write(json.dumps(handle_request(req)) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
