"""Evidence grouping utilities. Contract: section 7.

Groups scored candidates by shared evidence patterns for
downstream subgraph extraction and explanation assembly.
"""

from __future__ import annotations

from typing import Any


def group_by_artifact(
    ranked: list[dict[str, Any]],
    artifact_map: dict[str, str] | None = None,
) -> dict[str, list[dict[str, Any]]]:
    """Group ranked results by artifact membership.

    Args:
        ranked: Output of ``score_candidates``.
        artifact_map: Optional mapping of fragment_id → artifact_id.
            When ``None``, each fragment is its own group.

    Returns:
        Mapping of group key → list of ranked entries.
    """
    groups: dict[str, list[dict[str, Any]]] = {}
    for entry in ranked:
        fid: str = entry["fragment_id"]
        group_key: str = fid
        if artifact_map is not None and fid in artifact_map:
            group_key = artifact_map[fid]
        groups.setdefault(group_key, []).append(entry)
    return groups
