from __future__ import annotations


def build_ref_head(*, ref: str, commit_id: str, head_object_id: str | None = None) -> dict[str, str]:
    payload = {"ref": ref, "commit_id": commit_id}
    if head_object_id:
        payload["head_object_id"] = head_object_id
    return payload
