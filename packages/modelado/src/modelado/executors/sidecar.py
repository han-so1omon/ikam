from fastapi import FastAPI, HTTPException
from fastapi.responses import StreamingResponse
from pydantic import BaseModel, Field
from typing import Any, Dict, Optional, List
import importlib
import traceback
import json

app = FastAPI(title="IKAM Executor Sidecar")


@app.get("/health")
def health():
    return {"status": "ok"}

class ExecutePayload(BaseModel):
    fragment: Any
    params: Dict[str, Any]

class ExecuteContext(BaseModel):
    tokens: Dict[str, int] = Field(default_factory=dict)
    meta: Dict[str, Any] = Field(default_factory=dict)
    env_scope: Dict[str, Any] = Field(default_factory=dict)

class ExecuteRequest(BaseModel):
    module: str
    entrypoint: str
    payload: ExecutePayload
    context: ExecuteContext

class ContextMutations(BaseModel):
    meta_updates: Dict[str, Any] = Field(default_factory=dict)
    meta_deletes: List[str] = Field(default_factory=list)
    token_deltas: Dict[str, int] = Field(default_factory=dict)

class ExecuteResponse(BaseModel):
    status: str = "success"
    result: Any = None
    context_mutations: Optional[ContextMutations] = None
    error: Optional[str] = None

@app.post("/execute")
async def execute(request: ExecuteRequest):
    try:
        mod = importlib.import_module(request.module)
        func = getattr(mod, request.entrypoint)
        
        def generate():
            try:
                # Pass payload and context to the entrypoint
                result_or_gen = func(request.payload.model_dump(), request.context.model_dump())
                
                if hasattr(result_or_gen, "__iter__") and not isinstance(result_or_gen, dict):
                    for chunk in result_or_gen:
                        yield json.dumps(chunk) + "\n"
                else:
                    # Expecting the function to return a dict matching ExecuteResponse if it wants
                    # to send context_mutations, or just the raw result
                    if isinstance(result_or_gen, dict) and ("result" in result_or_gen or "context_mutations" in result_or_gen):
                        yield json.dumps({"type": "result", **result_or_gen}) + "\n"
                    else:
                        yield json.dumps({"type": "result", "status": "success", "result": result_or_gen}) + "\n"
            except Exception as e:
                traceback.print_exc()
                yield json.dumps({"type": "result", "status": "failed", "error": str(e)}) + "\n"

        return StreamingResponse(generate(), media_type="application/x-ndjson")

    except ImportError as e:
        raise HTTPException(status_code=400, detail=f"Module not found: {e}")
    except AttributeError as e:
        raise HTTPException(status_code=400, detail=f"Entrypoint not found: {e}")
    except Exception as e:
        traceback.print_exc()
        raise HTTPException(status_code=500, detail=f"Execution failed: {str(e)}")

if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="0.0.0.0", port=8000)
