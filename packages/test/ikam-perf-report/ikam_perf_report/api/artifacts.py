"""Artifact download endpoint.

Reconstructs artifact content from V3 fragments stored in GraphSnapshot
using IKAM algebra (root relation DAG traversal).

Flow:
1. Find artifact node in any run's graph by artifact_id
2. Collect child fragment cas_ids via artifact-root edges
3. Filter stored V3 fragments to those belonging to this artifact
4. Reconstruct via root relation DAG traversal
5. Stream the result with Content-Disposition
"""

from __future__ import annotations

from fastapi import APIRouter, HTTPException
from fastapi.responses import Response

from ikam.fragments import Fragment, RELATION_MIME, is_relation_fragment, Relation
from ikam.forja.reconstructor import reconstruct_document, reconstruct_binary

from ikam_perf_report.benchmarks.store import STORE

router = APIRouter(prefix="/artifacts", tags=["artifacts"])


def _find_artifact(artifact_id: str):
    """Find artifact node and its parent graph across all runs.

    Returns (graph_snapshot, artifact_node) or (None, None).
    """
    for run in STORE.list_runs():
        graph = run.graph
        for node in graph.nodes:
            if node.get("id") == artifact_id and node.get("type") == "artifact":
                return graph, node
    return None, None


def _collect_artifact_fragments(graph, artifact_id: str) -> list[Fragment]:
    """Collect V3 fragments belonging to a specific artifact.

    Uses artifact-root edges to find content fragment cas_ids,
    then finds the matching relation fragment whose binding groups
    reference those cas_ids.
    """
    # Step 1: Get content cas_ids from artifact-root edges
    content_cas_ids: set[str] = set()
    for edge in graph.edges:
        if edge.get("source") == artifact_id:
            # handle naive graph
            if edge.get("kind") == "artifact-root":
                content_cas_ids.add(edge["target"])
            # handle mapped graph
            elif edge.get("predicate") == "contains":
                content_cas_ids.add(edge["target"])
                
    if not content_cas_ids:
        # Fallback: some graphs use the artifact_id prefix in the fragment ID or just try to find fragments matching
        # the artifact name if no edges match exactly.
        for frag in graph.fragments:
            if isinstance(frag, dict):
                continue
            frag_id = getattr(frag, "cas_id", None) or getattr(frag, "id", None)
            if frag_id and (artifact_id in frag_id or frag_id in artifact_id):
                content_cas_ids.add(frag_id)

    if not content_cas_ids:
        print(f"DEBUG: No artifact-root edges found for artifact_id {artifact_id}. Edges: {[e for e in graph.edges if e.get('source') == artifact_id]}")
        return []

    # Map nodes might be wrappers around CAS IDs. Try to resolve map:artifact:... -> actual CAS ID.
    resolved_cas_ids = set()
    for cid in content_cas_ids:
        if cid.startswith("map:"):
            # find original node to get its fragment id
            for node in graph.nodes:
                if node.get("id") == cid and "fragment_id" in node:
                    resolved_cas_ids.add(node["fragment_id"])
            # if we couldn't resolve, maybe it just strips the prefix?
        else:
            resolved_cas_ids.add(cid)
    
    # If we couldn't resolve from map, let's just collect ALL fragments that are not relations
    # if there is only 1 artifact anyway... no, this is risky.
    if not resolved_cas_ids:
        resolved_cas_ids = content_cas_ids

    # Step 2: Partition fragments into content and relation
    content_fragments: list[Fragment] = []
    relation_fragment: Fragment | None = None

    for frag in graph.fragments:
        if isinstance(frag, dict):
            # Debug-scoped payload copies are not reconstructable Fragment objects.
            continue
        frag_id = getattr(frag, "cas_id", None) or getattr(frag, "id", None)

        if is_relation_fragment(frag):
            # Check if this relation references our content fragments
            value = frag.value
            if isinstance(value, dict):
                try:
                    rel = Relation.model_validate(value)
                    for bg in rel.binding_groups:
                        for sb in bg.slots:
                            if sb.fragment_id in resolved_cas_ids or sb.fragment_id in content_cas_ids:
                                relation_fragment = frag
                                break
                        if relation_fragment:
                            break
                except Exception:
                    pass
        elif frag_id in resolved_cas_ids or frag_id in content_cas_ids:
            content_fragments.append(frag)

    if not content_fragments:
        print(f"DEBUG: Failed to map content_cas_ids {content_cas_ids} or resolved {resolved_cas_ids} to fragments! Fragments available: {[getattr(f, 'cas_id', getattr(f, 'id', None)) for f in graph.fragments if not isinstance(f, dict)]}")

    if not relation_fragment:
        return content_fragments

    return content_fragments + [relation_fragment]


def _is_text_artifact(node: dict, fragments: list[Fragment]) -> bool:
    """Determine text artifacts based on filename or fragment MIME types."""
    file_name = str(node.get("file_name") or "").lower()
    if file_name.endswith((".md", ".txt", ".json")):
        return True
    return any((getattr(frag, "mime_type", "") or "").startswith("text/") for frag in fragments)


def _guess_filename(node: dict) -> str:
    """Derive a filename from the artifact node label."""
    label = str(node.get("label") or "artifact")
    # Clean up label for filename use
    clean = label.replace(" ", "_").replace("/", "_")
    # Add extension if missing
    if "." not in clean:
        clean += ".md" if _is_text_artifact(node, []) else ".bin"
    return clean


def _resolve_filename(node: dict) -> str:
    """Return filename from stored file_name, falling back to heuristic."""
    file_name = (
        node.get("file_name")
        or (node.get("meta") or {}).get("file_name")
    )
    if not file_name:
        node_id = str(node.get("id") or "")
        if ":" in node_id and "." in node_id.split(":")[-1]:
            file_name = node_id.split(":")[-1]
    
    return file_name or _guess_filename(node)


@router.get("/{artifact_id:path}/download")
def download_artifact(artifact_id: str):
    """Download reconstructed artifact content via IKAM fragment algebra.

    Performs graph traversal to find the artifact's fragments,
    then reconstructs the original content via root relation DAG.
    """
    print(f"DEBUG: download_artifact called with {artifact_id}")
    graph, node = _find_artifact(artifact_id)
    if not graph or not node:
        print(f"DEBUG: artifact {artifact_id} not found in _find_artifact")
        raise HTTPException(status_code=404, detail=f"Artifact {artifact_id} not found")

    fragments = _collect_artifact_fragments(graph, artifact_id)
    if not fragments:
        print(f"DEBUG: No fragments found for artifact {artifact_id}")
        raise HTTPException(status_code=404, detail=f"No fragments found for artifact {artifact_id}")

    # Check if any fragment has a relation — needed for reconstruction
    has_relation = any(is_relation_fragment(f) for f in fragments)
    if not has_relation:
        # Storage-layer fragment (e.g. from ingest_uploaded_file) — serve raw bytes
        frag = fragments[0]
        raw_bytes = getattr(frag, "bytes", None)
        
        # If it's not bytes, maybe it's string content in value?
        if not raw_bytes and getattr(frag, "value", None) is not None:
            if isinstance(frag.value, str):
                raw_bytes = frag.value.encode("utf-8")
            elif isinstance(frag.value, dict):
                import json
                raw_bytes = json.dumps(frag.value).encode("utf-8")

        if raw_bytes is not None:
            filename = _resolve_filename(node)
            mime = getattr(frag, "mime_type", "application/octet-stream")
            return Response(
                content=raw_bytes,
                media_type=mime,
                headers={"Content-Disposition": f'attachment; filename="{filename}"'},
            )
        raise HTTPException(status_code=500, detail="Fragment has no reconstructable content")

    # Reconstruct via IKAM algebra
    filename = _resolve_filename(node)
    if _is_text_artifact(node, fragments):
        content = reconstruct_document(fragments)
        return Response(
            content=content.encode("utf-8"),
            media_type="text/plain; charset=utf-8",
            headers={"Content-Disposition": f'attachment; filename="{filename}"'},
        )
    else:
        content_bytes = reconstruct_binary(fragments)
        return Response(
            content=content_bytes,
            media_type="application/octet-stream",
            headers={"Content-Disposition": f'attachment; filename="{filename}"'},
        )
