from __future__ import annotations

from collections import deque
from typing import Any, Literal


def build_lineage_graph(
    cx: Any,
    *,
    artifact_id: str,
    project_id: str,
    depth: int = 3,
    direction: Literal["upstream", "downstream", "both"] = "both",
    max_nodes: int = 10_000,
    max_edges: int = 50_000,
) -> dict:
    """Build a lightweight lineage graph for an artifact.

    This is an explicit BFS over the authoritative `graph_edge_events` log.

    Notes:
    - Nodes are emitted as "artifact" for compatibility; callers may enrich.
    - Edges are derived from `edge_label` (expected format: 'derivation:<type>').
    """

    root_id = str(artifact_id)
    max_depth = max(0, int(depth))

    nodes: dict[str, dict] = {}
    edges: list[dict] = []

    def ensure_node(node_id: str) -> None:
        if node_id in nodes:
            return
        if len(nodes) >= max_nodes:
            raise RuntimeError(f"max_nodes exceeded: {len(nodes)} >= {max_nodes}")
        nodes[node_id] = {
            "id": node_id,
            "type": "artifact",
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
