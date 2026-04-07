from __future__ import annotations

from datetime import datetime, timezone
from hashlib import sha256
from typing import Any


def _hash_payload(payload: Any) -> str:
    return sha256(repr(payload).encode("utf-8", errors="ignore")).hexdigest()


def _section(title: str, content: str, graph_id: str, run_id: str, model_id: str, harness_id: str, refs: dict[str, list[str]]):
    section_payload = {
        "title": title,
        "content": content,
        "graph_id": graph_id,
        "run_id": run_id,
        "refs": refs,
    }
    return {
        "section_id": f"sec-{_hash_payload(section_payload)[:12]}",
        "title": title,
        "generated_markdown": content,
        "linked_node_ids": refs.get("node_ids", []),
        "linked_edge_ids": refs.get("edge_ids", []),
        "linked_artifact_ids": refs.get("artifact_ids", []),
        "linked_fragment_ids": refs.get("fragment_ids", []),
        "semantic_entity_ids": refs.get("semantic_entity_ids", []),
        "semantic_relation_ids": refs.get("semantic_relation_ids", []),
        "generation_provenance": {
            "model_id": model_id,
            "harness_id": harness_id,
            "prompt_fingerprint": _hash_payload({"title": title, "graph_id": graph_id})[:16],
            "input_snapshot_hash": _hash_payload(refs)[:16],
            "generated_at": datetime.now(timezone.utc).isoformat(),
            "input_graph_id": graph_id,
            "input_run_id": run_id,
        },
    }


def generate_graph_wiki(
    *,
    graph_id: str,
    run_id: str,
    nodes: list[dict[str, Any]],
    edges: list[dict[str, Any]],
    decisions: list[dict[str, Any]],
    semantic: dict[str, Any] | None,
    model_id: str = "openai/gpt-4o-mini",
    harness_id: str = "modelado.wiki_generation.v1",
) -> dict[str, Any]:
    semantic = semantic or {}
    entities = semantic.get("entities", [])
    relations = semantic.get("relations", [])

    node_kinds = sorted({str(node.get("kind") or node.get("type") or "fragment") for node in nodes})
    edge_kinds = sorted({str(edge.get("kind") or edge.get("label") or "composition") for edge in edges})

    dynamic_sections: list[dict[str, Any]] = []
    for kind in node_kinds[:4]:
        kind_nodes = [node for node in nodes if str(node.get("kind") or node.get("type") or "fragment") == kind]
        refs = {
            "node_ids": [str(node.get("id")) for node in kind_nodes[:50] if node.get("id")],
            "edge_ids": [str(edge.get("id")) for edge in edges[:50] if edge.get("id")],
            "artifact_ids": [str((node.get("meta") or {}).get("artifact_id")) for node in kind_nodes[:50] if (node.get("meta") or {}).get("artifact_id")],
            "fragment_ids": [str(node.get("id")) for node in kind_nodes[:50] if node.get("id")],
            "semantic_entity_ids": [str(item.get("id")) for item in entities[:30] if item.get("id")],
            "semantic_relation_ids": [str(item.get("id")) for item in relations[:30] if item.get("id")],
        }
        dynamic_sections.append(
            _section(
                title=f"{kind.title()} Narrative",
                content=(
                    f"This section synthesizes how `{kind}` nodes contribute to graph `{graph_id}`. "
                    f"The graph currently contains {len(kind_nodes)} nodes of this kind and {len(edges)} edges overall. "
                    "Connections are interpreted from decomposition, semantic linking, and merge events observed in the run."
                ),
                graph_id=graph_id,
                run_id=run_id,
                model_id=model_id,
                harness_id=harness_id,
                refs=refs,
            )
        )

    ikam_refs = {
        "node_ids": [str(node.get("id")) for node in nodes[:200] if node.get("id")],
        "edge_ids": [str(edge.get("id")) for edge in edges[:200] if edge.get("id")],
        "artifact_ids": [
            str((node.get("meta") or {}).get("artifact_id"))
            for node in nodes[:200]
            if (node.get("meta") or {}).get("artifact_id")
        ],
        "fragment_ids": [str(node.get("id")) for node in nodes[:200] if node.get("id")],
        "semantic_entity_ids": [str(item.get("id")) for item in entities if item.get("id")],
        "semantic_relation_ids": [str(item.get("id")) for item in relations if item.get("id")],
    }
    ikam_breakdown = _section(
        title="IKAM Breakdown",
        content=(
            f"IKAM breakdown for graph `{graph_id}` links {len(nodes)} nodes and {len(edges)} edges to artifacts and fragments. "
            f"Semantic coverage includes {len(entities)} entities and {len(relations)} relations. "
            f"Connection provenance spans edge kinds: {', '.join(edge_kinds) if edge_kinds else 'none'}. "
            f"Decision trace references {len(decisions)} steps from run `{run_id}`."
        ),
        graph_id=graph_id,
        run_id=run_id,
        model_id=model_id,
        harness_id=harness_id,
        refs=ikam_refs,
    )

    return {
        "graph_id": graph_id,
        "run_id": run_id,
        "sections": dynamic_sections,
        "ikam_breakdown": ikam_breakdown,
        "generated_at": datetime.now(timezone.utc).isoformat(),
    }
