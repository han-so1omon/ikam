from __future__ import annotations

import importlib

from fastapi import APIRouter, HTTPException

from ikam_perf_report.benchmarks.store import STORE

# SQI Framework Imports
from modelado.reasoning.query import SemanticQuery, QueryConstraints, InterpretationContext, SearchStrategy
from modelado.reasoning.explorer import GraphExplorer
from modelado.reasoning.synthesizer import SynthesizerService
from modelado.oraculo.factory import create_ai_client_from_env
from modelado.environment_scope import EnvironmentScope

router = APIRouter(prefix="/graph", tags=["graph"])


def _latest_run_for_graph(graph_id: str | None):
    if not graph_id:
        runs = STORE.list_runs()
        return runs[-1] if runs else None
    for run in reversed(STORE.list_runs()):
        if run.project_id == graph_id:
            return run
    return None


def _semantic_refs(run):
    semantic = run.semantic or {} if run else {}
    entity_ids = [str(item.get("id")) for item in semantic.get("entities", []) if item.get("id")]
    entity_labels = {
        str(item.get("id")): str(item.get("label") or item.get("id"))
        for item in semantic.get("entities", [])
        if item.get("id")
    }
    relation_ids = [str(item.get("id")) for item in semantic.get("relations", []) if item.get("id")]
    relation_labels = [str(item.get("kind")) for item in semantic.get("relations", []) if item.get("kind")]
    return entity_ids, entity_labels, relation_ids, relation_labels


def _enrich_nodes(graph, run):
    entity_ids, entity_labels, relation_ids, relation_labels = _semantic_refs(run)
    enriched = []
    for node in graph.nodes:
        node_type = str(node.get("type") or "fragment")
        node_id = str(node.get("id") or "")
        node_entity_ids: list[str] = []
        inferred_entity_id = None
        if isinstance(node.get("meta"), dict):
            if isinstance(node["meta"].get("semantic_entity_ids"), list):
                node_entity_ids = [str(item) for item in node["meta"]["semantic_entity_ids"] if isinstance(item, str)]
            if isinstance(node["meta"].get("semantic_entity_id"), str):
                inferred_entity_id = str(node["meta"]["semantic_entity_id"])
            elif node_entity_ids:
                inferred_entity_id = node_entity_ids[0]

        enriched.append(
            {
                "id": node_id,
                "type": node_type,
                "kind": node_type,
                "label": str(node.get("label") or node_id),
                "level": node.get("level"),
                "salience": node.get("salience"),
                "meta": {
                    **node,
                    "origin": str(node.get("origin") or "map"),
                    "run_id": run.run_id if run else None,
                    "case_id": run.case_id if run else None,
                    "decision_ref": "ikam_map" if run else None,
                    "semantic_entity_ids": node_entity_ids,
                    "semantic_entity_id": inferred_entity_id,
                    "semantic_entity_label": entity_labels.get(inferred_entity_id, inferred_entity_id) if inferred_entity_id else None,
                    "semantic_relation_ids": relation_ids[:20],
                    "semantic_relation_labels": relation_labels[:20],
                },
            }
        )

    for relation in graph.relational_fragments:
        relation_id = str(relation.get("id") or "")
        if not relation_id:
            continue
        relation_kind = str(relation.get("kind") or "relation_fragment")
        relation_status = str(relation.get("status") or "committed")
        enriched.append(
            {
                "id": relation_id,
                "type": "relation_fragment",
                "kind": "relation_fragment",
                "label": relation_kind,
                "level": 1,
                "salience": None,
                "meta": {
                    **relation,
                    "origin": str(relation.get("origin") or "enrichment"),
                    "run_id": run.run_id if run else None,
                    "case_id": run.case_id if run else None,
                    "decision_ref": "enrichment_overlay",
                    "relation_status": relation_status,
                },
            }
        )
    return enriched


def _enrich_edges(graph, run):
    entity_ids, _entity_labels, relation_ids, relation_labels = _semantic_refs(run)
    enriched = []
    for edge in graph.edges:
        label = str(edge.get("label") or edge.get("kind") or "composition")
        origin = "merge" if label == "merge_candidate" else str(edge.get("origin") or "map")
        enriched.append(
            {
                "id": edge.get("id"),
                "source": edge.get("source"),
                "target": edge.get("target"),
                "kind": str(edge.get("kind") or label),
                "meta": {
                    **edge,
                    "origin": origin,
                    "run_id": run.run_id if run else None,
                    "case_id": run.case_id if run else None,
                    "decision_ref": "graph_merge" if origin == "merge" else "ikam_map",
                    "semantic_entity_ids": entity_ids[:20],
                    "semantic_relation_ids": relation_ids[:20],
                    "semantic_relation_labels": relation_labels[:20],
                },
            }
        )

    for relation in graph.relational_fragments:
        relation_id = str(relation.get("id") or "")
        source = relation.get("source")
        target = relation.get("target")
        if not (isinstance(source, str) and source and isinstance(target, str) and target and relation_id):
            continue
        relation_kind = str(relation.get("kind") or "semantic_link")
        relation_status = str(relation.get("status") or "committed")
        enriched.append(
            {
                "id": f"edge:{relation_id}",
                "source": source,
                "target": target,
                "kind": relation_kind,
                "meta": {
                    "origin": "enrichment",
                    "run_id": run.run_id if run else None,
                    "case_id": run.case_id if run else None,
                    "decision_ref": "enrichment_overlay",
                    "relation_id": relation_id,
                    "relation_status": relation_status,
                    "evidence": relation.get("evidence") or [],
                    "rationale": relation.get("rationale") or "",
                    "semantic_entity_ids": entity_ids[:20],
                    "semantic_relation_ids": relation_ids[:20],
                    "semantic_relation_labels": relation_labels[:20],
                },
            }
        )
    return enriched


@router.get("/summary")
def graph_summary(graph_id: str | None = None):
    graph = STORE.get_graph(graph_id) if graph_id else STORE.latest_graph()
    if graph_id and not graph:
        return {"graph_id": graph_id, "status": "missing"}
    if not graph:
        return {
            "nodes": 0,
            "edges": 0,
            "semantic_entities": 0,
            "semantic_relations": 0,
        }
    semantic = {}
    matched_run = None
    runs = STORE.list_runs()
    for run in reversed(runs):
        if run.project_id == graph.graph_id:
            semantic = run.semantic or {}
            matched_run = run
            break
    return {
        "graph_id": graph.graph_id,
        "nodes": len(graph.nodes),
        "edges": len(graph.edges),
        "relation_fragments": len(graph.relational_fragments),
        "semantic_entities": len(semantic.get("entities", [])),
        "semantic_relations": len(semantic.get("relations", [])),
        "answer_quality": matched_run.answer_quality if matched_run else None,
    }


@router.get("/enrichment/runs")
def graph_enrichment_runs(graph_id: str):
    return {"graph_id": graph_id, "runs": STORE.list_enrichment_runs(graph_id)}


@router.get("/enrichment/staged")
def graph_enrichment_staged(graph_id: str):
    return {"graph_id": graph_id, "items": STORE.list_enrichment_items(graph_id)}


@router.post("/enrichment/{enrichment_id}/approve")
def approve_enrichment(enrichment_id: str, graph_id: str):
    return STORE.approve_enrichment(graph_id=graph_id, enrichment_id=enrichment_id)


@router.post("/enrichment/{enrichment_id}/reject")
def reject_enrichment(enrichment_id: str, graph_id: str):
    return STORE.reject_enrichment(graph_id=graph_id, enrichment_id=enrichment_id)


@router.post("/enrichment/commit")
def commit_enrichment_queue(graph_id: str):
    return STORE.commit_stage_queue(graph_id=graph_id)


@router.get("/enrichment/receipts")
def enrichment_receipts(graph_id: str):
    return {"graph_id": graph_id, "receipts": STORE.list_commit_receipts(graph_id)}


@router.get("/nodes")
def graph_nodes(graph_id: str | None = None):
    graph = STORE.get_graph(graph_id) if graph_id else STORE.latest_graph()
    run = _latest_run_for_graph(graph_id)
    if graph_id and not graph:
        return []
    if not graph:
        return []
    return _enrich_nodes(graph, run)


@router.get("/edges")
def graph_edges(graph_id: str | None = None):
    graph = STORE.get_graph(graph_id) if graph_id else STORE.latest_graph()
    run = _latest_run_for_graph(graph_id)
    if graph_id and not graph:
        return []
    if not graph:
        return []
    return _enrich_edges(graph, run)


@router.get("/decisions/{run_id}")
def graph_decisions(run_id: str):
    run = STORE.get_run(run_id)
    if not run:
        return {"run_id": run_id, "decisions": []}
    return {"run_id": run_id, "decisions": run.decisions or []}


@router.get("/explain/entity/{entity_id}")
def explain_semantic_entity(entity_id: str, graph_id: str | None = None):
    run = None
    for item in reversed(STORE.list_runs()):
        if graph_id is None or item.project_id == graph_id:
            run = item
            break
    if not run:
        return {"entity_id": entity_id, "status": "missing"}
    semantic = run.semantic or {}
    entities = semantic.get("entities", [])
    for entity in entities:
        if entity.get("id") == entity_id:
            return entity
    return {"entity_id": entity_id, "status": "missing"}


@router.get("/explain/relation/{relation_id}")
def explain_semantic_relation(relation_id: str, graph_id: str | None = None):
    run = None
    for item in reversed(STORE.list_runs()):
        if graph_id is None or item.project_id == graph_id:
            run = item
            break
    if not run:
        return {"relation_id": relation_id, "status": "missing"}
    semantic = run.semantic or {}
    relations = semantic.get("relations", [])
    for relation in relations:
        if relation.get("id") == relation_id:
            return relation
    return {"relation_id": relation_id, "status": "missing"}


@router.post("/wiki/generate")
def generate_wiki(graph_id: str):
    graph = STORE.get_graph(graph_id)
    run = _latest_run_for_graph(graph_id)
    if not graph or not run:
        return {"graph_id": graph_id, "status": "missing"}

    wiki_module = importlib.import_module("modelado.wiki_generation")
    generate_graph_wiki = getattr(wiki_module, "generate_graph_wiki")
    wiki_doc = generate_graph_wiki(
        graph_id=graph.graph_id,
        run_id=run.run_id,
        nodes=_enrich_nodes(graph, run),
        edges=_enrich_edges(graph, run),
        decisions=run.decisions,
        semantic=run.semantic,
    )
    STORE.set_wiki(graph.graph_id, wiki_doc)
    return wiki_doc


@router.get("/wiki")
def get_wiki(graph_id: str):
    wiki_doc = STORE.get_wiki(graph_id)
    if not wiki_doc:
        return {"graph_id": graph_id, "status": "missing"}
    return wiki_doc


@router.post("/search")
async def search_graph(payload: dict):
    query_text = str(payload.get("query") or "").strip()
    graph_id = payload.get("graph_id")

    # SQI Framework Pipeline
    ai_client = create_ai_client_from_env()
    explorer = GraphExplorer()
    synthesizer = SynthesizerService(ai_client)

    # 1. Identify Anchor Nodes (The 'Finder' step)
    # For BenchmarkStore, we search by keyword if not provided
    anchor_ids = payload.get("anchor_ids", [])
    if not anchor_ids and query_text:
        graph = STORE.get_graph(graph_id) if graph_id else STORE.latest_graph()
        print("GRAPH FOUND:", graph is not None)
        if graph:
            for f in graph.fragments:
                p = STORE._fragment_to_payload(f)
                label = str(p.get("label") or "").lower()
                print("LABEL:", label, "QUERY:", query_text)
                if query_text.lower() in label:
                    anchor_ids.append(str(p.get("id")))
            anchor_ids = list(set(anchor_ids))[:5]  # Deduplicate and limit

    if not anchor_ids:
        raise HTTPException(status_code=422, detail="No anchor fragments found for this query")

    # 2. Define Discovery Constraints (D19 Isolation)
    ref = payload.get("ref") or "refs/heads/main"

    constraints = QueryConstraints(
        env_scope=EnvironmentScope(ref=ref),
        max_hops=payload.get("max_hops", 2),
        extra_filters={"graph_id": graph_id, "anchor_ids": anchor_ids},
    )

    # 3. Discovery (SQI Framework Step 1)
    # We use the 'benchmark_bfs' strategy registered in main.py
    strategy_name = str(payload.get("search_strategy") or "benchmark_bfs")

    semantic_query = SemanticQuery(
        intent=query_text,
        constraints=constraints,
        search_strategy=SearchStrategy(name=strategy_name),
        interpretation=InterpretationContext(
            directives=payload.get("directives", ["Explain the relationships clearly."]),
            audience=payload.get("audience", "Performance Analyst"),
            purpose=payload.get("purpose", "Benchmark Report Analysis"),
        ),
    )

    # Discovery traversal
    try:
        from modelado.db import connection_scope
        with connection_scope() as cx:
            subgraph = explorer.discover(cx, semantic_query)
    except ValueError as exc:
        raise HTTPException(status_code=422, detail=str(exc)) from exc

    if not subgraph.nodes:
        raise HTTPException(status_code=422, detail="No supported IR nodes discovered for this query")

    # 4. Synthesis / Interpretation (SQI Framework Step 2 - D18 Attribution)
    # This involves the AIClient call to summarize the discovered facts.
    try:
        result = await synthesizer.synthesize(semantic_query, subgraph)
    except ValueError as exc:
        raise HTTPException(status_code=422, detail=str(exc)) from exc

    # 5. Format response
    # Map SQI Subgraph nodes to the legacy 'results' and 'scores' schema for frontend compatibility
    results = []
    scores = []
    for node in subgraph.nodes:
        results.append({"node_id": node.fragment_id, "group_ids": ["sqi-discovery"], "confidence": 1.0})
        scores.append(
            {
                "node_id": node.fragment_id,
                "semantic": 1.0,
                "graph": 1.0,
                "evidence": 1.0,
                "confidence": 1.0,
            }
        )

    return {
        "query": query_text,
        "query_type": "sqi-framework",
        "results": results,
        "groups": [{"id": "sqi-discovery", "label": "Discovered Evidence", "size": len(results)}],
        "interpretation": result.get("interpretation"),
        "attribution": result.get("attribution"),
        "scores": scores,
        "subgraph": {"nodes": len(subgraph.nodes), "edges": len(subgraph.edges)},
    }
