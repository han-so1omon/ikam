from __future__ import annotations

from typing import Any

from ikam.inspection import (
    InspectionEdge,
    InspectionNode,
    InspectionRef,
    InspectionSubgraph,
    ResolveInspectionRequest,
    edge_id_for,
    node_id_for,
)

from modelado.hot_subgraph_store import HotSubgraphStore
from modelado.oraculo.persistent_graph_state import PersistentGraphState


_MEMBER_KEYS = {
    "document_set": ("document_refs", "documents"),
    "chunk_extraction_set": ("extraction_refs", "extractions"),
    "entity_relationship_set": ("entity_relationship_refs", "entity_relationships"),
    "claim_set": ("claim_refs", "claims"),
}


class HotInspectionResolver:
    def __init__(self, store: HotSubgraphStore) -> None:
        self._store = store

    def resolve(self, request: ResolveInspectionRequest) -> InspectionSubgraph:
        payload = self._resolve_payload(request.inspection_ref)
        subgraph_ref = str(payload.get("subgraph_ref") or request.inspection_ref.locator.get("subgraph_ref") or "")
        root_node_id = node_id_for("subgraph", {"subgraph_ref": subgraph_ref})
        nodes = [
            InspectionNode(
                id=root_node_id,
                kind="subgraph",
                ir_kind=str(payload.get("kind") or None),
                label=subgraph_ref or "hot-subgraph",
                payload=payload if request.include_payload else {},
                refs={"self": self._inspection_ref(subgraph_ref, request.inspection_ref)},
                provenance=self._provenance(request.inspection_ref),
            )
        ]
        edges: list[InspectionEdge] = []

        source_subgraph_ref = payload.get("source_subgraph_ref")
        if isinstance(source_subgraph_ref, str) and source_subgraph_ref:
            source_node_id = node_id_for("subgraph", {"subgraph_ref": source_subgraph_ref})
            nodes.append(
                InspectionNode(
                    id=source_node_id,
                    kind="subgraph",
                    label=source_subgraph_ref,
                    refs={"self": self._inspection_ref(source_subgraph_ref)},
                    provenance={"source_backend": "hot", "role": "source_subgraph"},
                )
            )
            if request.include_edges:
                edges.append(
                    InspectionEdge(
                        id=edge_id_for("source_subgraph", root_node_id, source_node_id),
                        **{"from": root_node_id, "to": source_node_id},
                        relation="source_subgraph",
                        provenance={"source_backend": "hot"},
                    )
                )

        member_refs, member_values = _members_for_payload(payload)
        for index, fragment_ref in enumerate(member_refs):
            member_payload = member_values.get(fragment_ref, {"cas_id": fragment_ref})
            node_id = node_id_for("fragment", {"cas_id": fragment_ref})
            nodes.append(
                InspectionNode(
                    id=node_id,
                    kind="fragment",
                    ir_kind=_infer_ir_kind(payload.get("kind"), member_payload),
                    label=_inspection_label(member_payload, fragment_ref),
                    payload=member_payload if request.include_payload else {},
                    refs={
                        "self": InspectionRef(
                            backend="hot",
                            locator={"cas_id": fragment_ref, "position": index},
                        )
                    },
                    provenance=self._provenance(request.inspection_ref),
                )
            )
            if request.include_edges:
                edges.append(
                    InspectionEdge(
                        id=edge_id_for("contains", root_node_id, node_id),
                        **{"from": root_node_id, "to": node_id},
                        relation="contains",
                        provenance={"source_backend": "hot"},
                    )
                )

        explicit_edges = _hot_edges_for_payload(payload)
        for edge_payload in explicit_edges:
            relation = _normalize_relation(edge_payload.get("edge_label"))
            from_node_id = edge_payload.get("from")
            to_node_id = edge_payload.get("to")
            if not isinstance(relation, str) or not isinstance(from_node_id, str) or not isinstance(to_node_id, str):
                continue
            if request.include_edges:
                edges.append(
                    InspectionEdge(
                        id=edge_id_for(relation, from_node_id, to_node_id),
                        **{"from": from_node_id, "to": to_node_id},
                        relation=relation,
                        provenance={"source_backend": "hot"},
                    )
                )
            for node_id in (from_node_id, to_node_id):
                _append_edge_node(
                    nodes,
                    node_id=node_id,
                    payload=payload,
                    member_values=member_values,
                    request=request,
                )

        return InspectionSubgraph(
            schema_version="v1",
            root_node_id=root_node_id,
            nodes=_dedupe_nodes(nodes),
            edges=edges,
            navigation={"focus": {"node_id": root_node_id}},
        )

    def _resolve_payload(self, inspection_ref: InspectionRef) -> dict[str, Any]:
        locator = inspection_ref.locator
        head_fragment_id = locator.get("head_fragment_id")
        if not isinstance(head_fragment_id, str) or not head_fragment_id:
            raise ValueError("hot inspection requires head_fragment_id")
        payload = self._store.get(
            {
                "type": "subgraph_ref",
                "head_fragment_id": head_fragment_id,
            }
        )
        if not isinstance(payload, dict):
            raise ValueError(f"hot inspection payload not found for {head_fragment_id}")
        return payload

    @staticmethod
    def _inspection_ref(subgraph_ref: str, original: InspectionRef | None = None) -> InspectionRef:
        locator = {"subgraph_ref": subgraph_ref}
        if original is not None and isinstance(original.locator.get("head_fragment_id"), str):
            locator["head_fragment_id"] = original.locator["head_fragment_id"]
        return InspectionRef(backend="hot", locator=locator)

    @staticmethod
    def _provenance(inspection_ref: InspectionRef) -> dict[str, Any]:
        provenance = {"source_backend": "hot"}
        head_fragment_id = inspection_ref.locator.get("head_fragment_id")
        if isinstance(head_fragment_id, str) and head_fragment_id:
            provenance["head_fragment_id"] = head_fragment_id
        return provenance


class PersistentInspectionResolver:
    def __init__(self, state: PersistentGraphState) -> None:
        self._state = state

    def resolve(self, request: ResolveInspectionRequest) -> InspectionSubgraph:
        payload = self._resolve_payload(request.inspection_ref)
        subgraph_ref = str(payload.get("subgraph_ref") or request.inspection_ref.locator.get("subgraph_ref") or "")
        root_node_id = node_id_for("subgraph", {"subgraph_ref": subgraph_ref})
        nodes = [
            InspectionNode(
                id=root_node_id,
                kind="subgraph",
                ir_kind=str(payload.get("kind") or None),
                label=subgraph_ref or "persistent-subgraph",
                payload=payload if request.include_payload else {},
                refs={"self": InspectionRef(backend="persistent", locator={"subgraph_ref": subgraph_ref})},
                provenance={"source_backend": "persistent"},
            )
        ]
        edges: list[InspectionEdge] = []

        for edge_payload in _persistent_edges_for_payload(payload):
            target_subgraph_ref = edge_payload.get("to_subgraph_ref")
            target_fragment_id = edge_payload.get("to")
            relation = _normalize_relation(edge_payload.get("edge_label"))
            if relation == "source_subgraph" and isinstance(target_subgraph_ref, str) and target_subgraph_ref:
                source_node_id = node_id_for("subgraph", {"subgraph_ref": target_subgraph_ref})
                nodes.append(
                    InspectionNode(
                        id=source_node_id,
                        kind="subgraph",
                        label=target_subgraph_ref,
                        refs={"self": InspectionRef(backend="persistent", locator={"subgraph_ref": target_subgraph_ref})},
                        provenance={"source_backend": "persistent", "role": "source_subgraph"},
                    )
                )
                if request.include_edges:
                    edges.append(
                        InspectionEdge(
                            id=edge_id_for("source_subgraph", root_node_id, source_node_id),
                            **{"from": root_node_id, "to": source_node_id},
                            relation="source_subgraph",
                            provenance={"source_backend": "persistent"},
                        )
                    )
                continue
            if relation != "contains" or not isinstance(target_fragment_id, str) or not target_fragment_id:
                continue
            fragment = self._state.fragment_by_id(target_fragment_id)
            member_payload = _fragment_payload(fragment, target_fragment_id)
            node_id = node_id_for("fragment", {"cas_id": target_fragment_id})
            nodes.append(
                InspectionNode(
                    id=node_id,
                    kind="fragment",
                    ir_kind=_infer_ir_kind(payload.get("kind"), member_payload),
                    label=target_fragment_id,
                    payload=member_payload if request.include_payload else {},
                    refs={
                        "self": InspectionRef(
                            backend="persistent",
                            locator={"cas_id": target_fragment_id},
                        )
                    },
                    provenance={"source_backend": "persistent"},
                )
            )
            if request.include_edges:
                edges.append(
                    InspectionEdge(
                        id=edge_id_for("contains", root_node_id, node_id),
                        **{"from": root_node_id, "to": node_id},
                        relation="contains",
                        provenance={"source_backend": "persistent"},
                    )
                )

        return InspectionSubgraph(
            schema_version="v1",
            root_node_id=root_node_id,
            nodes=_dedupe_nodes(nodes),
            edges=edges,
            navigation={"focus": {"node_id": root_node_id}},
        )

    def _resolve_payload(self, inspection_ref: InspectionRef) -> dict[str, Any]:
        subgraph_ref = inspection_ref.locator.get("subgraph_ref")
        if not isinstance(subgraph_ref, str) or not subgraph_ref:
            raise ValueError("persistent inspection requires subgraph_ref")
        payload = self._state.inspection_subgraph_by_ref(subgraph_ref)
        if not isinstance(payload, dict):
            raise ValueError(f"persistent inspection payload not found for {subgraph_ref}")
        return payload


def _members_for_payload(payload: dict[str, Any]) -> tuple[list[str], dict[str, dict[str, Any]]]:
    kind = payload.get("kind")
    ref_key, value_key = _MEMBER_KEYS.get(str(kind), ("", ""))
    refs = payload.get(ref_key)
    values = payload.get(value_key)
    member_refs = [str(item) for item in refs if isinstance(item, str)] if isinstance(refs, list) else []
    payloads_by_ref: dict[str, dict[str, Any]] = {}
    if isinstance(values, list):
        for value in values:
            if not isinstance(value, dict):
                continue
            cas_id = value.get("cas_id") or value.get("id")
            if isinstance(cas_id, str) and cas_id:
                payloads_by_ref[cas_id] = value
    if str(kind) == "chunk_extraction_set":
        document_chunk_sets = payload.get("document_chunk_sets")
        if isinstance(document_chunk_sets, list):
            for value in document_chunk_sets:
                if not isinstance(value, dict):
                    continue
                cas_id = value.get("cas_id") or value.get("id")
                if isinstance(cas_id, str) and cas_id:
                    member_refs.append(cas_id)
                    payloads_by_ref[cas_id] = value
    return member_refs, payloads_by_ref


def _infer_ir_kind(parent_kind: Any, payload: dict[str, Any]) -> str | None:
    mime_type = str(payload.get("mime_type") or "").lower()
    value = payload.get("value")
    if "entity-relationship" in mime_type:
        return "entity_relationship"
    if "chunk" in mime_type:
        return "chunk_extraction"
    if "claim" in mime_type:
        return "claim"
    if "loaded-document" in mime_type:
        return "document"
    if mime_type.startswith("text/"):
        return "document"
    if mime_type == "application/json":
        return "json"
    if isinstance(value, dict):
        if any(isinstance(value.get(key), str) and value.get(key) for key in ("name", "filename", "file_name", "document_id")):
            return "document"
        if {"source", "target", "relationship"} <= set(value):
            return "entity_relationship"
        if "claim" in value:
            return "claim"
        if "text" in value and "span" in value:
            return "chunk_extraction"
        return "json"
    fallback = {
        "document_set": "document",
        "chunk_extraction_set": "chunk_extraction",
        "entity_relationship_set": "entity_relationship",
        "claim_set": "claim",
    }
    return fallback.get(str(parent_kind))


def _persistent_edges_for_payload(payload: dict[str, Any]) -> list[dict[str, Any]]:
    raw_edges = payload.get("edges")
    if isinstance(raw_edges, list):
        return [edge for edge in raw_edges if isinstance(edge, dict)]
    member_refs = payload.get("member_refs")
    if isinstance(member_refs, list):
        return [{"to": ref, "edge_label": "knowledge:contains"} for ref in member_refs if isinstance(ref, str)]
    return []


def _hot_edges_for_payload(payload: dict[str, Any]) -> list[dict[str, Any]]:
    raw_edges = payload.get("edges")
    if isinstance(raw_edges, list):
        return [edge for edge in raw_edges if isinstance(edge, dict)]
    return []


def _normalize_relation(value: Any) -> str | None:
    if not isinstance(value, str) or not value:
        return None
    relation = value.split(":", 1)[1] if ":" in value else value
    relation = relation.replace("-", "_")
    if relation in {"contains", "source_subgraph", "references", "related_to", "derives", "anchors", "emits"}:
        return relation
    return None


def _fragment_payload(fragment: Any, fallback_cas_id: str) -> dict[str, Any]:
    if fragment is None:
        return {"cas_id": fallback_cas_id}
    payload = {
        "cas_id": fragment.cas_id or fallback_cas_id,
        "mime_type": fragment.mime_type,
    }
    if fragment.value is not None:
        payload["value"] = fragment.value
    return payload


def _append_edge_node(
    nodes: list[InspectionNode],
    *,
    node_id: str,
    payload: dict[str, Any],
    member_values: dict[str, dict[str, Any]],
    request: ResolveInspectionRequest,
) -> None:
    if not node_id.startswith("fragment:"):
        return
    fragment_ref = node_id.replace("fragment:", "", 1)
    if any(node.id == node_id for node in nodes):
        return
    member_payload = member_values.get(fragment_ref) or _edge_fragment_payload(payload, fragment_ref)
    if member_payload is None:
        return
    nodes.append(
        InspectionNode(
            id=node_id,
            kind="fragment",
            ir_kind=_infer_ir_kind(payload.get("kind"), member_payload),
            label=_inspection_label(member_payload, fragment_ref),
            payload=member_payload if request.include_payload else {},
            refs={"self": InspectionRef(backend="hot", locator={"cas_id": fragment_ref})},
            provenance={"source_backend": "hot"},
        )
    )


def _edge_fragment_payload(payload: dict[str, Any], fragment_ref: str) -> dict[str, Any] | None:
    for key in ("documents", "document_members", "source_documents", "document_chunk_sets"):
        values = payload.get(key)
        if not isinstance(values, list):
            continue
        for value in values:
            if not isinstance(value, dict):
                continue
            cas_id = value.get("cas_id") or value.get("id")
            if isinstance(cas_id, str) and cas_id == fragment_ref:
                return value
    return None


def _inspection_label(payload: dict[str, Any], fallback: str) -> str:
    value = payload.get("value")
    if isinstance(value, dict):
        if _infer_ir_kind(None, payload) == "chunk_extraction":
            for key in ("chunk_id", "document_id", "filename", "file_name"):
                candidate = value.get(key)
                if isinstance(candidate, str) and candidate:
                    return candidate
        for key in ("name", "filename", "file_name", "document_id"):
            candidate = value.get(key)
            if isinstance(candidate, str) and candidate:
                return candidate
    for key in ("name", "filename", "file_name", "document_id", "cas_id", "id"):
        candidate = payload.get(key)
        if isinstance(candidate, str) and candidate:
            return candidate
    return fallback


def _dedupe_nodes(nodes: list[InspectionNode]) -> list[InspectionNode]:
    deduped: dict[str, InspectionNode] = {}
    for node in nodes:
        deduped[node.id] = node
    return list(deduped.values())


__all__ = ["HotInspectionResolver", "PersistentInspectionResolver"]
