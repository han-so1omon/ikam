from __future__ import annotations

from collections import deque
from typing import Any, Literal

from modelado.environment_scope import EnvironmentScope
from modelado.history.head_locators import resolve_graph_target


def build_lineage_graph(
    cx: Any,
    *,
    artifact_id: str,
    project_id: str,
    env_scope: EnvironmentScope | None = None,
    depth: int = 3,
    direction: Literal["upstream", "downstream", "both"] = "both",
    max_nodes: int = 10_000,
    max_edges: int = 50_000,
) -> dict:
    """Build a lightweight lineage graph for an artifact or fragment target.

    This is an explicit BFS over the authoritative `graph_edge_events` log.

    `artifact_id` may be a raw graph node id, an artifact locator, or a fragment
    locator. Artifact locators resolve through the selected ref head before
    traversal starts.

    Notes:
    - Nodes are emitted as "artifact" for compatibility; callers may enrich.
    - Edges are derived from `edge_label` (expected format: 'derivation:<type>').
    """

    root_id, root_type = _resolve_root_identity(cx, artifact_id=artifact_id, env_scope=env_scope)
    max_depth = max(0, int(depth))

    nodes: dict[str, dict] = {}
    edges: list[dict] = []
    fragment_node_ids: set[str] = {root_id} if root_type == "fragment" else set()

    def ensure_node(node_id: str) -> None:
        if node_id in nodes:
            return
        if len(nodes) >= max_nodes:
            raise RuntimeError(f"max_nodes exceeded: {len(nodes)} >= {max_nodes}")
        node_type = _node_type_for(
            node_id=node_id,
            root_id=root_id,
            root_type=root_type,
            fragment_node_ids=fragment_node_ids,
        )
        nodes[node_id] = {
            "id": node_id,
            "type": node_type,
            "name": node_id,
            "artifactType": None,
            "createdAt": None,
        }

    def latest_outgoing(node_id: str) -> list[dict]:
        return (
            cx.execute(
                """
                SELECT out_id, in_id, edge_label, t
                FROM (
                    SELECT
                        out_id,
                        in_id,
                        edge_label,
                        t,
                        op,
                        row_number() OVER (
                            PARTITION BY out_id, in_id, edge_label
                            ORDER BY id DESC
                        ) AS rn
                    FROM graph_edge_events
                    WHERE project_id = ?
                      AND out_id = ?
                ) s
                WHERE rn = 1
                  AND op != 'delete'
                ORDER BY in_id ASC
                """,
                (project_id, node_id),
            ).fetchall()
            or []
        )

    def latest_incoming(node_id: str) -> list[dict]:
        return (
            cx.execute(
                """
                SELECT out_id, in_id, edge_label, t
                FROM (
                    SELECT
                        out_id,
                        in_id,
                        edge_label,
                        t,
                        op,
                        row_number() OVER (
                            PARTITION BY out_id, in_id, edge_label
                            ORDER BY id DESC
                        ) AS rn
                    FROM graph_edge_events
                    WHERE project_id = ?
                      AND in_id = ?
                ) s
                WHERE rn = 1
                  AND op != 'delete'
                ORDER BY out_id ASC
                """,
                (project_id, node_id),
            ).fetchall()
            or []
        )

    def record_edge(*, source_id: str, target_id: str, edge_label: str, t: int | None) -> None:
        if len(edges) >= max_edges:
            raise RuntimeError(f"max_edges exceeded: {len(edges)} >= {max_edges}")
        derivation_type = edge_label.split(":", 1)[1] if ":" in edge_label else edge_label
        edges.append(
            {
                "sourceId": source_id,
                "targetId": target_id,
                "derivationType": derivation_type,
                "createdAt": int(t) if t is not None else None,
                "metadata": {"edgeLabel": edge_label},
            }
        )

    ensure_node(root_id)

    q: deque[tuple[str, int]] = deque([(root_id, 0)])
    seen: set[str] = {root_id}

    while q:
        node_id, d = q.popleft()
        if d >= max_depth:
            continue

        if direction in {"downstream", "both"}:
            for r in latest_outgoing(node_id):
                out_id = str(r["out_id"]) if isinstance(r, dict) else str(r[0])
                in_id = str(r["in_id"]) if isinstance(r, dict) else str(r[1])
                edge_label = str(r["edge_label"]) if isinstance(r, dict) else str(r[2])
                t_val = int(r["t"]) if isinstance(r, dict) else int(r[3])

                if node_id in fragment_node_ids:
                    fragment_node_ids.add(out_id)
                    fragment_node_ids.add(in_id)

                ensure_node(out_id)
                ensure_node(in_id)
                record_edge(source_id=out_id, target_id=in_id, edge_label=edge_label, t=t_val)

                if in_id not in seen:
                    seen.add(in_id)
                    q.append((in_id, d + 1))

        if direction in {"upstream", "both"}:
            for r in latest_incoming(node_id):
                out_id = str(r["out_id"]) if isinstance(r, dict) else str(r[0])
                in_id = str(r["in_id"]) if isinstance(r, dict) else str(r[1])
                edge_label = str(r["edge_label"]) if isinstance(r, dict) else str(r[2])
                t_val = int(r["t"]) if isinstance(r, dict) else int(r[3])

                if node_id in fragment_node_ids:
                    fragment_node_ids.add(out_id)
                    fragment_node_ids.add(in_id)

                ensure_node(out_id)
                ensure_node(in_id)
                record_edge(source_id=out_id, target_id=in_id, edge_label=edge_label, t=t_val)

                if out_id not in seen:
                    seen.add(out_id)
                    q.append((out_id, d + 1))

    return {
        "rootId": root_id,
        "nodes": list(nodes.values()),
        "edges": edges,
        "depth": max_depth,
    }


def _resolve_root_identity(cx: Any, *, artifact_id: str, env_scope: EnvironmentScope | None) -> tuple[str, str]:
    if artifact_id.startswith(("artifact://", "fragment://", "ref://")):
        target = resolve_graph_target(artifact_id, env_scope=env_scope, cx=cx)
        return target.target_id, target.kind
    return str(artifact_id), "artifact"


def _node_type_for(*, node_id: str, root_id: str, root_type: str, fragment_node_ids: set[str]) -> str:
    if node_id == root_id:
        return root_type
    if node_id in fragment_node_ids:
        return "fragment"
    return "artifact"
