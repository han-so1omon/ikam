from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, List
from uuid import uuid4

from ikam_perf_report.benchmarks.aqs import summarize_aqs
from ikam_perf_report.benchmarks.debug_models import DebugRunState, DebugStepEvent
from modelado.environment_scope import EnvironmentScope
from modelado.hot_subgraph_store import InMemoryHotSubgraphStore, JsonValue, SubgraphRef


@dataclass
class GraphSnapshot:
    graph_id: str
    nodes: List[Dict[str, Any]] = field(default_factory=list)
    edges: List[Dict[str, Any]] = field(default_factory=list)
    relational_fragments: List[Dict[str, Any]] = field(default_factory=list)
    manifests: List[Dict[str, Any]] = field(default_factory=list)
    fragments: List[Any] = field(default_factory=list)


@dataclass
class BenchmarkRunRecord:
    run_id: str
    project_id: str
    case_id: str
    stages: List[Dict[str, Any]]
    decisions: List[Dict[str, Any]]
    project: Dict[str, Any]
    graph: GraphSnapshot
    semantic: Dict[str, Any] | None = None
    answer_quality: Dict[str, Any] | None = None
    commit_receipt: Dict[str, Any] | None = None
    evaluation: Dict[str, Any] | None = None


@dataclass
class EnrichmentItem:
    enrichment_id: str
    run_id: str
    graph_id: str
    relation_id: str
    relation_kind: str
    source: str
    target: str
    rationale: str = ""
    evidence: List[str] = field(default_factory=list)
    status: str = "staged"  # staged|approved|queued|committed|rejected
    sequence: int = 0
    lane_mode: str = "explore-graph"
    unresolved: bool = False


@dataclass
class EnrichmentRun:
    enrichment_id: str
    run_id: str
    graph_id: str
    sequence: int
    lane_mode: str
    status: str
    relation_count: int
    unresolved_count: int


class BenchmarkStore:
    def __init__(self) -> None:
        self._runs: Dict[str, BenchmarkRunRecord] = {}
        self._graphs: Dict[str, GraphSnapshot] = {}
        self._wikis: Dict[str, Dict[str, Any]] = {}
        self._case_counters: Dict[str, int] = {}
        self._enrichment_sequence: Dict[str, int] = {}
        self._enrichment_runs_by_graph: Dict[str, List[EnrichmentRun]] = {}
        self._enrichment_items_by_graph: Dict[str, List[EnrichmentItem]] = {}
        self._commit_receipts_by_graph: Dict[str, List[Dict[str, Any]]] = {}
        self._debug_runs: Dict[str, DebugRunState] = {}
        self._debug_events: Dict[str, List[DebugStepEvent]] = {}
        self._debug_runtime_context: Dict[str, Dict[str, Any]] = {}
        self._hot_subgraph_store = InMemoryHotSubgraphStore()
        self._hot_subgraph_refs_by_run: Dict[str, Dict[str, SubgraphRef]] = {}
        self._control_commands: set[tuple[str, str]] = set()

    def add_run(self, run: BenchmarkRunRecord) -> None:
        self._runs[run.run_id] = run
        self._graphs[run.project_id] = run.graph

    def next_project_id(self, case_id: str) -> str:
        counter = self._case_counters.get(case_id, 0) + 1
        self._case_counters[case_id] = counter
        return f"{case_id}#{counter}"

    def get_run(self, run_id: str) -> BenchmarkRunRecord | None:
        return self._runs.get(run_id)

    def list_runs(self) -> List[BenchmarkRunRecord]:
        return list(reversed(list(self._runs.values())))

    def get_graph(self, graph_id: str) -> GraphSnapshot | None:
        return self._graphs.get(graph_id)

    def set_wiki(self, graph_id: str, wiki: Dict[str, Any]) -> None:
        self._wikis[graph_id] = wiki

    def get_wiki(self, graph_id: str) -> Dict[str, Any] | None:
        return self._wikis.get(graph_id)

    def latest_graph(self) -> GraphSnapshot | None:
        runs = self.list_runs()
        if not runs:
            return None
        return runs[0].graph

    def reset(self) -> None:
        self._runs.clear()
        self._graphs.clear()
        self._wikis.clear()
        self._case_counters.clear()
        self._enrichment_sequence.clear()
        self._enrichment_runs_by_graph.clear()
        self._enrichment_items_by_graph.clear()
        self._commit_receipts_by_graph.clear()
        self._debug_runs.clear()
        self._debug_events.clear()
        self._debug_runtime_context.clear()
        self._hot_subgraph_store = InMemoryHotSubgraphStore()
        self._hot_subgraph_refs_by_run.clear()
        self._control_commands.clear()

    def create_debug_run_state(self, state: DebugRunState) -> None:
        self._debug_runs[state.run_id] = state

    def get_debug_run_state(self, run_id: str) -> DebugRunState | None:
        return self._debug_runs.get(run_id)

    def set_debug_run_state(self, run_id: str, state: DebugRunState) -> None:
        self._debug_runs[run_id] = state

    def append_debug_event(self, event: DebugStepEvent) -> None:
        bucket = self._debug_events.setdefault(event.run_id, [])
        bucket.append(event)

    @staticmethod
    def _deep_merge(base: Dict[str, Any], patch: Dict[str, Any]) -> Dict[str, Any]:
        merged = dict(base)
        for key, value in patch.items():
            if isinstance(value, dict) and isinstance(merged.get(key), dict):
                merged[key] = BenchmarkStore._deep_merge(merged[key], value)
            else:
                merged[key] = value
        return merged

    def update_debug_event(self, *, run_id: str, step_id: str, **changes: Any) -> DebugStepEvent:
        bucket = self._debug_events.get(run_id, [])
        for index, event in enumerate(bucket):
            if event.step_id != step_id:
                continue
            payload = dict(event.__dict__)
            for key, value in changes.items():
                if key == "metrics" and isinstance(value, dict) and isinstance(payload.get("metrics"), dict):
                    payload["metrics"] = self._deep_merge(payload["metrics"], value)
                elif value is not None:
                    payload[key] = value
            updated = DebugStepEvent(**payload)
            bucket[index] = updated
            return updated
        raise KeyError(f"Unknown debug step_id: {step_id}")

    def list_debug_events(self, run_id: str) -> List[DebugStepEvent]:
        return list(self._debug_events.get(run_id, []))

    def register_control_command(self, run_id: str, command_id: str) -> bool:
        key = (run_id, command_id)
        if key in self._control_commands:
            return False
        self._control_commands.add(key)
        return True

    def set_debug_runtime_context(self, run_id: str, context: Dict[str, Any]) -> None:
        self._debug_runtime_context[run_id] = dict(context)

    def get_debug_runtime_context(self, run_id: str) -> Dict[str, Any] | None:
        context = self._debug_runtime_context.get(run_id)
        if context is None:
            return None
        return dict(context)

    def put_hot_subgraph(self, *, run_id: str, step_id: str, contract_type: str, payload: JsonValue) -> str:
        ref = self._hot_subgraph_store.put(payload)
        hot_ref = f"hot://{run_id}/{contract_type}/{step_id}"
        bucket = self._hot_subgraph_refs_by_run.setdefault(run_id, {})
        bucket[hot_ref] = ref
        return hot_ref

    def get_hot_subgraph(self, hot_ref: str) -> JsonValue | None:
        for refs_by_name in self._hot_subgraph_refs_by_run.values():
            ref = refs_by_name.get(hot_ref)
            if ref is not None:
                return self._hot_subgraph_store.get(ref)
        return None

    @staticmethod
    def _fragment_to_payload(fragment: Any) -> Dict[str, Any]:
        if isinstance(fragment, dict):
            payload = dict(fragment)
            payload.setdefault("id", payload.get("cas_id"))
            payload.setdefault("meta", payload.get("metadata", {}))
            return payload

        payload: Dict[str, Any] = {
            "id": getattr(fragment, "id", None) or getattr(fragment, "cas_id", None),
            "cas_id": getattr(fragment, "cas_id", None),
            "mime_type": getattr(fragment, "mime_type", None),
            "value": getattr(fragment, "value", None),
            "meta": getattr(fragment, "meta", None) or getattr(fragment, "metadata", None) or {},
        }
        return payload

    @staticmethod
    def _meta_get(meta: Any, snake: str, camel: str) -> Any:
        if not isinstance(meta, dict):
            return None
        if snake in meta:
            return meta.get(snake)
        return meta.get(camel)

    @staticmethod
    def _payload_ref(*, env_type: str, env_id: str) -> str:
        normalized_type = env_type.strip()
        normalized_id = env_id.strip()
        if normalized_type == "committed":
            return EnvironmentScope(ref="refs/heads/main").ref
        if normalized_type == "staging":
            return EnvironmentScope(ref=f"refs/heads/staging/{normalized_id}").ref
        if normalized_type == "dev":
            return EnvironmentScope(ref=f"refs/heads/run/{normalized_id}").ref
        raise ValueError(f"Invalid env_type: {env_type}")

    @classmethod
    def _resolve_scope_ref(
        cls,
        *,
        ref: str | None = None,
        env_type: str | None = None,
        env_id: str | None = None,
    ) -> str:
        if isinstance(ref, str) and ref.strip():
            return EnvironmentScope(ref=ref.strip()).ref
        if isinstance(env_type, str) and isinstance(env_id, str) and env_type.strip() and env_id.strip():
            return cls._payload_ref(env_type=env_type, env_id=env_id)
        raise ValueError("scope requires ref or env_type/env_id")

    @classmethod
    def _meta_ref(cls, meta: Dict[str, Any]) -> str | None:
        ref = meta.get("ref")
        if isinstance(ref, str) and ref.strip():
            return ref.strip()
        env_type = cls._meta_get(meta, "env_type", "envType")
        env_id = cls._meta_get(meta, "env_id", "envId")
        if isinstance(env_type, str) and isinstance(env_id, str) and env_type.strip() and env_id.strip():
            return cls._payload_ref(env_type=env_type, env_id=env_id)
        return None

    @staticmethod
    def _meta_dict(payload: Dict[str, Any]) -> Dict[str, Any]:
        raw = payload.get("meta")
        if isinstance(raw, dict):
            return dict(raw)
        return {}

    def list_scoped_fragments(
        self,
        *,
        run_id: str,
        ref: str | None = None,
        env_type: str | None = None,
        env_id: str | None = None,
        step_id: str,
        attempt_index: int,
    ) -> List[Dict[str, Any]]:
        run = self._runs.get(run_id)
        if not run:
            return []

        target_ref = self._resolve_scope_ref(ref=ref, env_type=env_type, env_id=env_id)
        out: List[Dict[str, Any]] = []
        for fragment in run.graph.fragments:
            payload = self._fragment_to_payload(fragment)
            meta = self._meta_dict(payload)
            frag_step_id = self._meta_get(meta, "step_id", "stepId")
            frag_attempt = self._meta_get(meta, "attempt_index", "attemptIndex")

            if self._meta_ref(meta) != target_ref:
                continue
            if frag_step_id != step_id:
                continue
            if frag_attempt != attempt_index:
                continue

            out.append(payload)
        return out

    def list_environment_fragments(
        self,
        *,
        run_id: str,
        ref: str | None = None,
        env_type: str | None = None,
        env_id: str | None = None,
    ) -> List[Dict[str, Any]]:
        run = self._runs.get(run_id)
        if not run:
            return []

        target_ref = self._resolve_scope_ref(ref=ref, env_type=env_type, env_id=env_id)
        out: List[Dict[str, Any]] = []
        for fragment in run.graph.fragments:
            payload = self._fragment_to_payload(fragment)
            meta = self._meta_dict(payload)
            if self._meta_ref(meta) != target_ref:
                continue

            out.append(payload)
        return out

    @staticmethod
    def _is_verification_record(payload: Dict[str, Any]) -> bool:
        meta = BenchmarkStore._meta_dict(payload)
        record_type = meta.get("record_type") or meta.get("recordType")
        mime_type = str(payload.get("mime_type") or "")
        return record_type == "verification" or "verification" in mime_type

    @staticmethod
    def _is_reconstruction_program(payload: Dict[str, Any]) -> bool:
        meta = BenchmarkStore._meta_dict(payload)
        record_type = meta.get("record_type") or meta.get("recordType")
        mime_type = str(payload.get("mime_type") or "")
        return record_type == "reconstruction_program" or "reconstruction-program" in mime_type

    def list_scoped_verification_records(
        self,
        *,
        run_id: str,
        ref: str | None = None,
        env_type: str | None = None,
        env_id: str | None = None,
        step_id: str,
        attempt_index: int,
    ) -> List[Dict[str, Any]]:
        scoped = self.list_scoped_fragments(
            run_id=run_id,
            ref=ref,
            env_type=env_type,
            env_id=env_id,
            step_id=step_id,
            attempt_index=attempt_index,
        )
        return [payload for payload in scoped if self._is_verification_record(payload)]

    def list_scoped_reconstruction_programs(
        self,
        *,
        run_id: str,
        ref: str | None = None,
        env_type: str | None = None,
        env_id: str | None = None,
        step_id: str,
        attempt_index: int,
    ) -> List[Dict[str, Any]]:
        scoped = self.list_scoped_fragments(
            run_id=run_id,
            ref=ref,
            env_type=env_type,
            env_id=env_id,
            step_id=step_id,
            attempt_index=attempt_index,
        )
        return [payload for payload in scoped if self._is_reconstruction_program(payload)]

    def summarize_environment_scope(
        self,
        *,
        run_id: str,
        ref: str | None = None,
        env_type: str | None = None,
        env_id: str | None = None,
    ) -> Dict[str, Any]:
        scope = EnvironmentScope(ref=ref) if isinstance(ref, str) and ref.strip() else EnvironmentScope(ref=self._resolve_scope_ref(env_type=env_type, env_id=env_id))
        run = self._runs.get(run_id)
        if not run:
            return {
                "fragment_count": 0,
                "verification_count": 0,
                "reconstruction_program_count": 0,
                "ref": scope.ref,
                "executors_seen": [],
            }

        target_ref = scope.ref
        scoped: List[Dict[str, Any]] = []
        for fragment in run.graph.fragments:
            payload = self._fragment_to_payload(fragment)
            meta = self._meta_dict(payload)
            if self._meta_ref(meta) == target_ref:
                scoped.append(payload)

        verification_count = sum(1 for payload in scoped if self._is_verification_record(payload))
        reconstruction_count = sum(1 for payload in scoped if self._is_reconstruction_program(payload))
        executors_seen = sorted(
            {
                str(executor_kind)
                for event in self._debug_events.get(run_id, [])
                if self._payload_ref(env_type=event.env_type, env_id=event.env_id) == target_ref
                for executor_kind in [
                    ((event.metrics.get("trace") or {}).get("executor_kind") if isinstance(event.metrics, dict) and isinstance(event.metrics.get("trace"), dict) else None)
                ]
                if isinstance(executor_kind, str) and executor_kind.strip()
            }
        )
        return {
            "fragment_count": len(scoped),
            "verification_count": verification_count,
            "reconstruction_program_count": reconstruction_count,
            "ref": target_ref,
            "executors_seen": executors_seen,
        }

    def next_enrichment_id(self, graph_id: str) -> str:
        seq = self._enrichment_sequence.get(graph_id, 0) + 1
        self._enrichment_sequence[graph_id] = seq
        return f"{graph_id}:enrichment:{seq}"

    def add_enrichment_run(self, run: EnrichmentRun, items: List[EnrichmentItem]) -> None:
        bucket = self._enrichment_runs_by_graph.setdefault(run.graph_id, [])
        bucket.append(run)
        item_bucket = self._enrichment_items_by_graph.setdefault(run.graph_id, [])
        item_bucket.extend(items)

    def list_enrichment_runs(self, graph_id: str) -> List[Dict[str, Any]]:
        runs = self._enrichment_runs_by_graph.get(graph_id, [])
        return [
            {
                "enrichment_id": run.enrichment_id,
                "run_id": run.run_id,
                "graph_id": run.graph_id,
                "sequence": run.sequence,
                "lane_mode": run.lane_mode,
                "status": run.status,
                "relation_count": run.relation_count,
                "unresolved_count": run.unresolved_count,
            }
            for run in sorted(runs, key=lambda item: item.sequence)
        ]

    def list_enrichment_items(self, graph_id: str) -> List[Dict[str, Any]]:
        items = self._enrichment_items_by_graph.get(graph_id, [])
        return [
            {
                "enrichment_id": item.enrichment_id,
                "run_id": item.run_id,
                "graph_id": item.graph_id,
                "relation_id": item.relation_id,
                "relation_kind": item.relation_kind,
                "source": item.source,
                "target": item.target,
                "rationale": item.rationale,
                "evidence": list(item.evidence),
                "status": item.status,
                "sequence": item.sequence,
                "lane_mode": item.lane_mode,
                "unresolved": item.unresolved,
            }
            for item in sorted(items, key=lambda entry: (entry.sequence, entry.relation_id))
        ]

    def list_stage_queue(self, graph_id: str) -> List[Dict[str, Any]]:
        return [item for item in self.list_enrichment_items(graph_id) if item["status"] == "queued"]

    def approve_enrichment(self, graph_id: str, enrichment_id: str) -> Dict[str, Any]:
        items = self._enrichment_items_by_graph.get(graph_id, [])
        moved = 0
        for item in items:
            if item.enrichment_id != enrichment_id:
                continue
            if item.status in ("staged", "approved"):
                item.status = "queued"
                moved += 1
        self._refresh_enrichment_run_status(graph_id, enrichment_id)
        return {"graph_id": graph_id, "enrichment_id": enrichment_id, "queued": moved}

    def reject_enrichment(self, graph_id: str, enrichment_id: str) -> Dict[str, Any]:
        items = self._enrichment_items_by_graph.get(graph_id, [])
        changed = 0
        for item in items:
            if item.enrichment_id != enrichment_id:
                continue
            if item.status in ("staged", "approved", "queued"):
                item.status = "rejected"
                changed += 1
        self._refresh_enrichment_run_status(graph_id, enrichment_id)
        return {"graph_id": graph_id, "enrichment_id": enrichment_id, "rejected": changed}

    def commit_stage_queue(self, graph_id: str) -> Dict[str, Any]:
        graph = self._graphs.get(graph_id)
        if graph is None:
            return {
                "graph_id": graph_id,
                "committed": 0,
                "receipt": None,
            }

        items = self._enrichment_items_by_graph.get(graph_id, [])
        queued = [item for item in items if item.status == "queued"]
        committed_edges = 0
        committed_relations = 0
        existing_edge_keys = {(edge.get("source"), edge.get("target"), edge.get("kind") or edge.get("label")) for edge in graph.edges}
        existing_rel_ids = {fragment.get("id") for fragment in graph.relational_fragments}

        for item in queued:
            edge_key = (item.source, item.target, item.relation_kind)
            if edge_key not in existing_edge_keys:
                graph.edges.append(
                    {
                        "id": f"edge-{uuid4().hex}",
                        "source": item.source,
                        "target": item.target,
                        "kind": item.relation_kind,
                        "label": item.relation_kind,
                        "meta": {
                            "origin": "enrichment",
                            "enrichment_id": item.enrichment_id,
                            "relation_id": item.relation_id,
                            "status": "committed",
                            "evidence": item.evidence,
                            "rationale": item.rationale,
                        },
                    }
                )
                existing_edge_keys.add(edge_key)
                committed_edges += 1

            if item.relation_id not in existing_rel_ids:
                graph.relational_fragments.append(
                    {
                        "id": item.relation_id,
                        "kind": item.relation_kind,
                        "source": item.source,
                        "target": item.target,
                        "status": "committed",
                        "evidence": list(item.evidence),
                        "rationale": item.rationale,
                        "enrichment_id": item.enrichment_id,
                    }
                )
                existing_rel_ids.add(item.relation_id)
                committed_relations += 1

            item.status = "committed"
            self._refresh_enrichment_run_status(graph_id, item.enrichment_id)

        receipt = {
            "receipt_id": f"commit-{uuid4().hex}",
            "graph_id": graph_id,
            "committed": len(queued),
            "committed_edges": committed_edges,
            "committed_relations": committed_relations,
        }
        if queued:
            self._commit_receipts_by_graph.setdefault(graph_id, []).append(receipt)
        return {
            "graph_id": graph_id,
            "committed": len(queued),
            "receipt": receipt if queued else None,
        }

    def list_commit_receipts(self, graph_id: str) -> List[Dict[str, Any]]:
        return list(self._commit_receipts_by_graph.get(graph_id, []))

    def _refresh_enrichment_run_status(self, graph_id: str, enrichment_id: str) -> None:
        runs = self._enrichment_runs_by_graph.get(graph_id, [])
        items = [item for item in self._enrichment_items_by_graph.get(graph_id, []) if item.enrichment_id == enrichment_id]
        if not runs or not items:
            return
        statuses = {item.status for item in items}
        next_status = "staged"
        if statuses == {"rejected"}:
            next_status = "rejected"
        elif "queued" in statuses:
            next_status = "queued"
        elif statuses == {"committed"}:
            next_status = "committed"
        elif "staged" in statuses:
            next_status = "staged"
        for run in runs:
            if run.enrichment_id == enrichment_id:
                run.status = next_status
                run.relation_count = len(items)
                run.unresolved_count = sum(1 for item in items if item.unresolved)
                break

    def apply_review(self, run_id: str, review_payload: Dict[str, Any]) -> Dict[str, Any] | None:
        run = self._runs.get(run_id)
        if not run or not run.answer_quality:
            return None

        query_id = str(review_payload.get("query_id") or "").strip()
        if not query_id:
            return None

        relevance = float(review_payload.get("relevance", 0.0))
        fidelity = float(review_payload.get("fidelity", 0.0))
        clarity = float(review_payload.get("clarity", 0.0))
        note = str(review_payload.get("note", "")).strip()

        next_evaluations: list[Dict[str, Any]] = []
        for query in run.answer_quality.get("query_scores", []):
            current_query_id = str(query.get("query_id") or "")
            oracle = dict(query.get("oracle") or {})
            current_review = query.get("review") if query.get("review_mode") == "manual" else None
            if current_query_id == query_id:
                current_review = {
                    "relevance": relevance,
                    "fidelity": fidelity,
                    "clarity": clarity,
                    "note": note,
                }
            next_evaluations.append(
                {
                    "query_id": current_query_id,
                    "oracle": oracle,
                    "review": current_review,
                }
            )

        run.answer_quality = summarize_aqs(next_evaluations)
        self._runs[run_id] = run
        return run.answer_quality

    def apply_merge_updates(
        self,
        graph_ids: list[str],
        proposed_edges: list[Dict[str, Any]],
        proposed_relational_fragments: list[Dict[str, Any]],
    ) -> Dict[str, Any]:
        edge_updates = 0
        relation_updates = 0
        for graph_id in graph_ids:
            graph = self._graphs.get(graph_id)
            if not graph:
                continue
            existing_edge_keys = {
                (edge.get("source"), edge.get("target"), edge.get("label"))
                for edge in graph.edges
            }
            for edge in proposed_edges:
                key = (edge.get("source"), edge.get("target"), edge.get("label"))
                if key in existing_edge_keys:
                    continue
                graph.edges.append(
                    {
                        "id": edge.get("id") or f"edge-{uuid4().hex}",
                        "source": edge.get("source"),
                        "target": edge.get("target"),
                        "label": edge.get("label", "merge_candidate"),
                        "properties": edge.get("properties", {}),
                        "project_id": graph_id,
                    }
                )
                existing_edge_keys.add(key)
                edge_updates += 1

            existing_rel_ids = {rel.get("id") for rel in graph.relational_fragments}
            for relation in proposed_relational_fragments:
                relation_id = relation.get("id") or f"relation-{uuid4().hex}"
                if relation_id in existing_rel_ids:
                    continue
                new_relation = dict(relation)
                new_relation["id"] = relation_id
                new_relation["project_id"] = graph_id
                graph.relational_fragments.append(new_relation)
                existing_rel_ids.add(relation_id)
                relation_updates += 1

        return {
            "edge_updates": edge_updates,
            "relational_fragment_updates": relation_updates,
        }


STORE = BenchmarkStore()
