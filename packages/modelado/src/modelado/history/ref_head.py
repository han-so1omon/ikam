from __future__ import annotations


def build_ref_head(*, ref: str, commit_id: str) -> dict[str, str]:
    return {"ref": ref, "commit_id": commit_id}
