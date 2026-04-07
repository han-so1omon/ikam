from __future__ import annotations

from typing import Any

from fastapi import FastAPI, HTTPException
from pydantic import BaseModel, Field

from mcp_ikam.router import call_tool, list_tools


class ToolCallRequest(BaseModel):
    name: str
    payload: dict[str, Any] = Field(default_factory=dict)


app = FastAPI(title="mcp-ikam")


@app.get("/health")
def health() -> dict[str, str]:
    return {"status": "ok"}


@app.get("/v1/tools")
def tools() -> dict[str, list[str]]:
    return {"tools": list_tools()}


@app.post("/v1/tools/call")
def call(req: ToolCallRequest) -> dict[str, Any]:
    try:
        return {"result": call_tool(req.name, req.payload)}
    except ValueError as exc:
        raise HTTPException(status_code=400, detail=str(exc)) from exc
    except RuntimeError as exc:
        raise HTTPException(status_code=500, detail=str(exc)) from exc
