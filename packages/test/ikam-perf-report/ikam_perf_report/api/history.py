from __future__ import annotations

from fastapi import APIRouter, HTTPException

from ikam_perf_report.benchmarks.store import STORE


router = APIRouter(prefix="/history", tags=["history"])


def _history_for_run(run_id: str) -> dict:
    runtime_context = STORE.get_debug_runtime_context(run_id)
    if runtime_context is None:
        raise HTTPException(status_code=404, detail={"status": "not_found", "run_id": run_id})
    history = runtime_context.get("history")
    if not isinstance(history, dict):
        return {"commit_entries": [], "ref_heads": {}, "commit_items": {}}
    return history


@router.get("/refs")
def list_refs(run_id: str):
    history = _history_for_run(run_id)
    ref_heads = history.get("ref_heads") if isinstance(history.get("ref_heads"), dict) else {}
    refs = [
        {"ref": str(ref), "commit_id": str((head or {}).get("commit_id") or "")}
        for ref, head in sorted(ref_heads.items())
        if isinstance(ref, str)
    ]
    return {"run_id": run_id, "refs": refs}


@router.get("/commits")
def list_commits(run_id: str, ref: str | None = None):
    history = _history_for_run(run_id)
    commits = history.get("commit_entries") if isinstance(history.get("commit_entries"), list) else []
    if ref:
        commits = [
            commit
            for commit in commits
            if isinstance(commit, dict) and isinstance(commit.get("content"), dict) and commit["content"].get("ref") == ref
        ]
    return {"run_id": run_id, "commits": commits}


@router.get("/commits/{commit_id}")
def get_commit(run_id: str, commit_id: str):
    history = _history_for_run(run_id)
    commits = history.get("commit_entries") if isinstance(history.get("commit_entries"), list) else []
    for commit in commits:
        if isinstance(commit, dict) and str(commit.get("id") or "") == commit_id:
            return {"run_id": run_id, "commit": commit}
    raise HTTPException(status_code=404, detail={"status": "not_found", "commit_id": commit_id})


@router.get("/commits/{commit_id}/semantic-graph")
def get_commit_semantic_graph(run_id: str, commit_id: str):
    history = _history_for_run(run_id)
    commit_items = history.get("commit_items") if isinstance(history.get("commit_items"), dict) else {}
    items = commit_items.get(commit_id) if isinstance(commit_items.get(commit_id), list) else []
    nodes = [
        {"id": str(item.get("id") or ""), "kind": str(item.get("kind") or "unknown"), "value": item.get("value")}
        for item in items
        if isinstance(item, dict)
    ]
    return {"run_id": run_id, "commit_id": commit_id, "nodes": nodes, "edges": []}
