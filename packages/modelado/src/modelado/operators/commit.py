from __future__ import annotations

from typing import Any

from modelado.history.commit_entry import build_commit_entry_payload
from modelado.history.ref_head import build_ref_head
from modelado.operators.core import Operator, OperatorEnv, OperatorParams, ProvenanceRecord, record_provenance


class CommitOperator(Operator):
    """Build commit lists from explicit promotion policies."""

    _VALID_POLICIES = {
        "semantic_relations_only",
        "semantic_relations_plus_evidence",
        "full_preservation",
    }

    def apply(self, fragment: Any, params: OperatorParams, env: OperatorEnv) -> dict[str, Any]:
        policy = str(params.parameters.get("commit_policy") or "semantic_relations_only")
        if policy not in self._VALID_POLICIES:
            raise ValueError(f"Unsupported commit_policy: {policy}")

        mapping_mode = str(params.parameters.get("mapping_mode") or "semantic_relations_only")
        include_evidence = bool(params.parameters.get("include_evidence_surface_fragments", False))

        if policy == "semantic_relations_plus_evidence" and not include_evidence:
            raise ValueError("semantic_relations_plus_evidence requires include_evidence_surface_fragments=true")
        if policy == "semantic_relations_only" and include_evidence:
            raise ValueError("semantic_relations_only cannot include evidence; use semantic_relations_plus_evidence")
        if policy == "full_preservation" and mapping_mode != "full_preservation":
            raise ValueError("Invalid mapping_mode for full_preservation commit policy")
        if policy.startswith("semantic_relations") and mapping_mode == "full_preservation":
            raise ValueError("Invalid mapping_mode for semantic commit policy")

        commit_list: list[dict[str, Any]] = []
        commit_list.extend(self._as_entries(params.parameters.get("proposition_fragments"), kind="proposition"))
        commit_list.extend(self._as_entries(params.parameters.get("community_reports"), kind="community_report"))
        commit_list.extend(self._as_entries([params.parameters.get("semantic_subgraph_snapshot")], kind="semantic_subgraph_snapshot"))

        if policy in {"semantic_relations_plus_evidence", "full_preservation"}:
            commit_list.extend(self._as_entries(params.parameters.get("surface_fragments"), kind="surface_fragment"))
        if policy == "full_preservation":
            commit_list.extend(self._as_entries(params.parameters.get("reconstruction_artifacts"), kind="reconstruction_artifact"))

        ref = str(params.parameters.get("ref") or "refs/heads/main")
        target_ref = str(params.parameters.get("target_ref") or ref)
        parents = [str(parent) for parent in (params.parameters.get("parents") or [])]
        promoted_fragment_ids = [
            str(fragment_id)
            for fragment_id in (params.parameters.get("promoted_fragment_ids") or [entry["id"] for entry in commit_list])
        ]
        commit_entry = build_commit_entry_payload(
            ref=ref,
            target_ref=target_ref,
            parents=parents,
            commit_policy=policy,
            commit_list=commit_list,
            promoted_fragment_ids=promoted_fragment_ids,
        )
        ref_head = build_ref_head(ref=ref, commit_id=str(commit_entry["id"]))
        self._append_history(env, commit_entry, ref_head)

        return {
            "status": "committed",
            "target_ref": target_ref,
            "promoted_fragment_ids": promoted_fragment_ids,
            "commit_policy": policy,
            "commit_list": commit_list,
            "commit_count": len(commit_list),
            "commit_entry": commit_entry,
            "ref_head": ref_head,
        }

    def _as_entries(self, value: Any, *, kind: str) -> list[dict[str, Any]]:
        if value is None:
            return []
        items = value if isinstance(value, list) else [value]
        entries: list[dict[str, Any]] = []
        for item in items:
            if item is None:
                continue
            if isinstance(item, dict):
                item_id = str(item.get("id") or item.get("cas_id") or "")
                if not item_id:
                    continue
                entries.append({"id": item_id, "kind": kind, "value": item})
                continue
            item_id = getattr(item, "id", None) or getattr(item, "cas_id", None)
            if not item_id:
                continue
            entries.append({"id": str(item_id), "kind": kind, "value": item})
        return entries

    def _append_history(self, env: OperatorEnv, commit_entry: dict[str, Any], ref_head: dict[str, str]) -> None:
        if not isinstance(env.history, dict):
            return
        entries = env.history.setdefault("commit_entries", [])
        if isinstance(entries, list):
            entries.append(commit_entry)
        ref_heads = env.history.setdefault("ref_heads", {})
        if isinstance(ref_heads, dict):
            ref_heads[ref_head["ref"]] = ref_head

    def provenance(self, params: OperatorParams, env: OperatorEnv) -> ProvenanceRecord:
        return record_provenance(params, env)
