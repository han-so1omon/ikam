from __future__ import annotations

import hashlib
import json
from typing import Any


def build_commit_entry_payload(
    *,
    ref: str,
    target_ref: str,
    parents: list[str],
    commit_policy: str,
    commit_list: list[dict[str, Any]],
    promoted_fragment_ids: list[str],
) -> dict[str, Any]:
    content = {
        "ref": ref,
        "target_ref": target_ref,
        "parents": list(parents),
        "commit_policy": commit_policy,
        "commit_item_ids": [str(item.get("id")) for item in commit_list],
        "promoted_fragment_ids": list(promoted_fragment_ids),
    }
    commit_id = _commit_id(content)
    return {
        "id": commit_id,
        "mime_type": "application/ikam-structured-data+json",
        "profile": "modelado/commit-entry@1",
        "content": {**content, "commit_id": commit_id},
    }


def _commit_id(content: dict[str, Any]) -> str:
    encoded = json.dumps(content, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return f"commit-{hashlib.sha256(encoded).hexdigest()[:16]}"
