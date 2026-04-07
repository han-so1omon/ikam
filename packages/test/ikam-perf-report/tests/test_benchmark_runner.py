import os
from pathlib import Path

from ikam_perf_report.benchmarks import runner
from ikam_perf_report.benchmarks.store import STORE
import pytest
from fastapi import HTTPException

# ---------------------------------------------------------------------------
# Real fixture helper — reads brand-guide.md from s-local-retail-v01 case.
# Replaces inline b"# Revenue..." stubs with actual multi-section markdown.
# ---------------------------------------------------------------------------
_BRAND_GUIDE_PATH = (
    Path(__file__).resolve().parents[4]  # repo root
    / "tests" / "fixtures" / "cases" / "s-local-retail-v01" / "brand-guide.md"
)


def _brand_guide_source() -> bytes:
    """Load brand-guide.md bytes from the real fixture directory."""
    return _BRAND_GUIDE_PATH.read_bytes()


def test_benchmark_records_stage_and_decision(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": [{"id": "r1"}]},
    )
    result = runner.run_benchmark_legacy(case_ids="s-construction-v01")
    assert result["runs"]
    first = result["runs"][0]
    assert first["stages"]
    assert first["decisions"]
    assert first["stages"][0]["stage_name"] == "semantic_pipeline"
    assert first["stages"][1]["stage_name"] == "decompose_artifacts"
    assert first["stages"][1]["duration_ms"] >= 1


def test_run_records_semantic_entities(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": []},
    )
    result = runner.run_benchmark_legacy(case_ids="s-construction-v01")
    assert result["runs"][0]["semantic"]
    assert result["runs"][0]["semantic"]["entities"]


def test_run_includes_deterministic_commit_receipt(case_fixtures_root, monkeypatch):
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": [{"id": "r1"}]},
    )
    first = runner.run_benchmark_legacy(case_ids="s-construction-v01")

    first_receipt = first["runs"][0]["commit_receipt"]
    assert first_receipt["receipt_id"]
    assert first_receipt["mode"] == "commit-strict"
    assert first_receipt["case_id"] == "s-construction-v01"
    assert first_receipt["target_ref"] == "refs/heads/main"
    assert isinstance(first_receipt["promoted_fragment_ids"], list)
    assert isinstance(first_receipt["edge_idempotency_keys"], list)


def test_invalid_case_ids_raise_http_error(case_fixtures_root):
    with pytest.raises(HTTPException) as exc:
        runner.run_benchmark(case_ids="missing-case")
    assert exc.value.status_code == 400


def test_graph_snapshot_contains_artifact_manifest_structure(case_fixtures_root, monkeypatch):
    """Assert graph snapshot includes content nodes, edges, and manifests.

    Contract: A4 (shared fragment reuse), B1 (manifest composition).
    V3 fragments encode hierarchy in relation fragments (edges) rather than
    per-fragment ``artifact_id`` fields. Content nodes are non-relation
    fragments; edges are extracted from relation fragments.
    """
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [], "relations": []},
    )
    STORE.reset()
    result = runner.run_benchmark_legacy(case_ids="s-construction-v01")
    project_id = result["runs"][0]["project_id"]
    graph = STORE.get_graph(project_id)
    assert graph is not None

    # Graph must contain content fragment nodes
    content_nodes = [n for n in graph.nodes if n.get("type") in ("text", "binary")]
    assert content_nodes, "Graph must contain content fragment nodes"

    # Graph must expose artifact root nodes to provide file-level structure
    artifact_nodes = [n for n in graph.nodes if n.get("type") == "artifact"]
    assert artifact_nodes, "Graph must contain artifact root nodes"

    artifact_root_edges = [e for e in graph.edges if (e.get("kind") or e.get("label")) == "artifact-root"]
    assert artifact_root_edges, "Graph must include artifact-root edges"
    assert any(edge.get("source") != edge.get("target") for edge in artifact_root_edges), (
        "Artifact-root edges should connect artifact nodes to fragment nodes"
    )

    # Manifests must be populated in the snapshot
    assert graph.manifests, "Graph snapshot must include manifests"
    for manifest in graph.manifests:
        assert manifest["kind"]
        assert manifest["artifactId"]
        assert isinstance(manifest["fragments"], list)


def test_run_benchmark_includes_oraculo_evaluation(monkeypatch):
    """run_benchmark must include oráculo evaluation when oracle fixture exists.

    Uses real s-local-retail-v01 case (has oracle.json) with monkeypatched
    semantic pipeline (avoids that OpenAI call) but real OpenAI judge for
    oráculo evaluation. Asserts the unified pipeline returns evaluation data
    alongside the graph snapshot in a single run response.
    """
    # Point at real fixtures (not the temp dir from case_fixtures_root)
    real_root = os.path.join(
        os.path.dirname(__file__), os.pardir, os.pardir, os.pardir, os.pardir,
        "tests", "fixtures", "cases",
    )
    real_root = os.path.normpath(real_root)
    monkeypatch.setenv("IKAM_CASES_ROOT", real_root)

    # Monkeypatch semantic pipeline to avoid its separate OpenAI call
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {
            "entities": [
                {"id": "e1", "label": "Maya Chen", "canonical_label": "maya chen"},
                {"id": "e2", "label": "Bramble & Bitters", "canonical_label": "bramble & bitters"},
            ],
            "relations": [
                {
                    "id": "r1",
                    "kind": "employs",
                    "source": "e2",
                    "target": "e1",
                    "source_label": "Bramble & Bitters",
                    "target_label": "Maya Chen",
                },
            ],
        },
    )

    STORE.reset()
    result = runner.run_benchmark_legacy(case_ids="s-local-retail-v01")
    assert result["runs"], "Should produce at least one run"
    first_run = result["runs"][0]

    # Core assertion: evaluation field present and structured
    assert "evaluation" in first_run, (
        "run_benchmark must include 'evaluation' from unified oráculo pipeline"
    )
    evaluation = first_run["evaluation"]
    assert isinstance(evaluation, dict)

    # Report structure checks
    report = evaluation.get("report") or evaluation
    assert "compression" in report, "Evaluation report must include compression metrics"
    assert "entities" in report, "Evaluation report must include entity coverage"
    assert "predicates" in report, "Evaluation report must include predicate coverage"
    assert "exploration" in report, "Evaluation report must include exploration recall"
    assert "query" in report, "Evaluation report must include query quality"
    assert "passed" in report, "Evaluation report must include overall pass/fail"

    # Rendered text must be present
    assert "rendered" in evaluation, "Evaluation must include rendered text"
    assert isinstance(evaluation["rendered"], str)
    assert len(evaluation["rendered"]) > 0

    # Detailed traces must be exposed for debugging.
    assert "details" in evaluation
    details = evaluation["details"]
    assert "pipeline_steps" in details
    assert "entities" in details
    assert "predicates" in details
    assert "exploration_queries" in details
    assert "query_results" in details


def test_run_benchmark_can_skip_oraculo_evaluation(monkeypatch):
    real_root = os.path.join(
        os.path.dirname(__file__), os.pardir, os.pardir, os.pardir, os.pardir,
        "tests", "fixtures", "cases",
    )
    real_root = os.path.normpath(real_root)
    monkeypatch.setenv("IKAM_CASES_ROOT", real_root)

    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": []},
    )

    def _unexpected_oraculo(*_args, **_kwargs):
        raise AssertionError("oraculo_evaluation_must_not_run_when_include_evaluation_false")

    monkeypatch.setattr(runner, "_run_oraculo_evaluation", _unexpected_oraculo)

    STORE.reset()
    result = runner.run_benchmark_legacy(case_ids="s-local-retail-v01", include_evaluation=False)
    assert result["runs"]
    assert result["runs"][0]["evaluation"] == {"status": "skipped", "reason": "fast_debug_init"}


def test_run_benchmark_records_all_pipeline_stages(case_fixtures_root, monkeypatch):
    """Contract: run_benchmark must record timing stages for all major phases.

    Expected stages (in order):
      semantic_pipeline, decompose_artifacts, graph_conversion,
      quality_signals, enrichment_prep, debug_init

    Each stage must have: stage_name, duration_ms >= 1, started_at, ended_at.
    """
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": [{"id": "r1"}]},
    )
    result = runner.run_benchmark_legacy(case_ids="s-construction-v01")
    first = result["runs"][0]
    stages = first["stages"]

    expected_stage_names = [
        "semantic_pipeline",
        "decompose_artifacts",
        "graph_conversion",
        "quality_signals",
        "enrichment_prep",
        "debug_init",
    ]

    actual_stage_names = [s["stage_name"] for s in stages]
    assert actual_stage_names == expected_stage_names, (
        f"Expected stages {expected_stage_names}, got {actual_stage_names}"
    )

    for stage in stages:
        assert "stage_name" in stage
        assert stage["duration_ms"] >= 1, f"Stage '{stage['stage_name']}' has invalid duration"
        assert "started_at" in stage, f"Stage '{stage['stage_name']}' missing started_at"
        assert "ended_at" in stage, f"Stage '{stage['stage_name']}' missing ended_at"


def test_run_benchmark_skipped_evaluation_returns_structured_payload(case_fixtures_root, monkeypatch):
    """Contract: when include_evaluation=False, evaluation must be a structured dict, not None.

    Expected: {"status": "skipped", "reason": "fast_debug_init"} instead of bare None.
    """
    monkeypatch.setattr(
        runner,
        "run_semantic_pipeline",
        lambda text: {"entities": [{"id": "e1"}], "relations": []},
    )
    result = runner.run_benchmark_legacy(case_ids="s-construction-v01", include_evaluation=False)
    evaluation = result["runs"][0]["evaluation"]

    assert isinstance(evaluation, dict), (
        f"Expected structured dict for skipped evaluation, got {type(evaluation).__name__}: {evaluation!r}"
    )
    assert evaluation["status"] == "skipped"
    assert evaluation["reason"] == "fast_debug_init"


def test_lift_handler_produces_real_claim_ir_fragments():
    """Lift handler must use a real Lifter to produce ClaimIR fragments.

    TDD RED: The current stub copies metadata dicts, NOT real IR fragments.
    The real handler must:
    - Create a ClaimLifter (Lifter protocol) with a real LLM client
    - Call lifter.lift() on each surface (structural) fragment
    - Store Fragment objects with CLAIM_IR MIME type in state.outputs["ir_fragments"]
    - Store a lifted_from_map (IR cas_id → source cas_id) in state.outputs
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.fragments import Fragment
    from ikam.ir.mime_types import CLAIM_IR

    # Real decomposition using brand-guide.md fixture
    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source,
        mime_type="text/markdown",
        artifact_id="test-lift:art-1",
    )
    decomposition = decomposer.decompose(directive)
    assert len(decomposition.structural) > 0, "Decomposition must produce structural fragments"

    # Execute lift with real decomposition in state
    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-lift:art-1",
        outputs={"decomposition": decomposition},
    )
    metrics = asyncio.run(execute_step("map.conceptual.normalize.discovery", state))

    # Must produce ir_fragments as a list of real Fragment objects
    ir_fragments = state.outputs.get("ir_fragments")
    assert isinstance(ir_fragments, list), f"Expected list of ir_fragments, got {type(ir_fragments)}"
    assert len(ir_fragments) > 0, "Lift must produce at least one IR fragment"

    for frag in ir_fragments:
        assert isinstance(frag, Fragment), f"IR fragment must be Fragment, got {type(frag)}"
        assert frag.mime_type == CLAIM_IR, f"IR fragment must have CLAIM_IR MIME, got {frag.mime_type}"
        assert frag.cas_id is not None, "IR fragment must have CAS ID (content-addressed)"
        # Value must be a ClaimIR-shaped dict with SPO fields
        assert isinstance(frag.value, dict), f"IR fragment value must be dict, got {type(frag.value)}"
        assert "subject" in frag.value, "ClaimIR must have subject"
        assert "predicate" in frag.value, "ClaimIR must have predicate"
        assert "object" in frag.value, "ClaimIR must have object"

    # Must produce lifted_from_map: {ir_cas_id: source_cas_id}
    lifted_from_map = state.outputs.get("lifted_from_map")
    assert isinstance(lifted_from_map, dict), f"Expected lifted_from_map dict, got {type(lifted_from_map)}"
    assert len(lifted_from_map) == len(ir_fragments), "Every IR fragment must have a source mapping"
    for ir_id, source_id in lifted_from_map.items():
        assert isinstance(ir_id, str), "IR ID must be string"
        assert isinstance(source_id, str), "Source ID must be string"

    # Metrics must report real data
    assert metrics["lifted_count"] == len(ir_fragments)
    assert "details" in metrics
    assert isinstance(metrics["details"].get("lifted_ids"), list)


def test_lift_handler_produces_reconstruction_programs():
    """Lift handler must produce one ReconstructionProgram per surface→IR mapping.

    Each program is a Fragment with RECONSTRUCTION_PROGRAM MIME type, containing
    a single "transform" strategy step. The step's inputs record:
      - source_cas_id: the surface fragment CAS ID
      - ir_cas_ids: list of IR fragment CAS IDs lifted from that surface fragment

    Programs are stored in state.outputs["lift_reconstruction_programs"].
    One program per surface fragment that was lifted (surface fragments with no
    IR output get no program).
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.fragments import Fragment
    from ikam.ir.mime_types import RECONSTRUCTION_PROGRAM

    # Real decomposition using brand-guide.md fixture
    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source,
        mime_type="text/markdown",
        artifact_id="test-lift-programs:art-1",
    )
    decomposition = decomposer.decompose(directive)
    assert len(decomposition.structural) > 0

    # Execute lift
    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-lift-programs:art-1",
        outputs={"decomposition": decomposition},
    )
    asyncio.run(execute_step("map.conceptual.normalize.discovery", state))

    # Must produce lift_reconstruction_programs
    lift_programs = state.outputs.get("lift_reconstruction_programs")
    assert isinstance(lift_programs, list), (
        f"Expected list of lift_reconstruction_programs, got {type(lift_programs)}"
    )
    assert len(lift_programs) > 0, "Lift must produce at least one reconstruction program"

    # Each program must be a Fragment with RECONSTRUCTION_PROGRAM MIME type
    for prog_frag in lift_programs:
        assert isinstance(prog_frag, Fragment), (
            f"Reconstruction program must be Fragment, got {type(prog_frag)}"
        )
        assert prog_frag.mime_type == RECONSTRUCTION_PROGRAM, (
            f"Program must have RECONSTRUCTION_PROGRAM MIME, got {prog_frag.mime_type}"
        )
        assert prog_frag.cas_id is not None, "Program fragment must have CAS ID"

        # Value must be a valid ReconstructionProgram structure
        val = prog_frag.value
        assert isinstance(val, dict), f"Program value must be dict, got {type(val)}"
        assert "steps" in val, "Program must have 'steps' key"
        steps = val["steps"]
        assert isinstance(steps, list) and len(steps) == 1, (
            f"Each lift program must have exactly one step, got {len(steps)}"
        )

        step = steps[0]
        assert step["strategy"] == "transform", (
            f"Lift reconstruction must use 'transform' strategy, got {step['strategy']}"
        )
        inputs = step.get("inputs", {})
        assert "source_cas_id" in inputs, "Transform step must record source_cas_id"
        assert "ir_cas_ids" in inputs, "Transform step must record ir_cas_ids"
        assert isinstance(inputs["ir_cas_ids"], list), "ir_cas_ids must be a list"
        assert len(inputs["ir_cas_ids"]) > 0, "Must have at least one IR fragment per program"

    # There must be one program per surface fragment that produced IR output
    lifted_from_map = state.outputs.get("lifted_from_map", {})
    # Invert: surface_id → [ir_ids]
    surface_to_ir: dict[str, list[str]] = {}
    for ir_id, surface_id in lifted_from_map.items():
        surface_to_ir.setdefault(surface_id, []).append(ir_id)

    assert len(lift_programs) == len(surface_to_ir), (
        f"Expected one program per surface fragment with IR output "
        f"({len(surface_to_ir)}), got {len(lift_programs)}"
    )

    # Verify each program's inputs match the actual lifted_from_map
    program_sources = set()
    for prog_frag in lift_programs:
        step = prog_frag.value["steps"][0]
        source_id = step["inputs"]["source_cas_id"]
        ir_ids = set(step["inputs"]["ir_cas_ids"])
        program_sources.add(source_id)

        assert source_id in surface_to_ir, (
            f"Program source_cas_id {source_id} not found in lifted_from_map"
        )
        expected_ir = set(surface_to_ir[source_id])
        assert ir_ids == expected_ir, (
            f"Program ir_cas_ids {ir_ids} don't match lifted_from_map {expected_ir}"
        )

    assert program_sources == set(surface_to_ir.keys()), (
        "Programs must cover exactly the surface fragments that produced IR output"
    )


def test_embed_handler_produces_real_dense_vectors():
    """Embed handler must use LocalFragmentEmbedder to produce real dense vectors.

    TDD RED: The current stub uses blake2b hashes, NOT real embeddings.
    The real handler must:
    - Create a LocalFragmentEmbedder (FragmentEmbedder protocol)
    - Call embedder.embed() on each IR fragment AND each surface fragment
    - Store a dict mapping fragment CAS ID → list[float] in state.outputs["embeddings"]
    - Each embedding vector must be a list of floats with consistent dimensionality
    - Return metrics with embedding_count and details.embedding_keys
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.fragments import Fragment

    # Real decomposition using brand-guide.md fixture
    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source,
        mime_type="text/markdown",
        artifact_id="test-embed:art-1",
    )
    decomposition = decomposer.decompose(directive)

    # Create minimal IR fragments for test (avoid OpenAI call for lift)
    # We simulate what the lift handler produces: Fragment objects with CLAIM_IR MIME
    from ikam.forja.cas import cas_fragment
    from ikam.ir.mime_types import CLAIM_IR

    ir_frag_1 = cas_fragment(
        {"subject": "company", "predicate": "projects", "object": "$5M ARR", "confidence": 0.9},
        CLAIM_IR,
    )
    ir_frag_2 = cas_fragment(
        {"subject": "company", "predicate": "has", "object": "200 enterprise contracts", "confidence": 0.85},
        CLAIM_IR,
    )

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-embed:art-1",
        outputs={
            "decomposition": decomposition,
            "ir_fragments": [ir_frag_1, ir_frag_2],
            "lifted_from_map": {ir_frag_1.cas_id: "src-1", ir_frag_2.cas_id: "src-2"},
        },
    )
    metrics = asyncio.run(execute_step("map.reconstructable.embed", state))

    # Must produce embeddings as a dict: {cas_id: list[float]}
    embeddings = state.outputs.get("embeddings")
    assert isinstance(embeddings, dict), f"Expected dict of embeddings, got {type(embeddings)}"

    # Must embed both IR fragments AND surface fragments
    surface_ids = {f.cas_id for f in decomposition.structural if f.cas_id}
    ir_ids = {ir_frag_1.cas_id, ir_frag_2.cas_id}
    expected_ids = surface_ids | ir_ids
    assert set(embeddings.keys()) == expected_ids, (
        f"Embeddings must cover all fragments. Missing: {expected_ids - set(embeddings.keys())}"
    )

    # Each value must be a list of floats with consistent dimensionality
    dims = None
    for frag_id, vector in embeddings.items():
        assert isinstance(vector, list), f"Embedding for {frag_id} must be list, got {type(vector)}"
        assert len(vector) > 0, f"Embedding for {frag_id} must be non-empty"
        assert all(isinstance(v, float) for v in vector), f"Embedding values must be floats"
        if dims is None:
            dims = len(vector)
        else:
            assert len(vector) == dims, f"Embedding dim mismatch: {len(vector)} vs {dims}"

    # Metrics must report real data
    assert metrics["embedding_count"] == len(expected_ids)
    assert "details" in metrics
    assert set(metrics["details"]["embedding_keys"]) == expected_ids


def test_candidate_search_handler_uses_pgvector_hnsw():
    """candidate_search must store embeddings in ikam_fragment_store and query via pgvector HNSW.

    TDD RED: The current handler does brute-force pairwise cosine similarity in
    Python without touching the database at all.  The design doc says Tier 1 =
    "Embedding similarity (cosine, HNSW) O(log N)" via pgvector.

    Discriminating assertion: after candidate_search runs, the ikam_fragment_store
    table must contain rows with populated embedding vectors — something the
    brute-force handler never produces.
    """
    import asyncio
    import psycopg

    db_url = "postgresql://narraciones:narraciones@localhost:55432/ikam_perf_report"

    # Check DB connectivity — skip if unavailable
    try:
        with psycopg.connect(db_url, connect_timeout=3) as conn:
            conn.execute("SELECT 1")
    except Exception:
        pytest.skip("pgvector Postgres not available on port 55432")

    from ikam.forja.debug_execution import execute_step, StepExecutionState

    # Build 768-dim embeddings that match what LocalFragmentEmbedder produces
    import random
    rng = random.Random(42)
    vec_a = [rng.gauss(0, 1) for _ in range(768)]
    vec_b = [v + rng.gauss(0, 0.01) for v in vec_a]  # near-identical to A
    vec_c = [rng.gauss(0, 1) for _ in range(768)]     # unrelated to A

    op_id = f"test-hnsw-{rng.randint(0, 999999):06d}"
    embeddings = {
        f"{op_id}:frag-A": vec_a,
        f"{op_id}:frag-B": vec_b,
        f"{op_id}:frag-C": vec_c,
    }

    state = StepExecutionState(
        source_bytes=b"dummy",
        mime_type="text/markdown",
        artifact_id=f"test-hnsw:{op_id}",
        outputs={"embeddings": embeddings},
    )

    # Provide DATABASE_URL so the handler can find the DB
    old_env = os.environ.get("DATABASE_URL")
    os.environ["DATABASE_URL"] = db_url
    try:
        # Reset the pool so it picks up the new DATABASE_URL
        from modelado.db import reset_pool_for_pytest
        reset_pool_for_pytest()

        metrics = asyncio.run(execute_step("map.reconstructable.search.dependency_resolution", state))
    finally:
        if old_env is None:
            os.environ.pop("DATABASE_URL", None)
        else:
            os.environ["DATABASE_URL"] = old_env
        from modelado.db import reset_pool_for_pytest
        reset_pool_for_pytest()

    # === Discriminating assertions ===
    # 1) The handler must have written embeddings to ikam_fragment_store
    with psycopg.connect(db_url) as conn:
        rows = conn.execute(
            "SELECT cas_id, embedding FROM ikam_fragment_store WHERE operation_id = %s",
            (op_id,),
        ).fetchall()

    stored_cas_ids = {r[0] for r in rows}
    assert len(stored_cas_ids) >= 3, (
        f"Expected at least 3 fragment embeddings stored in DB, got {len(stored_cas_ids)}. "
        "The brute-force handler never writes to ikam_fragment_store."
    )
    for cas_id in embeddings:
        assert cas_id in stored_cas_ids, (
            f"Fragment {cas_id} not found in ikam_fragment_store — "
            "handler must persist all embeddings to DB"
        )
    # Every stored row must have a non-null embedding
    for row in rows:
        assert row[1] is not None, (
            f"Fragment {row[0]} stored without embedding vector — "
            "embedding column must be populated for HNSW"
        )

    # 2) Candidates must still be returned correctly
    candidates = state.outputs.get("candidates")
    assert isinstance(candidates, list)
    assert len(candidates) >= 1, "Must find at least 1 candidate (A-B pair)"
    pair_ids = {(c["source_id"], c["target_id"]) for c in candidates}
    pair_ids_sym = pair_ids | {(t, s) for s, t in pair_ids}
    assert (f"{op_id}:frag-A", f"{op_id}:frag-B") in pair_ids_sym

    # 3) Metrics must include tier=embedding
    for c in candidates:
        assert c["tier"] == "embedding"

    # Cleanup: remove test rows
    with psycopg.connect(db_url) as conn:
        conn.execute("DELETE FROM ikam_fragment_store WHERE operation_id = %s", (op_id,))
        conn.commit()


def test_normalize_handler_produces_real_concept_fragments():
    """Normalize handler must use SemanticNormalizer to produce CONCEPT_MIME fragments.

    TDD RED: The current stub extracts unique MIME types into a sorted string list.
    The real handler must:
    - Read state.outputs["ir_fragments"] (list of Fragment with CLAIM_IR MIME)
    - Convert claim IR values to text for SemanticNormalizer (which reads value["text"])
    - Call SemanticNormalizer.normalize() on each IR fragment → concept fragments
    - Build ReconstructionProgram objects mapping concepts back to source structure
    - Store state.outputs["normalized_fragments"] (list of Fragment with CONCEPT_MIME)
    - Store state.outputs["reconstruction_programs"] (list of Fragment with RECONSTRUCTION_PROGRAM MIME)
    - Return metrics with normalized_count, program_count, and details
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.forja.cas import cas_fragment
    from ikam.fragments import Fragment, CONCEPT_MIME
    from ikam.ir.mime_types import CLAIM_IR, RECONSTRUCTION_PROGRAM

    # Real decomposition using brand-guide.md fixture
    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-normalize:art-1",
    )
    decomposition = decomposer.decompose(directive)

    # Create IR fragments manually (avoid OpenAI call for lift step)
    ir_frag_1 = cas_fragment(
        {"subject": "company", "predicate": "projects", "object": "$5M ARR by year two", "confidence": 0.9},
        CLAIM_IR,
    )
    ir_frag_2 = cas_fragment(
        {"subject": "company", "predicate": "targets", "object": "200 enterprise contracts at $25K ACV", "confidence": 0.85},
        CLAIM_IR,
    )

    # Normalize needs: ir_fragments, candidates, decomposition (for structural fragments)
    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-normalize:art-1",
        outputs={
            "decomposition": decomposition,
            "ir_fragments": [ir_frag_1, ir_frag_2],
            "candidates": [],  # No duplicates for this test
            "lifted_from_map": {ir_frag_1.cas_id: "src-1", ir_frag_2.cas_id: "src-2"},
        },
    )
    metrics = asyncio.run(execute_step("map.reconstructable.normalize", state))

    # Must produce normalized_fragments as list of real Fragment objects with CONCEPT_MIME
    normalized = state.outputs.get("normalized_fragments")
    assert isinstance(normalized, list), (
        f"Expected list of normalized_fragments, got {type(normalized)}"
    )
    assert len(normalized) > 0, "Normalize must produce at least one concept fragment"

    for frag in normalized:
        assert isinstance(frag, Fragment), f"Normalized fragment must be Fragment, got {type(frag)}"
        assert frag.mime_type == CONCEPT_MIME, (
            f"Normalized fragment must have CONCEPT_MIME, got {frag.mime_type}"
        )
        assert frag.cas_id is not None, "Normalized fragment must have CAS ID"
        assert isinstance(frag.value, dict), f"Normalized value must be dict, got {type(frag.value)}"
        assert "concept" in frag.value, "Concept fragment must have 'concept' key"
        assert isinstance(frag.value["concept"], str), "Concept must be a string"
        assert len(frag.value["concept"]) > 0, "Concept must be non-empty"

    # Must produce reconstruction_programs as list of Fragment with RECONSTRUCTION_PROGRAM MIME
    programs = state.outputs.get("reconstruction_programs")
    assert isinstance(programs, list), (
        f"Expected list of reconstruction_programs, got {type(programs)}"
    )
    assert len(programs) > 0, "Normalize must produce at least one reconstruction program"

    for prog_frag in programs:
        assert isinstance(prog_frag, Fragment), f"Program must be Fragment, got {type(prog_frag)}"
        assert prog_frag.mime_type == RECONSTRUCTION_PROGRAM, (
            f"Program must have RECONSTRUCTION_PROGRAM MIME, got {prog_frag.mime_type}"
        )
        assert prog_frag.cas_id is not None, "Program fragment must have CAS ID"
        assert isinstance(prog_frag.value, dict), "Program value must be dict"
        assert "steps" in prog_frag.value, "Program must have 'steps' list"
        assert isinstance(prog_frag.value["steps"], list), "Program steps must be list"

    # Metrics must report real data
    assert metrics["normalized_count"] == len(normalized)
    assert metrics["program_count"] == len(programs)
    assert "details" in metrics
    assert isinstance(metrics["details"].get("normalized_ids"), list)
    assert isinstance(metrics["details"].get("program_ids"), list)


def test_normalize_handler_builds_concatenate_reconstruction_program():
    """Normalize must produce a concatenate-strategy reconstruction program.

    The reconstruction program must reference surface fragment CAS IDs
    from the decomposition in order, using the "concatenate" strategy.
    When executed via render_program(), concatenating their bytes_b64
    must reproduce the original source bytes.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.forja.cas import cas_fragment
    from ikam.fragments import Fragment
    from ikam.ir.mime_types import CLAIM_IR, RECONSTRUCTION_PROGRAM
    from ikam.ir.reconstruction import ReconstructionProgram, render_program

    # Real decomposition using brand-guide.md fixture
    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-normalize-concat:art-1",
    )
    decomposition = decomposer.decompose(directive)

    # Build IR fragments (what lift would produce)
    ir_frag = cas_fragment(
        {"subject": "company", "predicate": "projects", "object": "$5M ARR", "confidence": 0.9},
        CLAIM_IR,
    )

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-normalize-concat:art-1",
        outputs={
            "decomposition": decomposition,
            "ir_fragments": [ir_frag],
            "candidates": [],
            "lifted_from_map": {ir_frag.cas_id: "src-1"},
        },
    )
    metrics = asyncio.run(execute_step("map.reconstructable.normalize", state))

    # Must have reconstruction_programs
    programs = state.outputs.get("reconstruction_programs")
    assert isinstance(programs, list)
    assert len(programs) > 0, "Must produce at least one reconstruction program"

    # Find the concatenate program (there should be exactly one)
    concat_programs = [
        p for p in programs
        if isinstance(p.value, dict) and any(
            s.get("strategy") == "concatenate"
            for s in p.value.get("steps", [])
        )
    ]
    assert len(concat_programs) == 1, (
        f"Expected exactly 1 concatenate program, got {len(concat_programs)}. "
        f"Programs: {[p.value for p in programs]}"
    )

    concat_prog = concat_programs[0]
    prog_data = concat_prog.value
    step = prog_data["steps"][0]
    fragment_ids = step["inputs"]["fragment_ids"]

    # The fragment_ids must reference the structural fragments
    structural_ids = [f.cas_id for f in decomposition.structural]
    assert fragment_ids == structural_ids, (
        f"Concatenate program must reference structural fragment IDs in order. "
        f"Got {fragment_ids}, expected {structural_ids}"
    )

    # Execute the program to verify it reconstructs the original bytes
    store = {f.cas_id: f for f in decomposition.structural}
    program_obj = ReconstructionProgram.model_validate(prog_data)
    rendered = render_program(program_obj, store)
    assert rendered == source, (
        f"render_program(concatenate) must reconstruct original bytes. "
        f"Got {rendered!r}, expected {source!r}"
    )


def test_normalize_handler_produces_per_fragment_reconstruction_programs():
    """Normalize must produce one ReconstructionProgram per ir→normalized mapping.

    Each program is a Fragment with RECONSTRUCTION_PROGRAM MIME type, containing
    a single "transform" strategy step. The step's inputs record:
      - ir_cas_id: the IR fragment CAS ID that was normalized
      - normalized_cas_ids: list of normalized fragment CAS IDs produced from it

    Programs are stored in state.outputs["normalize_reconstruction_programs"].
    One program per IR fragment that produced normalized output.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.forja.cas import cas_fragment
    from ikam.fragments import Fragment
    from ikam.ir.mime_types import CLAIM_IR, RECONSTRUCTION_PROGRAM

    # Real decomposition using brand-guide.md fixture
    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-norm-progs:art-1",
    )
    decomposition = decomposer.decompose(directive)

    # Build IR fragments (what lift would produce)
    ir_frag = cas_fragment(
        {"subject": "company", "predicate": "projects", "object": "$5M ARR", "confidence": 0.9},
        CLAIM_IR,
    )

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-norm-progs:art-1",
        outputs={
            "decomposition": decomposition,
            "ir_fragments": [ir_frag],
            "candidates": [],
            "lifted_from_map": {ir_frag.cas_id: "src-1"},
        },
    )
    asyncio.run(execute_step("map.reconstructable.normalize", state))

    # Must produce normalize_reconstruction_programs
    norm_programs = state.outputs.get("normalize_reconstruction_programs")
    assert isinstance(norm_programs, list), (
        f"Expected list of normalize_reconstruction_programs, got {type(norm_programs)}"
    )
    assert len(norm_programs) > 0, "Normalize must produce at least one reconstruction program"

    # Each program must be a Fragment with RECONSTRUCTION_PROGRAM MIME type
    for prog_frag in norm_programs:
        assert isinstance(prog_frag, Fragment), (
            f"Reconstruction program must be Fragment, got {type(prog_frag)}"
        )
        assert prog_frag.mime_type == RECONSTRUCTION_PROGRAM, (
            f"Program must have RECONSTRUCTION_PROGRAM MIME, got {prog_frag.mime_type}"
        )
        assert prog_frag.cas_id is not None

        val = prog_frag.value
        assert isinstance(val, dict)
        assert "steps" in val
        steps = val["steps"]
        assert isinstance(steps, list) and len(steps) == 1, (
            f"Each normalize program must have exactly one step, got {len(steps)}"
        )
        step = steps[0]
        assert step["strategy"] == "transform", (
            f"Normalize reconstruction must use 'transform' strategy, got {step['strategy']}"
        )
        inputs = step.get("inputs", {})
        assert "ir_cas_id" in inputs, "Transform step must record ir_cas_id"
        assert "normalized_cas_ids" in inputs, "Transform step must record normalized_cas_ids"
        assert isinstance(inputs["normalized_cas_ids"], list)
        assert len(inputs["normalized_cas_ids"]) > 0

    # There must be one program per IR fragment that produced normalized output
    normalized_from_map = state.outputs.get("normalized_from_map", {})
    # Invert: ir_cas_id → [normalized_cas_ids]
    ir_to_normalized: dict[str, list[str]] = {}
    for norm_id, ir_id in normalized_from_map.items():
        ir_to_normalized.setdefault(ir_id, []).append(norm_id)

    assert len(norm_programs) == len(ir_to_normalized), (
        f"Expected one program per IR fragment with normalized output "
        f"({len(ir_to_normalized)}), got {len(norm_programs)}"
    )

    # Verify each program's inputs match the actual normalized_from_map
    program_irs = set()
    for prog_frag in norm_programs:
        step = prog_frag.value["steps"][0]
        ir_id = step["inputs"]["ir_cas_id"]
        norm_ids = set(step["inputs"]["normalized_cas_ids"])
        program_irs.add(ir_id)

        assert ir_id in ir_to_normalized, (
            f"Program ir_cas_id {ir_id} not found in normalized_from_map"
        )
        expected_norms = set(ir_to_normalized[ir_id])
        assert norm_ids == expected_norms, (
            f"Program normalized_cas_ids {norm_ids} don't match normalized_from_map {expected_norms}"
        )

    assert program_irs == set(ir_to_normalized.keys()), (
        "Programs must cover exactly the IR fragments that produced normalized output"
    )


def test_normalize_handler_produces_normalized_from_map():
    """normalize must produce normalized_from_map for provenance chain.

    TDD RED: The normalize handler produces normalized_fragments but does NOT
    record which normalized fragment was derived from which IR fragment.
    project_graph needs this mapping to build normalized-by edges.

    The handler must store normalized_from_map: dict mapping
    normalized CAS ID → IR CAS ID it was derived from.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.forja.cas import cas_fragment
    from ikam.ir.mime_types import CLAIM_IR

    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-norm-map:art-1",
    )
    decomposition = decomposer.decompose(directive)

    ir_frag = cas_fragment(
        {"subject": "company", "predicate": "projects", "object": "$5M ARR", "confidence": 0.9},
        CLAIM_IR,
    )

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-norm-map:art-1",
        outputs={
            "decomposition": decomposition,
            "ir_fragments": [ir_frag],
            "candidates": [],
            "lifted_from_map": {ir_frag.cas_id: "src-1"},
        },
    )
    metrics = asyncio.run(execute_step("map.reconstructable.normalize", state))

    # Must produce normalized_from_map
    normalized_from_map = state.outputs.get("normalized_from_map")
    assert isinstance(normalized_from_map, dict), (
        f"normalize must produce normalized_from_map dict, got {type(normalized_from_map)}"
    )

    # Every normalized fragment must be in the map
    normalized = state.outputs.get("normalized_fragments", [])
    for norm_frag in normalized:
        nid = getattr(norm_frag, "cas_id", None)
        if nid:
            assert nid in normalized_from_map, (
                f"Normalized fragment {nid} not in normalized_from_map"
            )
            # The target must be the IR fragment it was derived from
            assert normalized_from_map[nid] == ir_frag.cas_id, (
                f"normalized_from_map[{nid}] must point to IR fragment {ir_frag.cas_id}, "
                f"got {normalized_from_map[nid]}"
            )


def test_compose_proposal_handler_assembles_real_commit_proposal():
    """compose_proposal handler must assemble a structured commit proposal.

    TDD RED: The current stub reads "normalized" as a list of strings (MIME types)
    and builds a static dict. The real handler must:
    - Read state.outputs["normalized_fragments"] (list of CONCEPT_MIME Fragment objects)
    - Read state.outputs["reconstruction_programs"] (list of RECONSTRUCTION_PROGRAM Fragment objects)
    - Read state.outputs["decomposition"] for surface fragments
    - Read state.outputs["ir_fragments"] for provenance
    - Assemble a proposal dict with:
      - commit_mode: "normalized" (Path A) when programs exist
      - ir_fragment_ids: CAS IDs of all IR fragments
      - normalized_fragment_ids: CAS IDs of all concept fragments
      - program_ids: CAS IDs of reconstruction programs
      - surface_fragment_ids: CAS IDs of surface fragments (for verification)
    - Store in state.outputs["proposal"]
    - Return metrics with proposal details
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.forja.cas import cas_fragment
    from ikam.fragments import Fragment, CONCEPT_MIME
    from ikam.ir.mime_types import CLAIM_IR, RECONSTRUCTION_PROGRAM
    from ikam.ir.reconstruction import ReconstructionProgram, CompositionStep, program_to_fragment

    # Real decomposition using brand-guide.md fixture
    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-compose:art-1",
    )
    decomposition = decomposer.decompose(directive)

    # Create IR fragments (what lift would produce)
    ir_frag = cas_fragment(
        {"subject": "company", "predicate": "projects", "object": "$5M ARR", "confidence": 0.9},
        CLAIM_IR,
    )

    # Create normalized concept fragments (what normalize would produce)
    concept_1 = cas_fragment(
        {"concept": "revenue projection", "mode": "explore-fast", "policy_version": "2026-02-10"},
        CONCEPT_MIME,
    )
    concept_2 = cas_fragment(
        {"concept": "annual recurring revenue", "mode": "explore-fast", "policy_version": "2026-02-10"},
        CONCEPT_MIME,
    )

    # Create reconstruction program (what normalize would produce)
    program = ReconstructionProgram(
        steps=[CompositionStep(
            strategy="instantiate",
            inputs={"source_ir_id": ir_frag.cas_id, "concept_ids": [concept_1.cas_id, concept_2.cas_id]},
        )],
        output_mime_type=CLAIM_IR,
    )
    program_frag = program_to_fragment(program)

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-compose:art-1",
        outputs={
            "decomposition": decomposition,
            "ir_fragments": [ir_frag],
            "normalized_fragments": [concept_1, concept_2],
            "reconstruction_programs": [program_frag],
            "candidates": [],
        },
    )
    metrics = asyncio.run(execute_step("map.reconstructable.compose.reconstruction_programs", state))

    # Must produce a proposal dict
    proposal = state.outputs.get("proposal")
    assert isinstance(proposal, dict), f"Expected proposal dict, got {type(proposal)}"

    # Proposal must declare commit mode
    assert proposal["commit_mode"] == "normalized", (
        f"With programs present, commit_mode must be 'normalized', got {proposal['commit_mode']}"
    )

    # Proposal must list all fragment IDs for the commit
    assert isinstance(proposal["ir_fragment_ids"], list)
    assert ir_frag.cas_id in proposal["ir_fragment_ids"]

    assert isinstance(proposal["normalized_fragment_ids"], list)
    assert concept_1.cas_id in proposal["normalized_fragment_ids"]
    assert concept_2.cas_id in proposal["normalized_fragment_ids"]

    assert isinstance(proposal["program_ids"], list)
    assert program_frag.cas_id in proposal["program_ids"]

    # Surface fragment IDs must be present (for verification and Path B fallback)
    assert isinstance(proposal["surface_fragment_ids"], list)
    assert len(proposal["surface_fragment_ids"]) > 0

    # Metrics must report proposal readiness
    assert metrics["proposal_ready"] is True
    assert "details" in metrics
    assert metrics["details"]["commit_mode"] == "normalized"
    assert isinstance(metrics["details"]["fragment_count"], int)


def test_compose_proposal_includes_lift_and_normalize_program_ids():
    """compose_proposal must include lift and normalize program IDs in the proposal.

    When lift_reconstruction_programs and normalize_reconstruction_programs
    are present in state.outputs, compose_proposal must:
    - Extract CAS IDs from lift_reconstruction_programs → proposal["lift_program_ids"]
    - Extract CAS IDs from normalize_reconstruction_programs → proposal["normalize_program_ids"]
    - Include these in the total_fragments count
    - Report lift_program_count and normalize_program_count in metrics details

    When these outputs are absent (backward compat), keys must default to empty lists.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.forja.cas import cas_fragment
    from ikam.fragments import Fragment, CONCEPT_MIME
    from ikam.ir.mime_types import CLAIM_IR
    from ikam.ir.reconstruction import (
        ReconstructionProgram, CompositionStep, program_to_fragment,
    )

    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-compose-c15:art-1",
    )
    decomposition = decomposer.decompose(directive)

    # IR fragment (from lift)
    ir_frag = cas_fragment(
        {"subject": "company", "predicate": "projects", "object": "$5M ARR", "confidence": 0.9},
        CLAIM_IR,
    )

    # Normalized concept (from normalize)
    concept = cas_fragment(
        {"concept": "revenue projection", "mode": "explore-fast", "policy_version": "2026-02-10"},
        CONCEPT_MIME,
    )

    # Concatenate reconstruction program (from normalize — byte reconstruction)
    structural_ids = [f.cas_id for f in decomposition.structural]
    concat_program = ReconstructionProgram(
        steps=[CompositionStep(strategy="concatenate", inputs={"fragment_ids": structural_ids})],
        output_mime_type="text/markdown",
    )
    concat_frag = program_to_fragment(concat_program)

    # Lift reconstruction program (surface→IR transform)
    lift_program = ReconstructionProgram(
        steps=[CompositionStep(
            strategy="transform",
            inputs={"source_cas_id": "surface-001", "ir_cas_ids": ["ir-001"]},
        )],
        output_mime_type="text/markdown",
    )
    lift_frag = program_to_fragment(lift_program)

    # Normalize reconstruction program (IR→normalized transform)
    norm_program = ReconstructionProgram(
        steps=[CompositionStep(
            strategy="transform",
            inputs={"ir_cas_id": "ir-001", "normalized_cas_ids": ["norm-001"]},
        )],
        output_mime_type="text/markdown",
    )
    norm_frag = program_to_fragment(norm_program)

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-compose-c15:art-1",
        outputs={
            "decomposition": decomposition,
            "ir_fragments": [ir_frag],
            "normalized_fragments": [concept],
            "reconstruction_programs": [concat_frag],
            "lift_reconstruction_programs": [lift_frag],
            "normalize_reconstruction_programs": [norm_frag],
            "candidates": [],
        },
    )
    metrics = asyncio.run(execute_step("map.reconstructable.compose.reconstruction_programs", state))

    proposal = state.outputs.get("proposal")
    assert isinstance(proposal, dict)

    # Must include lift_program_ids
    assert "lift_program_ids" in proposal, (
        f"Proposal must include lift_program_ids, keys: {list(proposal.keys())}"
    )
    assert isinstance(proposal["lift_program_ids"], list)
    assert lift_frag.cas_id in proposal["lift_program_ids"]

    # Must include normalize_program_ids
    assert "normalize_program_ids" in proposal, (
        f"Proposal must include normalize_program_ids, keys: {list(proposal.keys())}"
    )
    assert isinstance(proposal["normalize_program_ids"], list)
    assert norm_frag.cas_id in proposal["normalize_program_ids"]

    # Concatenate program_ids must still be present
    assert concat_frag.cas_id in proposal["program_ids"]

    # Metrics details must report counts for all program categories
    details = metrics["details"]
    assert details["lift_program_count"] == 1
    assert details["normalize_program_count"] == 1

    # Total fragment count must include lift + normalize programs
    # ir(1) + normalized(1) + concat_programs(1) + surface(N) + lift_programs(1) + norm_programs(1)
    surface_count = len(decomposition.structural)
    expected_total = 1 + 1 + 1 + surface_count + 1 + 1
    assert details["fragment_count"] == expected_total, (
        f"Expected {expected_total} total fragments, got {details['fragment_count']}"
    )


def test_compose_proposal_backward_compat_without_lift_normalize_programs():
    """compose_proposal must work when lift/normalize programs are absent.

    The existing test does not provide lift or normalize programs.
    When absent, lift_program_ids and normalize_program_ids must be empty lists.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.forja.cas import cas_fragment
    from ikam.fragments import CONCEPT_MIME
    from ikam.ir.mime_types import CLAIM_IR
    from ikam.ir.reconstruction import ReconstructionProgram, CompositionStep, program_to_fragment

    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-compose-compat:art-1",
    )
    decomposition = decomposer.decompose(directive)

    ir_frag = cas_fragment(
        {"subject": "company", "predicate": "projects", "object": "$5M ARR", "confidence": 0.9},
        CLAIM_IR,
    )
    concept = cas_fragment(
        {"concept": "revenue projection", "mode": "explore-fast", "policy_version": "2026-02-10"},
        CONCEPT_MIME,
    )
    program = ReconstructionProgram(
        steps=[CompositionStep(strategy="instantiate", inputs={"source_ir_id": ir_frag.cas_id})],
        output_mime_type=CLAIM_IR,
    )
    program_frag = program_to_fragment(program)

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-compose-compat:art-1",
        outputs={
            "decomposition": decomposition,
            "ir_fragments": [ir_frag],
            "normalized_fragments": [concept],
            "reconstruction_programs": [program_frag],
            # NO lift_reconstruction_programs or normalize_reconstruction_programs
            "candidates": [],
        },
    )
    metrics = asyncio.run(execute_step("map.reconstructable.compose.reconstruction_programs", state))

    proposal = state.outputs.get("proposal")
    assert isinstance(proposal, dict)

    # lift_program_ids and normalize_program_ids must be empty lists
    assert proposal.get("lift_program_ids") == [], (
        f"Expected empty lift_program_ids, got {proposal.get('lift_program_ids')}"
    )
    assert proposal.get("normalize_program_ids") == [], (
        f"Expected empty normalize_program_ids, got {proposal.get('normalize_program_ids')}"
    )

    # Existing keys must still work
    assert proposal["commit_mode"] == "normalized"
    assert program_frag.cas_id in proposal["program_ids"]

    # Counts must be 0 for lift/normalize programs
    details = metrics["details"]
    assert details["lift_program_count"] == 0
    assert details["normalize_program_count"] == 0


def test_verify_handler_uses_real_byte_identity_verification():
    """verify handler must use ByteIdentityVerifier for BLAKE3 exact-byte check.

    TDD RED: The current stub always returns passed=True with a static
    "deterministic-pass" policy. The real handler must:
    - Read state.outputs["proposal"] for the commit proposal
    - Read state.source_bytes for the original artifact
    - Reconstruct bytes from decomposition.root_fragments via reconstruct_binary()
    - Use ByteIdentityVerifier + DriftSpec("byte-identity") to compare
    - Store VerificationResult Fragment in state.outputs["verification"]
    - Return metrics with passed status, drift measurement, and details
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.ir.mime_types import VERIFICATION_RESULT
    from ikam.ir.reconstruction import ReconstructionProgram, CompositionStep, program_to_fragment

    # Real decomposition that preserves canonical bytes for reconstruction
    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-verify:art-1",
    )
    decomposition = decomposer.decompose(directive)

    # Build concatenate program from structural fragments (what normalize produces)
    structural_ids = [f.cas_id for f in decomposition.structural]
    program = ReconstructionProgram(
        steps=[CompositionStep(strategy="concatenate", inputs={"fragment_ids": structural_ids})],
        output_mime_type="text/markdown",
    )
    program_frag = program_to_fragment(program)

    # Minimal proposal (verify doesn't need all fields, just needs context)
    proposal = {
        "commit_mode": "normalized",
        "ir_fragment_ids": [],
        "normalized_fragment_ids": [],
        "program_ids": [],
        "surface_fragment_ids": [],
    }

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-verify:art-1",
        outputs={
            "decomposition": decomposition,
            "reconstruction_programs": [program_frag],
            "proposal": proposal,
        },
    )
    metrics = asyncio.run(execute_step("map.conceptual.verify.discovery_gate", state))

    # Must produce a verification result
    verification = state.outputs.get("verification")
    assert isinstance(verification, dict), f"Expected verification dict, got {type(verification)}"

    # The verification_result_fragment must be a real Fragment with VERIFICATION_RESULT MIME
    vr_frag = state.outputs.get("verification_result_fragment")
    assert vr_frag is not None, "Must store verification_result_fragment"
    assert getattr(vr_frag, "mime_type", None) == VERIFICATION_RESULT
    assert getattr(vr_frag, "cas_id", None) is not None, "Verification fragment must have CAS ID"

    # With matching source bytes, verification MUST pass
    vr_value = getattr(vr_frag, "value", {})
    assert vr_value["passed"] is True, f"Byte-identity check should pass for round-trip, got: {vr_value}"
    assert vr_value["measured_drift"] == 0.0
    assert vr_value["drift_spec"]["metric"] == "byte-identity"

    # Metrics must reflect the real verification
    assert metrics["passed"] is True
    assert "details" in metrics
    assert metrics["details"]["metric"] == "byte-identity"
    assert isinstance(metrics["details"]["measured_drift"], float)
    assert metrics["details"]["measured_drift"] == 0.0


def test_verify_handler_detects_byte_mismatch():
    """verify handler must detect when reconstructed bytes don't match original.

    Uses a decomposition whose canonical bytes differ from state.source_bytes
    to confirm the verifier produces passed=False with drift=1.0.
    """
    import asyncio
    import base64
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam.forja.cas import cas_fragment
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionResult
    from ikam.fragments import Fragment, Relation, BindingGroup, SlotBinding, RELATION_MIME
    from ikam.ir.reconstruction import ReconstructionProgram, CompositionStep, program_to_fragment

    # Original source bytes
    source = b"# Original content"

    # Build a FAKE decomposition whose canonical bytes are DIFFERENT
    fake_canonical = cas_fragment(
        {"bytes_b64": base64.b64encode(b"# Tampered content").decode()},
        "text/markdown",
    )
    root_relation_value = Relation(
        predicate="decomposes",
        binding_groups=[BindingGroup(
            invocation_id="inv-1",
            slots=[SlotBinding(slot="canonical", fragment_id=fake_canonical.cas_id)],
        )],
    )
    root_relation_frag = cas_fragment(root_relation_value.model_dump(), RELATION_MIME)

    fake_decomposition = DecompositionResult(
        canonical=fake_canonical,
        structural=[fake_canonical],
        root_fragments=[root_relation_frag, fake_canonical],
    )

    # Build reconstruction program over the TAMPERED structural fragments
    # render_program(concatenate) will produce b"# Tampered content" != source
    program = ReconstructionProgram(
        steps=[CompositionStep(
            strategy="concatenate",
            inputs={"fragment_ids": [fake_canonical.cas_id]},
        )],
        output_mime_type="text/markdown",
    )
    program_frag = program_to_fragment(program)

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-verify-fail:art-1",
        outputs={
            "decomposition": fake_decomposition,
            "reconstruction_programs": [program_frag],
            "proposal": {"commit_mode": "normalized"},
        },
    )
    metrics = asyncio.run(execute_step("map.conceptual.verify.discovery_gate", state))

    # Verification must FAIL — bytes don't match
    verification = state.outputs.get("verification")
    assert verification["passed"] is False

    vr_frag = state.outputs.get("verification_result_fragment")
    vr_value = getattr(vr_frag, "value", {})
    assert vr_value["passed"] is False
    assert vr_value["measured_drift"] == 1.0
    assert vr_value["diff_summary"] is not None

    assert metrics["passed"] is False


def test_verify_handler_uses_render_program_not_reconstruct_binary():
    """verify must re-render via render_program(), not extract canonical bytes.

    This test proves verify actually executes the reconstruction program
    by providing a decomposition whose canonical bytes are WRONG but whose
    reconstruction programs are CORRECT. If verify used reconstruct_binary()
    (tautological path), the wrong canonical bytes would make it FAIL.
    With render_program(), the correct concatenate program produces matching bytes.
    """
    import asyncio
    import base64
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective, DecompositionResult
    from ikam.forja.cas import cas_fragment
    from ikam.fragments import Fragment, Relation, BindingGroup, SlotBinding, RELATION_MIME
    from ikam.ir.reconstruction import (
        ReconstructionProgram, CompositionStep, program_to_fragment,
    )
    from ikam.ir.mime_types import VERIFICATION_RESULT

    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-verify-render:art-1",
    )
    real_decomposition = decomposer.decompose(directive)

    # Build concatenate program from REAL structural fragments — this is CORRECT
    structural_ids = [f.cas_id for f in real_decomposition.structural]
    program = ReconstructionProgram(
        steps=[CompositionStep(strategy="concatenate", inputs={"fragment_ids": structural_ids})],
        output_mime_type="text/markdown",
    )
    program_frag = program_to_fragment(program)

    # Build a FAKE decomposition with WRONG canonical bytes but REAL structural fragments.
    # reconstruct_binary() would return "WRONG BYTES" (fail).
    # render_program() on the concatenate program uses structural fragments (pass).
    fake_canonical = cas_fragment(
        {"bytes_b64": base64.b64encode(b"WRONG BYTES").decode()},
        "text/markdown",
    )
    root_relation_value = Relation(
        predicate="decomposes",
        binding_groups=[BindingGroup(
            invocation_id="inv-1",
            slots=[
                SlotBinding(slot="canonical", fragment_id=fake_canonical.cas_id),
                *[SlotBinding(slot="part", fragment_id=sid) for sid in structural_ids],
            ],
        )],
    )
    root_frag = cas_fragment(root_relation_value.model_dump(), RELATION_MIME)
    fake_decomposition = DecompositionResult(
        canonical=fake_canonical,
        structural=real_decomposition.structural,  # REAL structural fragments
        root_fragments=[root_frag, fake_canonical, *real_decomposition.structural],
    )

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-verify-render:art-1",
        outputs={
            "decomposition": fake_decomposition,
            "reconstruction_programs": [program_frag],
            "proposal": {"commit_mode": "normalized"},
        },
    )
    metrics = asyncio.run(execute_step("map.conceptual.verify.discovery_gate", state))

    # Must PASS — render_program(concatenate) reconstructs exact bytes from structural
    # If verify used reconstruct_binary(), it would FAIL (wrong canonical)
    assert metrics["passed"] is True, (
        "verify should use render_program() (pass), not reconstruct_binary() (would fail). "
        f"Got metrics: {metrics}"
    )
    assert metrics["details"]["metric"] == "byte-identity"
    assert metrics["details"]["measured_drift"] == 0.0

    vr_frag = state.outputs.get("verification_result_fragment")
    assert vr_frag is not None
    assert getattr(vr_frag, "mime_type", None) == VERIFICATION_RESULT
    assert vr_frag.value["passed"] is True


def test_verify_handler_records_full_reconstruction_chain():
    """Verify handler must record the full reconstruction chain for audit.

    When lift_reconstruction_programs and normalize_reconstruction_programs
    are present in state.outputs, the verify handler must store them in
    state.outputs["verification"]["reconstruction_chain"] with keys:
      - lift_programs: list of CAS IDs from lift_reconstruction_programs
      - normalize_programs: list of CAS IDs from normalize_reconstruction_programs
      - concatenate_programs: list of CAS IDs from reconstruction_programs

    The byte-identity check still uses only the concatenate programs.
    The chain is recorded for debug/audit traceability of the full
    surface → IR → normalized → surface bytes reconstruction path.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.ir.reconstruction import (
        ReconstructionProgram, CompositionStep, program_to_fragment,
    )

    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-verify-chain:art-1",
    )
    decomposition = decomposer.decompose(directive)

    # Build concatenate program (what normalize produces for byte reconstruction)
    structural_ids = [f.cas_id for f in decomposition.structural]
    concat_program = ReconstructionProgram(
        steps=[CompositionStep(strategy="concatenate", inputs={"fragment_ids": structural_ids})],
        output_mime_type="text/markdown",
    )
    concat_frag = program_to_fragment(concat_program)

    # Build a lift reconstruction program (surface→IR transform)
    lift_program = ReconstructionProgram(
        steps=[CompositionStep(
            strategy="transform",
            inputs={"source_cas_id": "surface-001", "ir_cas_ids": ["ir-001", "ir-002"]},
        )],
        output_mime_type="text/markdown",
    )
    lift_frag = program_to_fragment(lift_program)

    # Build a normalize reconstruction program (IR→normalized transform)
    norm_program = ReconstructionProgram(
        steps=[CompositionStep(
            strategy="transform",
            inputs={"ir_cas_id": "ir-001", "normalized_cas_ids": ["norm-001"]},
        )],
        output_mime_type="text/markdown",
    )
    norm_frag = program_to_fragment(norm_program)

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-verify-chain:art-1",
        outputs={
            "decomposition": decomposition,
            "reconstruction_programs": [concat_frag],
            "lift_reconstruction_programs": [lift_frag],
            "normalize_reconstruction_programs": [norm_frag],
            "proposal": {"commit_mode": "normalized"},
        },
    )
    metrics = asyncio.run(execute_step("map.conceptual.verify.discovery_gate", state))

    # Byte-identity check must still pass (uses concatenate program)
    assert metrics["passed"] is True

    # Verification output must include the full reconstruction chain
    verification = state.outputs.get("verification")
    assert isinstance(verification, dict)
    chain = verification.get("reconstruction_chain")
    assert isinstance(chain, dict), (
        f"Expected reconstruction_chain dict in verification output, got {type(chain)}"
    )

    # Chain must have all three program categories
    assert "lift_programs" in chain, "Chain must include lift_programs"
    assert "normalize_programs" in chain, "Chain must include normalize_programs"
    assert "concatenate_programs" in chain, "Chain must include concatenate_programs"

    # Each entry must be a list of CAS IDs (strings)
    assert isinstance(chain["lift_programs"], list)
    assert len(chain["lift_programs"]) == 1
    assert chain["lift_programs"][0] == lift_frag.cas_id

    assert isinstance(chain["normalize_programs"], list)
    assert len(chain["normalize_programs"]) == 1
    assert chain["normalize_programs"][0] == norm_frag.cas_id

    assert isinstance(chain["concatenate_programs"], list)
    assert len(chain["concatenate_programs"]) == 1
    assert chain["concatenate_programs"][0] == concat_frag.cas_id

    # Details in metrics should also report chain presence
    assert metrics["details"].get("reconstruction_chain_recorded") is True


def test_promote_commit_handler_path_a_normalized():
    """promote_commit handler must commit IR + programs + verification when Path A.

    TDD RED: The current stub only sets commit_mode and verification_passed.
    The real handler must:
    - When verification passed: Path A (normalized) — commit IR fragments,
      reconstruction programs, and verification result fragment
    - Collect all committed CAS IDs in the commit record
    - Store structured commit in state.outputs["commit"]
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam.forja.cas import cas_fragment
    from ikam.forja.verifier import make_verification_result_fragment, DriftSpec
    from ikam.fragments import CONCEPT_MIME
    from ikam.ir.mime_types import CLAIM_IR, RECONSTRUCTION_PROGRAM, VERIFICATION_RESULT

    # Simulate upstream outputs for a successful pipeline
    ir_frag = cas_fragment(
        {"subject": "company", "predicate": "projects", "object": "$5M ARR"},
        CLAIM_IR,
    )
    concept_frag = cas_fragment(
        {"concept": "revenue projection"},
        CONCEPT_MIME,
    )
    program_frag = cas_fragment(
        {"steps": [{"strategy": "instantiate", "inputs": {}}], "output_mime_type": CLAIM_IR},
        RECONSTRUCTION_PROGRAM,
    )
    vr_frag = make_verification_result_fragment(
        drift_spec=DriftSpec(metric="byte-identity"),
        measured_drift=0.0,
        passed=True,
        renderer_version="1.0.0",
    )

    proposal = {
        "commit_mode": "normalized",
        "ir_fragment_ids": [ir_frag.cas_id],
        "normalized_fragment_ids": [concept_frag.cas_id],
        "program_ids": [program_frag.cas_id],
        "surface_fragment_ids": ["surface-1", "surface-2"],
    }

    state = StepExecutionState(
        source_bytes=b"test",
        mime_type="text/markdown",
        artifact_id="test-promote:art-1",
        outputs={
            "verification": {"passed": True, "measured_drift": 0.0},
            "verification_result_fragment": vr_frag,
            "proposal": proposal,
        },
    )
    metrics = asyncio.run(execute_step("map.conceptual.commit.semantic_only", state))

    commit = state.outputs.get("commit")
    assert isinstance(commit, dict), f"Expected commit dict, got {type(commit)}"

    # Path A: commit_mode must be "normalized"
    assert commit["mode"] == "normalized"

    # Must list all committed fragment CAS IDs
    promoted_ids = commit["promoted_fragment_ids"]
    assert commit["target_ref"] == "refs/heads/main"
    assert ir_frag.cas_id in promoted_ids
    assert concept_frag.cas_id in promoted_ids
    assert program_frag.cas_id in promoted_ids
    assert vr_frag.cas_id in promoted_ids

    # Surface fragments should NOT be in promoted output (Path A: they're reconstructable)
    assert "surface-1" not in promoted_ids
    assert "surface-2" not in promoted_ids

    # Metrics
    assert metrics["commit_mode"] == "normalized"
    assert "details" in metrics
    assert metrics["details"]["committed_count"] == len(promoted_ids)
    assert metrics["details"]["verification_passed"] is True


def test_promote_commit_handler_path_b_surface_only():
    """promote_commit handler must commit surface fragments when Path B (verification failed)."""
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState

    proposal = {
        "commit_mode": "normalized",
        "ir_fragment_ids": ["ir-1"],
        "normalized_fragment_ids": ["concept-1"],
        "program_ids": ["prog-1"],
        "surface_fragment_ids": ["surface-1", "surface-2"],
    }

    state = StepExecutionState(
        source_bytes=b"test",
        mime_type="text/markdown",
        artifact_id="test-promote-b:art-1",
        outputs={
            "verification": {"passed": False, "measured_drift": 1.0},
            "proposal": proposal,
        },
    )
    metrics = asyncio.run(execute_step("map.conceptual.commit.semantic_only", state))

    commit = state.outputs.get("commit")
    assert commit["mode"] == "surface_only"

    # Path B: only surface fragments are committed
    promoted_ids = commit["promoted_fragment_ids"]
    assert commit["target_ref"] == "refs/heads/main"
    assert "surface-1" in promoted_ids
    assert "surface-2" in promoted_ids
    # IR/concepts/programs should NOT be committed
    assert "ir-1" not in promoted_ids
    assert "concept-1" not in promoted_ids
    assert "prog-1" not in promoted_ids

    assert metrics["commit_mode"] == "surface_only"
    assert metrics["details"]["verification_passed"] is False


def test_decompose_emits_edges_and_graph_projection():
    """decompose must emit artifact→surface_fragment edges and a graph_projection.

    After decompose, state.outputs["edges"] must contain one edge per
    structural fragment: {source: artifact_id, target: frag_cas_id,
    predicate: "contains", step: "map.conceptual.lift.surface_fragments"}.

    state.outputs["graph_projection"] must contain nodes (artifact + surface
    fragments) and the accumulated edges.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective

    source = _brand_guide_source()
    register_defaults()

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-edges-decompose:art-1",
        outputs={},
    )
    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))

    # Must have edges
    edges = state.outputs.get("edges")
    assert isinstance(edges, list), f"Expected edges list, got {type(edges)}"
    assert len(edges) > 0, "decompose must emit at least one edge"

    # Each edge: artifact → surface_fragment with "contains" predicate
    # Plus one "original-bytes-of" edge from CAS(source_bytes) → artifact
    decomposition = state.outputs["decomposition"]
    surface_ids = [f.cas_id for f in decomposition.structural if f.cas_id]
    contains_edges = [e for e in edges if e["predicate"] == "contains"]
    ob_edges = [e for e in edges if e["predicate"] == "original-bytes-of"]
    assert len(contains_edges) == len(surface_ids), (
        f"Expected {len(surface_ids)} 'contains' edges, got {len(contains_edges)}"
    )
    assert len(ob_edges) == 1, (
        f"Expected 1 'original-bytes-of' edge, got {len(ob_edges)}"
    )
    assert len(edges) == len(surface_ids) + 1, (
        f"Expected {len(surface_ids) + 1} total edges, got {len(edges)}"
    )
    for edge in contains_edges:
        assert edge["source"] == "test-edges-decompose:art-1"
        assert edge["target"] in surface_ids
        assert edge["predicate"] == "contains"
        assert edge["step"] == "map.conceptual.lift.surface_fragments"

    # Must have graph_projection
    gp = state.outputs.get("graph_projection")
    assert isinstance(gp, dict), f"Expected graph_projection dict, got {type(gp)}"
    assert isinstance(gp["nodes"], list)
    assert isinstance(gp["edges"], list)
    # Nodes: 1 artifact + N surface fragments + 1 original-bytes CAS fragment
    assert gp["node_count"] == 1 + len(surface_ids) + 1
    assert gp["edge_count"] == len(edges)


def test_map_infers_docx_mime_from_filename_when_asset_is_octet_stream() -> None:
    import asyncio
    from pathlib import Path

    from ikam.forja.debug_execution import StepExecutionState, execute_step

    fixture = (
        Path(__file__).resolve().parents[4]
        / "tests"
        / "fixtures"
        / "cases"
        / "s-local-retail-v01"
        / "holiday-recap-2025.docx"
    )
    assert fixture.exists(), f"Missing DOCX fixture: {fixture}"

    payload = fixture.read_bytes()
    state = StepExecutionState(
        source_bytes=payload,
        mime_type="application/octet-stream",
        artifact_id="test-map-mime:docx-asset",
        assets=[
            {
                "artifact_id": "asset-docx-1",
                "filename": fixture.name,
                "mime_type": "application/octet-stream",
                "payload": payload,
            }
        ],
        outputs={},
    )

    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))

    statuses = state.outputs.get("asset_decomposition_statuses") or []
    assert len(statuses) == 1
    row = statuses[0]
    assert row["artifact_id"] == "asset-docx-1"
    assert row["mime_type"] == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    assert row["status"] == "fallback"
    assert row["reason"] == "semantic_map_segments"
    map_subgraph = state.outputs.get("map_subgraph")
    assert isinstance(map_subgraph, dict)
    assert isinstance(map_subgraph.get("root_node_id"), str)


def test_map_outputs_structural_map_and_map_dna() -> None:
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState

    source = b"# Title\n\nParagraph one.\n\nParagraph two."
    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-map-structural:art-1",
        outputs={},
    )

    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))

    structural_map = state.outputs.get("structural_map")
    map_dna = state.outputs.get("map_dna")
    outline_nodes = state.outputs.get("map_outline_nodes")
    root_node_id = state.outputs.get("map_root_node_id")
    node_summaries = state.outputs.get("map_node_summaries")
    node_constituents = state.outputs.get("map_node_constituents")
    segment_anchors = state.outputs.get("map_segment_anchors")
    segment_candidates = state.outputs.get("map_segment_candidates")
    profile_candidates = state.outputs.get("map_profile_candidates")

    assert isinstance(structural_map, dict)
    assert isinstance(map_dna, dict)
    assert isinstance(outline_nodes, list)
    assert isinstance(root_node_id, str)
    assert isinstance(map_dna.get("fingerprint"), str)
    assert isinstance(node_summaries, dict)
    assert isinstance(node_constituents, dict)
    assert isinstance(segment_anchors, dict)
    assert isinstance(segment_candidates, list)
    assert isinstance(profile_candidates, dict)
    assert root_node_id in node_summaries
    assert isinstance(node_summaries[root_node_id], str)
    assert node_summaries[root_node_id].strip()
    assert root_node_id in node_constituents
    assert isinstance(node_constituents[root_node_id], list)
    assert all(isinstance(item, str) for item in node_constituents[root_node_id])


def test_map_emits_runtime_chunking_milestone_logs(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    import asyncio
    from ikam.forja import debug_execution
    from ikam.forja.debug_execution import execute_step, StepExecutionState

    monkeypatch.setattr(
        debug_execution,
        "_load_parse_review_runner",
        lambda: lambda _payload: {"decision": "accept", "confidence": 1.0},
        raising=False,
    )

    class ScopeWithChunkOperator:
        def get_available_tools(self) -> list[dict[str, object]]:
            return []

        def get_step_execution_metadata(self, step_name: str) -> dict[str, object]:
            assert step_name == "map.conceptual.lift.surface_fragments"
            return {
                "implementation_step_name": "map.conceptual.lift.surface_fragments",
                "operator_id": "modelado/operators/chunking",
            }

    source = b"# Title\n\nParagraph one.\n\nParagraph two."
    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-map-runtime-logs:art-1",
        outputs={},
    )

    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state, scope=ScopeWithChunkOperator()))

    captured = capsys.readouterr()
    stdout_lines = [line for line in captured.out.splitlines() if line.strip()]
    assert all(line.startswith("[") and "Z] " in line for line in stdout_lines)
    assert any("parse.chunk: mapping_mode=" in line for line in stdout_lines)
    assert any("parse.chunk: phase=asset_intake finished assets=" in line for line in stdout_lines)
    assert any(
        "parse.chunk: asset filename=" in line and "source=primary" in line
        for line in stdout_lines
    )
    assert any(
        "parse.chunk: branch=python_native_chunking" in line and "chunk_execution_local=true" in line
        for line in stdout_lines
    )
    assert any(
        "parse.chunk: framework branch=python_native_chunking" in line
        and "framework=modelado" in line
        and "operation_method=ChunkOperator.apply" in line
        for line in stdout_lines
    )
    assert any(
        "parse.chunk: phase=chunking started documents=" in line
        and "assets=" in line
        for line in stdout_lines
    )
    assert any(
        "parse.chunk: phase=chunking finished chunk_count=" in line
        and "duration_ms=" in line
        for line in stdout_lines
    )
    assert any(
        "parse.chunk: chunk_count=" in line
        for line in stdout_lines
    )
    assert any(
        "parse.chunk: asset_chunks filename=" in line
        and "segments=" in line
        and "max_chars=" in line
        and "total_chars=" in line
        for line in stdout_lines
    )
    assert any("parse.chunk: summary assets=" in line and "total_duration_ms=" in line for line in stdout_lines)


def test_map_emits_agentic_chunker_path_logs(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    import asyncio
    import sys
    import types
    from ikam.forja import debug_execution
    from ikam.forja.debug_execution import execute_step, StepExecutionState

    def _fake_map(*_args: object, **_kwargs: object) -> dict[str, object]:
        return {
            "selected_tool_id": "tool.semantic_chunker",
            "generation_provenance": {
                "provider": "mcp-ikam",
                "model": "gpt-4o-mini",
                "prompt_version": "v1",
            },
        }

    monkeypatch.setattr(debug_execution, "_invoke_mcp_map_generation", _fake_map, raising=False)
    monkeypatch.setattr(
        debug_execution,
        "_load_parse_review_runner",
        lambda: lambda _payload: {"decision": "accept", "confidence": 1.0},
        raising=False,
    )

    class ScopeWithChunkOperator:
        def get_available_tools(self) -> list[dict[str, object]]:
            return []

        def get_step_execution_metadata(self, step_name: str) -> dict[str, object]:
            assert step_name == "map.conceptual.lift.surface_fragments"
            return {
                "implementation_step_name": "map.conceptual.lift.surface_fragments",
                "operator_id": "modelado/operators/chunking",
            }

    fake_chunker_module = types.ModuleType("modelado.chunking.llama_chunker")

    class FakeLosslessChunker:
        def chunk_text(self, text: str, mapping_mode: str = "semantic_relations_only") -> list[str]:
            return [part for part in text.split("\n\n") if part]

    fake_chunker_module.LosslessChunker = FakeLosslessChunker
    monkeypatch.setitem(sys.modules, "modelado.chunking.llama_chunker", fake_chunker_module)

    state = StepExecutionState(
        source_bytes=b"# Title\n\nParagraph one.\n\nParagraph two.",
        mime_type="text/markdown",
        artifact_id="test-map-agentic-logs:art-1",
        outputs={"mapping_mode": "full_preservation"},
    )

    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state, scope=ScopeWithChunkOperator()))

    captured = capsys.readouterr()
    stdout_lines = [line for line in captured.out.splitlines() if line.strip()]
    assert any(
        "parse.chunk: branch=agentic_chunker" in line
        and "tool_id=tool.semantic_chunker" in line
        and "algorithm=LosslessChunker" in line
        and "lossless_check=true" in line
        for line in stdout_lines
    )
    assert any(
        "parse.chunk: framework branch=agentic_chunker" in line
        and "framework=llama_index" in line
        and "operation_library=llama_index" in line
        and "operation_method=SemanticSplitterNodeParser.get_nodes_from_documents" in line
        and "wrapper=LosslessChunker" in line
        and "node_parser_method=SemanticSplitterNodeParser.get_nodes_from_documents" in line
        and "wrapper_method=LosslessChunker.chunk_text" in line
        for line in stdout_lines
    )
    assert any(
        "parse.chunk: asset_chunks filename=" in line
        and "segments=" in line
        and "max_chars=" in line
        and "total_chars=" in line
        for line in stdout_lines
    )


def test_parse_chunk_uses_scope_operator_metadata_for_python_native_path(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import asyncio
    from ikam.forja import debug_execution
    from ikam.forja.debug_execution import StepExecutionState, execute_step

    called = {"mcp": 0}

    def _explode_if_called(*_args: object, **_kwargs: object) -> dict[str, object]:
        called["mcp"] += 1
        raise AssertionError("MCP map generation should not run when scope selects chunking operator")

    monkeypatch.setattr(debug_execution, "_invoke_mcp_map_generation", _explode_if_called, raising=False)

    class ScopeWithChunkOperator:
        def get_available_tools(self) -> list[dict[str, object]]:
            return []

        def get_step_execution_metadata(self, step_name: str) -> dict[str, object]:
            assert step_name == "step.from.operator.fragment"
            return {
                "implementation_step_name": "step.from.operator.fragment",
                "operator_id": "modelado/operators/chunking",
            }

    state = StepExecutionState(
        source_bytes=b"# Title\n\nParagraph one.\n\nParagraph two.",
        mime_type="text/markdown",
        artifact_id="test-scope-operator-dispatch:art-1",
        outputs={},
    )

    asyncio.run(
        execute_step(
            "step.from.operator.fragment",
            state,
            scope=ScopeWithChunkOperator(),
        )
    )

    assert called["mcp"] == 0
    assert state.outputs["operation_telemetry"]["branch"] == "python_native_chunking"
    assert state.outputs["operation_telemetry"]["operation_method"] == "ChunkOperator.apply"


def test_entities_operator_telemetry_uses_scope_executor_metadata() -> None:
    import asyncio
    from ikam.forja.debug_execution import StepExecutionState, execute_step

    class ScopeWithEntitiesOperator:
        def get_step_execution_metadata(self, step_name: str) -> dict[str, object]:
            assert step_name == "step.from.entities.operator"
            return {
                "implementation_step_name": "step.from.entities.operator",
                "operator_id": "modelado/operators/entities_and_relationships",
                "executor_id": "executor://ml-primary",
                "executor_kind": "ml-executor",
            }

    state = StepExecutionState(
        source_bytes=b"# Title\n\nParagraph one.\n\nParagraph two.",
        mime_type="text/markdown",
        artifact_id="test-entities-telemetry:art-1",
        outputs={
            "chunks": [
                {
                    "fragment_id": "frag-chunk-1",
                    "chunk_id": "chunk-1",
                    "document_id": "doc-1",
                    "artifact_id": "test-entities-telemetry:art-1",
                    "filename": "brief.md",
                    "text": "Alice founded Acme.",
                    "span": {"start": 0, "end": 20},
                    "order": 0,
                }
            ],
            "fragment_ids": ["frag-chunk-1"],
            "inputs": {
                "chunk_extraction_set_ref": "hot://run/chunk_extraction_set/step-parse-chunk",
                "document_set_ref": "hot://run/document_set/step-load-documents",
            },
        },
    )

    asyncio.run(
        execute_step(
            "step.from.entities.operator",
            state,
            scope=ScopeWithEntitiesOperator(),
        )
    )

    assert state.outputs["operation_telemetry"]["executor_id"] == "executor://ml-primary"
    assert state.outputs["operation_telemetry"]["executor_kind"] == "ml-executor"
    assert state.outputs["operation_telemetry"]["operation_name"] == "step.from.entities.operator"


def test_ingestion_operator_branch_resolver_maps_scope_operator_ids() -> None:
    from ikam.forja.debug_execution import (
        _get_ingestion_branch_handler,
        _resolve_ingestion_operator_branch,
    )

    assert _resolve_ingestion_operator_branch(
        "not-a-step-name",
        {"operator_id": "modelado/operators/load_documents"},
    ) == "load_documents"
    assert _resolve_ingestion_operator_branch(
        "another-unknown-step",
        {"operator_id": "modelado/operators/chunking"},
    ) == "chunking"
    assert _resolve_ingestion_operator_branch(
        "yet-another-step",
        {"operator_id": "modelado/operators/entities_and_relationships"},
    ) == "entities_and_relationships"
    assert _resolve_ingestion_operator_branch(
        "claims-from-fragment-step",
        {"operator_id": "modelado/operators/claims"},
    ) == "claims"
    assert callable(_get_ingestion_branch_handler("load_documents"))
    assert callable(_get_ingestion_branch_handler("chunking"))
    assert callable(_get_ingestion_branch_handler("entities_and_relationships"))
    assert callable(_get_ingestion_branch_handler("claims"))


def test_load_documents_uses_scope_operator_metadata_for_branch_resolution(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    import asyncio
    from ikam.forja import debug_execution
    from ikam.forja.debug_execution import StepExecutionState, execute_step

    def _fake_load_asset(*, asset: dict[str, object], asset_index: int) -> dict[str, object]:
        return {
            "reader_key": "simple_directory_reader",
            "reader_library": "llama_index.core",
            "reader_method": "SimpleDirectoryReader.load_data",
            "status": "success",
            "documents": [
                {
                    "id": "doc-1",
                    "text": "Paragraph one.",
                    "metadata": {"file_name": "brief.md"},
                }
            ],
        }

    monkeypatch.setattr(debug_execution, "_load_single_asset_for_debug_step", _fake_load_asset, raising=False)

    class ScopeWithLoadDocumentsOperator:
        def get_step_execution_metadata(self, step_name: str) -> dict[str, object]:
            assert step_name == "parse_artifacts"
            return {
                "implementation_step_name": "parse_artifacts",
                "operator_id": "modelado/operators/load_documents",
                "executor_id": "executor://python-primary",
                "executor_kind": "python-executor",
            }

    state = StepExecutionState(
        source_bytes=b"# Title\n\nParagraph one.",
        mime_type="text/markdown",
        artifact_id="test-load-documents-branch:art-1",
        assets=[
            {
                "artifact_id": "test-load-documents-branch:brief.md",
                "filename": "brief.md",
                "mime_type": "text/markdown",
                "payload": b"# Title\n\nParagraph one.",
            }
        ],
        outputs={},
    )

    asyncio.run(execute_step("parse_artifacts", state, scope=ScopeWithLoadDocumentsOperator()))

    captured = capsys.readouterr()
    assert "load.documents: phase=asset_intake finished assets=1" in captured.out
    assert state.outputs["operation_telemetry"]["operation_name"] == "load.documents"


def test_load_documents_requires_scope_operator_metadata() -> None:
    import asyncio
    from ikam.forja.debug_execution import StepExecutionState, execute_step

    state = StepExecutionState(
        source_bytes=b"# Title\n\nParagraph one.",
        mime_type="text/markdown",
        artifact_id="test-load-documents-no-scope:art-1",
        assets=[
            {
                "artifact_id": "test-load-documents-no-scope:brief.md",
                "filename": "brief.md",
                "mime_type": "text/markdown",
                "payload": b"# Title\n\nParagraph one.",
            }
        ],
        outputs={},
    )

    with pytest.raises(RuntimeError, match="No executor configured for step: parse_artifacts"):
        asyncio.run(execute_step("parse_artifacts", state))


def test_load_documents_emits_reader_method_and_loaded_documents_logs(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    import asyncio
    from ikam.forja import debug_execution
    from ikam.forja.debug_execution import StepExecutionState, execute_step

    class ScopeWithLoadDocumentsOperator:
        def get_step_execution_metadata(self, step_name: str) -> dict[str, object]:
            assert step_name == "parse_artifacts"
            return {
                "implementation_step_name": "parse_artifacts",
                "operator_id": "modelado/operators/load_documents",
                "executor_id": "executor://python-primary",
                "executor_kind": "python-executor",
            }

    def _fake_load_asset(*, asset: dict[str, object], asset_index: int) -> dict[str, object]:
        filename = str(asset["filename"])
        if filename == "brief.md":
            return {
                "reader_key": "simple_directory_reader",
                "reader_library": "llama_index.core",
                "reader_method": "SimpleDirectoryReader.load_data",
                "status": "success",
                "documents": [
                    {
                        "id": "doc-1",
                        "text": "Title\n\nParagraph one.",
                        "metadata": {"file_name": "brief.md", "reader": "markdown"},
                    },
                    {
                        "id": "doc-2",
                        "text": "Paragraph two.",
                        "metadata": {"file_name": "brief.md", "section": "body"},
                    },
                ],
            }
        if filename == "data.json":
            return {
                "reader_key": "json_reader",
                "reader_library": "llama_index.readers.json",
                "reader_method": "JSONReader.load_data",
                "status": "success",
                "documents": [
                    {
                        "id": "doc-3",
                        "text": '{"revenue": 42}',
                        "metadata": {"file_name": "data.json", "reader": "json"},
                    }
                ],
            }
        return {
            "reader_key": "unsupported",
            "reader_library": "none",
            "reader_method": "none",
            "status": "unsupported",
            "documents": [],
        }

    monkeypatch.setattr(debug_execution, "_load_single_asset_for_debug_step", _fake_load_asset, raising=False)

    state = StepExecutionState(
        source_bytes=b"# Title\n\nParagraph one.\n\nParagraph two.",
        mime_type="text/markdown",
        artifact_id="test-load-documents-logs:art-1",
        assets=[
            {
                "artifact_id": "test-load-documents-logs:brief.md",
                "filename": "brief.md",
                "mime_type": "text/markdown",
                "payload": b"# Title\n\nParagraph one.\n\nParagraph two.",
            },
            {
                "artifact_id": "test-load-documents-logs:data.json",
                "filename": "data.json",
                "mime_type": "application/json",
                "payload": b'{"revenue": 42}',
            },
            {
                "artifact_id": "test-load-documents-logs:table.xlsx",
                "filename": "table.xlsx",
                "mime_type": "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "payload": b'PK\x03\x04',
            }
        ],
        outputs={},
    )

    asyncio.run(execute_step("parse_artifacts", state, scope=ScopeWithLoadDocumentsOperator()))

    captured = capsys.readouterr()
    stdout_lines = [line for line in captured.out.splitlines() if line.strip()]
    stderr_lines = [line for line in captured.err.splitlines() if line.strip()]
    assert all(line.startswith("[") and "Z] " in line for line in stdout_lines)
    assert all(line.startswith("[") and "Z] " in line for line in stderr_lines)
    assert any("load.documents: phase=asset_intake finished assets=3" in line for line in stdout_lines)
    assert any(
        "load.documents: asset_reader" in line
        and "filename=brief.md" in line
        and "reader_key=simple_directory_reader" in line
        and "reader_method=SimpleDirectoryReader.load_data" in line
        for line in stdout_lines
    )
    assert any(
        "load.documents: asset_reader" in line
        and "filename=data.json" in line
        and "reader_key=json_reader" in line
        and "reader_method=JSONReader.load_data" in line
        for line in stdout_lines
    )
    assert any(
        "load.documents: asset_status" in line
        and "filename=table.xlsx" in line
        and "status=unsupported" in line
        for line in stderr_lines
    )
    assert any(
        "load.documents: document" in line
        and "index=0" in line
        and "doc_id=doc-1" in line
        and "artifact_id=test-load-documents-logs:brief.md" in line
        and "filename=brief.md" in line
        for line in stdout_lines
    )
    assert any(
        "load.documents: document" in line
        and "index=1" in line
        and "doc_id=doc-2" in line
        for line in stdout_lines
    )
    assert any(
        "load.documents: document" in line
        and "index=2" in line
        and "doc_id=doc-3" in line
        and "filename=data.json" in line
        for line in stdout_lines
    )
    assert any(
        "load.documents: summary documents=3 assets=3 loaded_assets=2 errored_assets=0 unsupported_assets=1" in line
        for line in stdout_lines
    )
    assert len(state.outputs["documents"]) == 3
    assert len(state.outputs["document_loads"]) == 3


def test_load_documents_surfaces_docx_dependency_errors_in_logs_without_fallback(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    import asyncio
    from ikam.forja.debug_execution import StepExecutionState, execute_step
    from modelado.executors import loaders

    class ScopeWithLoadDocumentsOperator:
        def get_step_execution_metadata(self, step_name: str) -> dict[str, object]:
            assert step_name == "parse_artifacts"
            return {
                "implementation_step_name": "parse_artifacts",
                "operator_id": "modelado/operators/load_documents",
                "executor_id": "executor://python-primary",
                "executor_kind": "python-executor",
            }

    def _raise_docx_error(payload: dict[str, object], context: dict[str, object]):
        raise RuntimeError("docx2txt is required to read Microsoft Word files: `pip install docx2txt`")

    monkeypatch.setattr(loaders, "run", _raise_docx_error)

    state = StepExecutionState(
        source_bytes=b"PK\x03\x04",
        mime_type="application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        artifact_id="test-load-documents-docx-error:art-1",
        assets=[
            {
                "artifact_id": "test-load-documents-docx-error:docx-1",
                "filename": "brief.docx",
                "mime_type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "payload": b"PK\x03\x04",
            }
        ],
        outputs={},
    )

    asyncio.run(execute_step("parse_artifacts", state, scope=ScopeWithLoadDocumentsOperator()))

    captured = capsys.readouterr()
    stdout_lines = [line for line in captured.out.splitlines() if line.strip()]
    stderr_lines = [line for line in captured.err.splitlines() if line.strip()]
    assert any("load.documents: asset_reader" in line and "filename=brief.docx" in line for line in stdout_lines)
    assert any(
        "load.documents: asset_status" in line
        and "filename=brief.docx" in line
        and "status=error" in line
        and "docx2txt is required to read Microsoft Word files" in line
        for line in stderr_lines
    )
    assert any(
        "load.documents: summary documents=0 assets=1 loaded_assets=0 errored_assets=1 unsupported_assets=0" in line
        for line in stdout_lines
    )
    assert state.outputs["documents"] == []
    assert state.outputs["document_loads"] == [
        {
            "artifact_id": "test-load-documents-docx-error:docx-1",
            "filename": "brief.docx",
            "mime_type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "reader_key": "simple_directory_reader",
            "reader_library": "llama_index.core",
            "reader_method": "SimpleDirectoryReader.load_data",
            "status": "error",
            "document_count": 0,
            "error_message": "docx2txt is required to read Microsoft Word files: `pip install docx2txt`",
        }
    ]


def test_load_documents_normalizes_docx_octet_stream_mime(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    import asyncio
    from ikam.forja.debug_execution import StepExecutionState, execute_step
    from modelado.executors import loaders

    class ScopeWithLoadDocumentsOperator:
        def get_step_execution_metadata(self, step_name: str) -> dict[str, object]:
            assert step_name == "parse_artifacts"
            return {
                "implementation_step_name": "parse_artifacts",
                "operator_id": "modelado/operators/load_documents",
                "executor_id": "executor://python-primary",
                "executor_kind": "python-executor",
            }

    def _fake_run(payload: dict[str, object], context: dict[str, object]):
        params = payload.get("params", {}) if isinstance(payload, dict) else {}
        assert params.get("mime_type") == "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        yield {
            "type": "result",
            "status": "success",
            "result": {
                "documents": [
                    {
                        "id": "doc-docx-1",
                        "text": "Holiday recap",
                        "metadata": {"file_name": "holiday-recap-2025.docx"},
                    }
                ],
                "reader_key": "simple_directory_reader",
                "reader_library": "llama_index.core",
                "reader_method": "SimpleDirectoryReader.load_data",
            },
        }

    monkeypatch.setattr(loaders, "run", _fake_run)

    state = StepExecutionState(
        source_bytes=b"PK\x03\x04",
        mime_type="application/octet-stream",
        artifact_id="test-load-documents-docx-mime:art-1",
        assets=[
            {
                "artifact_id": "test-load-documents-docx-mime:docx-1",
                "filename": "holiday-recap-2025.docx",
                "mime_type": "application/octet-stream",
                "payload": b"PK\x03\x04",
            }
        ],
        outputs={},
    )

    asyncio.run(execute_step("parse_artifacts", state, scope=ScopeWithLoadDocumentsOperator()))

    captured = capsys.readouterr()
    stdout_lines = [line for line in captured.out.splitlines() if line.strip()]
    assert any(
        "load.documents: asset filename=holiday-recap-2025.docx" in line
        and "mime=application/vnd.openxmlformats-officedocument.wordprocessingml.document" in line
        for line in stdout_lines
    )
    assert state.outputs["document_loads"] == [
        {
            "artifact_id": "test-load-documents-docx-mime:docx-1",
            "filename": "holiday-recap-2025.docx",
            "mime_type": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "reader_key": "simple_directory_reader",
            "reader_library": "llama_index.core",
            "reader_method": "SimpleDirectoryReader.load_data",
            "status": "success",
            "document_count": 1,
        }
    ]


def test_load_documents_to_parse_chunk_uses_branch_scoped_fragment_refs(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import asyncio

    from ikam.forja import debug_execution
    from ikam_perf_report.api import benchmarks
    from ikam_perf_report.benchmarks.debug_models import DebugRunState
    from ikam_perf_report.benchmarks.store import BenchmarkRunRecord, GraphSnapshot

    STORE.reset()
    run_id = "run-vertical-slice"
    artifact_id = "proj-vertical-slice:brief.md"
    expected_ref = f"refs/heads/run/{run_id}"

    STORE.add_run(
        BenchmarkRunRecord(
            run_id=run_id,
            project_id="proj-vertical-slice",
            case_id="case-vertical-slice",
            stages=[],
            decisions=[],
            project={},
            graph=GraphSnapshot(graph_id="proj-vertical-slice", fragments=[]),
        )
    )
    state = DebugRunState(
        run_id=run_id,
        pipeline_id="ingestion-early-parse",
        pipeline_run_id="pipe-vertical-slice",
        project_id="proj-vertical-slice",
        operation_id="op-vertical-slice",
        env_type="dev",
        env_id=run_id,
        execution_mode="manual",
        execution_state="paused",
        current_step_name="init.initialize",
        current_attempt_index=1,
    )
    STORE.create_debug_run_state(state)
    STORE.set_debug_runtime_context(
        run_id,
        {
            "source_bytes": b"# Brief\n\nParagraph one.\n\nParagraph two.",
            "mime_type": "text/markdown",
            "artifact_id": artifact_id,
            "asset_manifest": [
                {
                    "artifact_id": artifact_id,
                    "filename": "brief.md",
                    "mime_type": "text/markdown",
                    "size_bytes": 39,
                }
            ],
            "asset_payloads": [
                {
                    "artifact_id": artifact_id,
                    "filename": "brief.md",
                    "mime_type": "text/markdown",
                    "payload": b"# Brief\n\nParagraph one.\n\nParagraph two.",
                }
            ],
            "step_outputs": {},
        },
    )

    def _fake_load_asset(*, asset: dict[str, object], asset_index: int) -> dict[str, object]:
        assert asset_index == 0
        return {
            "reader_key": "simple_directory_reader",
            "reader_library": "llama_index.core",
            "reader_method": "SimpleDirectoryReader.load_data",
            "status": "success",
            "documents": [
                {
                    "id": "doc-1",
                    "text": "Brief\n\nParagraph one.",
                    "metadata": {"file_name": "brief.md", "artifact_id": artifact_id},
                },
                {
                    "id": "doc-2",
                    "text": "Paragraph two.",
                    "metadata": {"file_name": "brief.md", "artifact_id": artifact_id},
                },
            ],
        }

    captured_map_kwargs: dict[str, object] = {}

    def _stub_map_generation(*, artifact_id: str, assets: list[dict[str, object]], **kwargs: object) -> dict[str, object]:
        captured_map_kwargs.update(kwargs)
        return {
            "map_subgraph": {
                "root_node_id": f"map:{artifact_id}:root",
                "nodes": [
                    {"id": f"map:{artifact_id}:root", "title": "Corpus", "kind": "corpus"},
                    {"id": f"map:{artifact_id}:seg:1", "title": "Segment 1", "kind": "segment"},
                ],
                "relationships": [],
            },
            "map_dna": {"fingerprint": f"stub:{artifact_id}", "structural_hashes": [], "version": "1"},
            "segment_anchors": {f"map:{artifact_id}:seg:1": [{"artifact_id": artifact_id}]},
            "segment_candidates": [
                {
                    "segment_id": f"map:{artifact_id}:seg:1",
                    "title": "Segment 1",
                    "artifact_ids": [artifact_id],
                    "text": "Brief\n\nParagraph one.",
                }
            ],
            "profile_candidates": {f"map:{artifact_id}:seg:1": ["modelado/prose-backbone@1"]},
            "generation_provenance": {
                "provider": "test-stub",
                "model": "gpt-4o-mini",
                "prompt_version": "map-v2",
            },
        }

    monkeypatch.setattr(debug_execution, "_load_single_asset_for_debug_step", _fake_load_asset, raising=False)
    monkeypatch.setattr(debug_execution, "_invoke_mcp_map_generation", _stub_map_generation, raising=False)
    monkeypatch.setattr(
        debug_execution,
        "_load_parse_review_runner",
        lambda: (lambda payload: {"decision": "accept", "confidence": 1.0, "payload": payload}),
        raising=False,
    )

    load_event = asyncio.run(benchmarks._execute_next_pipeline_step(run_id=run_id, state=state))
    assert load_event.step_name == "load.documents"
    assert benchmarks._run_scope_ref_from_state(state) == expected_ref

    scoped_fragments = STORE.list_environment_fragments(run_id=run_id, ref=expected_ref)
    document_fragments = [
        payload
        for payload in scoped_fragments
        if isinstance(payload, dict)
        and isinstance(payload.get("meta"), dict)
        and payload["meta"].get("record_type") == "loaded_document"
    ]
    assert len(document_fragments) == 2

    runtime_context = STORE.get_debug_runtime_context(run_id)
    assert runtime_context is not None
    step_outputs = dict(runtime_context.get("step_outputs") or {})
    document_fragment_refs = list(step_outputs.get("document_fragment_refs") or [])
    assert len(document_fragment_refs) == 2
    step_outputs.pop("documents", None)
    step_outputs.pop("document_fragments", None)
    runtime_context["step_outputs"] = step_outputs
    STORE.set_debug_runtime_context(run_id, runtime_context)

    parse_event = asyncio.run(benchmarks._execute_next_pipeline_step(run_id=run_id, state=state))
    assert parse_event.step_name == "parse.chunk"
    assert captured_map_kwargs.get("document_fragment_refs") == document_fragment_refs

    latest_context = STORE.get_debug_runtime_context(run_id)
    assert latest_context is not None
    telemetry = (latest_context.get("step_outputs") or {}).get("operation_telemetry") or {}
    assert telemetry.get("document_input_mode") == "fragment_refs"
    assert telemetry.get("document_ref_count") == 2


def test_parse_chunk_emits_agent_review_and_elicitation_logs(
    monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    import asyncio
    from ikam.forja import debug_execution
    from ikam.forja.debug_execution import StepExecutionState, execute_step

    monkeypatch.setenv("IKAM_PARSE_REVIEW_APPROVAL_MODE", "human_required")

    def _stub(*, artifact_id: str, assets: list[dict[str, object]], **kwargs: object) -> dict[str, object]:
        return {
            "map_subgraph": {
                "root_node_id": f"map:{artifact_id}:root",
                "nodes": [
                    {"id": f"map:{artifact_id}:root", "title": "Corpus", "kind": "corpus"},
                    {"id": f"map:{artifact_id}:seg:a", "title": "Segment A", "kind": "segment"},
                ],
                "relationships": [],
            },
            "map_dna": {"fingerprint": f"stub:{artifact_id}", "structural_hashes": [], "version": "1"},
            "segment_anchors": {},
            "segment_candidates": [{"segment_id": f"map:{artifact_id}:seg:a", "title": "Segment A", "artifact_ids": [artifact_id], "rationale": "stub candidate"}],
            "profile_candidates": {f"map:{artifact_id}:seg:a": ["modelado/prose-backbone@1"]},
            "generation_provenance": {"provider": "test-stub", "model": "gpt-4o-mini", "prompt_version": "map-v2", "temperature": 0.0, "seed": 0},
        }

    monkeypatch.setattr(debug_execution, "_invoke_mcp_map_generation", _stub, raising=False)

    state = StepExecutionState(
        source_bytes=b"# Title\n\nParagraph one.",
        mime_type="text/markdown",
        artifact_id="agent-review:art-1",
        outputs={},
    )

    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))

    stdout_lines = [line for line in capsys.readouterr().out.splitlines() if line.strip()]
    assert any("parse.chunk: agent_review decision=" in line for line in stdout_lines)
    assert any("parse.chunk: elicitation requested approval_mode=human_required" in line for line in stdout_lines)


def test_map_generation_failure_fails_fast(monkeypatch: pytest.MonkeyPatch) -> None:
    import asyncio
    from ikam.forja import debug_execution
    from ikam.forja.debug_execution import execute_step, StepExecutionState

    def _boom(*_args: object, **_kwargs: object) -> object:
        raise RuntimeError("synthetic map failure")

    monkeypatch.setattr(debug_execution, "_invoke_mcp_map_generation", _boom, raising=False)

    state = StepExecutionState(
        source_bytes=b"hello world",
        mime_type="text/markdown",
        artifact_id="test-map-fallback:art-1",
        outputs={},
    )

    with pytest.raises(RuntimeError, match="structural map generation failed"):
        asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))


def test_map_generation_failure_ignores_legacy_strict_env(monkeypatch: pytest.MonkeyPatch) -> None:
    import asyncio
    from ikam.forja import debug_execution
    from ikam.forja.debug_execution import execute_step, StepExecutionState

    def _boom(*_args: object, **_kwargs: object) -> object:
        raise RuntimeError("synthetic map failure")

    monkeypatch.setenv("IKAM_MAP_STRICT", "1")
    monkeypatch.setattr(debug_execution, "_invoke_mcp_map_generation", _boom, raising=False)

    state = StepExecutionState(
        source_bytes=b"hello world",
        mime_type="text/markdown",
        artifact_id="test-map-strict:art-1",
        outputs={},
    )

    with pytest.raises(RuntimeError, match="structural map generation failed"):
        asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))


@pytest.mark.slow
@pytest.mark.timeout(180)
def test_lift_emits_edges_and_graph_projection():
    """lift must emit ir_fragment→surface_fragment edges ("lifted-from").

    After lift, state.outputs["edges"] must contain edges from decompose
    PLUS new lift edges: {source: ir_cas_id, target: surface_cas_id,
    predicate: "lifted-from", step: "map.conceptual.normalize.discovery"}.

    graph_projection must include all accumulated nodes and edges.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective

    source = _brand_guide_source()
    register_defaults()

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-edges-lift:art-1",
        outputs={},
    )
    # Run decompose first to populate edges and decomposition
    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))
    decompose_edge_count = len(state.outputs["edges"])

    # Run lift
    asyncio.run(execute_step("map.conceptual.normalize.discovery", state))

    edges = state.outputs["edges"]
    assert len(edges) > decompose_edge_count, (
        "lift must add new edges beyond what decompose produced"
    )

    lift_edges = [e for e in edges if e["step"] == "map.conceptual.normalize.discovery"]
    assert len(lift_edges) > 0, "lift must emit at least one 'lifted-from' edge"

    for edge in lift_edges:
        assert edge["predicate"] == "lifted-from"
        assert edge["step"] == "map.conceptual.normalize.discovery"

    # graph_projection must reflect all accumulated nodes and edges
    gp = state.outputs["graph_projection"]
    assert gp["edge_count"] == len(edges)
    # Must have ir_fragment nodes
    ir_node_ids = {n["id"] for n in gp["nodes"] if n["type"] == "ir_fragment"}
    assert len(ir_node_ids) > 0, "graph_projection must include ir_fragment nodes after lift"


@pytest.mark.slow
@pytest.mark.timeout(180)
def test_normalize_emits_edges_and_graph_projection():
    """normalize must emit normalized_fragment→ir_fragment edges ("normalized-by").

    After normalize, state.outputs["edges"] must contain edges from decompose
    and lift PLUS new normalize edges: {source: normalized_cas_id,
    target: ir_cas_id, predicate: "normalized-by", step: "map.reconstructable.normalize"}.

    graph_projection must include normalized_fragment nodes.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam.forja.cas import cas_fragment
    from ikam.fragments import Fragment as _Fragment, CONCEPT_MIME
    from ikam.ir.mime_types import CLAIM_IR

    source = _brand_guide_source()
    register_defaults()

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-edges-normalize:art-1",
        outputs={},
    )
    # Run decompose → lift → embed → candidate_search → normalize
    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))
    asyncio.run(execute_step("map.conceptual.normalize.discovery", state))

    # embed + candidate_search need DB; instead, simulate their outputs
    # so normalize can run. embed produces embeddings, candidate_search
    # produces candidates. normalize only reads ir_fragments + candidates.
    state.outputs["embeddings"] = {}
    state.outputs["candidates"] = []

    lift_edge_count = len(state.outputs["edges"])

    asyncio.run(execute_step("map.reconstructable.normalize", state))

    edges = state.outputs["edges"]
    assert len(edges) > lift_edge_count, (
        "normalize must add new edges beyond what decompose+lift produced"
    )

    norm_edges = [e for e in edges if e["step"] == "map.reconstructable.normalize"]
    assert len(norm_edges) > 0, "normalize must emit at least one edge"

    # normalize emits two predicate types: "normalized-by" and "composed-by"
    norm_by = [e for e in norm_edges if e["predicate"] == "normalized-by"]
    composed_by = [e for e in norm_edges if e["predicate"] == "composed-by"]
    assert len(norm_by) > 0, "normalize must emit at least one 'normalized-by' edge"
    assert len(composed_by) > 0, "normalize must emit at least one 'composed-by' edge"
    for edge in norm_edges:
        assert edge["predicate"] in ("normalized-by", "composed-by"), (
            f"normalize edge predicate must be 'normalized-by' or 'composed-by', got '{edge['predicate']}'"
        )
        assert edge["step"] == "map.reconstructable.normalize"

    # graph_projection must include normalized_fragment nodes
    gp = state.outputs["graph_projection"]
    assert gp["edge_count"] == len(edges)
    norm_node_ids = {n["id"] for n in gp["nodes"] if n["type"] == "normalized_fragment"}
    assert len(norm_node_ids) > 0, "graph_projection must include normalized_fragment nodes"


@pytest.mark.slow
@pytest.mark.timeout(180)
def test_verify_emits_edges_and_graph_projection():
    """verify must emit verification_result→artifact edges ("verified-by").

    After verify, state.outputs["edges"] must contain a new edge:
    {source: vr_cas_id, target: artifact_id, predicate: "verified-by",
    step: "map.conceptual.verify.discovery_gate"}.

    graph_projection must include the verification_result node.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam.forja.cas import cas_fragment
    from ikam.fragments import CONCEPT_MIME
    from ikam.ir.mime_types import CLAIM_IR

    source = _brand_guide_source()
    register_defaults()

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-edges-verify:art-1",
        outputs={},
    )
    # Run decompose → lift → skip embed/candidate_search → normalize
    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))
    asyncio.run(execute_step("map.conceptual.normalize.discovery", state))
    state.outputs["embeddings"] = {}
    state.outputs["candidates"] = []
    asyncio.run(execute_step("map.reconstructable.normalize", state))

    pre_verify_edge_count = len(state.outputs["edges"])

    # compose_proposal to build the proposal
    asyncio.run(execute_step("map.reconstructable.compose.reconstruction_programs", state))

    # verify
    asyncio.run(execute_step("map.conceptual.verify.discovery_gate", state))

    edges = state.outputs["edges"]
    assert len(edges) > pre_verify_edge_count, (
        "verify must add new edges beyond what prior steps produced"
    )

    verify_edges = [e for e in edges if e["step"] == "map.conceptual.verify.discovery_gate"]
    assert len(verify_edges) >= 1, "verify must emit at least one 'verified-by' edge"

    for edge in verify_edges:
        assert edge["predicate"] == "verified-by"
        assert edge["target"] == "test-edges-verify:art-1"
        assert edge["step"] == "map.conceptual.verify.discovery_gate"

    # graph_projection must include verification_result node
    gp = state.outputs["graph_projection"]
    assert gp["edge_count"] == len(edges)
    vr_nodes = [n for n in gp["nodes"] if n["type"] == "verification_result"]
    assert len(vr_nodes) >= 1, "graph_projection must include verification_result node"


def test_project_graph_handler_builds_edge_projection():
    """project_graph handler must build a graph projection with real edges.

    TDD RED: The current stub returns node_count from root_fragments and edge_count=0.
    The real handler must:
    - Create edges from the commit record: artifact→fragment (contains),
      IR→surface (lifted-from), concept→IR (normalized-from),
      program→concept (composed-by), verification→proposal (verified-by)
    - Each edge: {"source": CAS_ID, "target": CAS_ID, "predicate": str}
    - Store graph_projection with nodes and edges lists
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam.forja.cas import cas_fragment
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.fragments import CONCEPT_MIME
    from ikam.ir.mime_types import CLAIM_IR

    # Real decomposition
    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-graph:art-1",
    )
    decomposition = decomposer.decompose(directive)
    surface_ids = [f.cas_id for f in decomposition.structural if f.cas_id]

    # Upstream fragment references
    ir_frag = cas_fragment({"subject": "s", "predicate": "p", "object": "o"}, CLAIM_IR)
    concept_frag = cas_fragment({"concept": "revenue"}, CONCEPT_MIME)

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-graph:art-1",
        outputs={
            "decomposition": decomposition,
            "commit": {
                "mode": "normalized",
                "committed_fragment_ids": [ir_frag.cas_id, concept_frag.cas_id],
            },
            "lifted_from_map": {ir_frag.cas_id: surface_ids[0]} if surface_ids else {},
            "ir_fragments": [ir_frag],
            "normalized_fragments": [concept_frag],
        },
    )
    metrics = asyncio.run(execute_step("map.reconstructable.build_subgraph.reconstruction", state))

    graph = state.outputs.get("graph_projection")
    assert isinstance(graph, dict), f"Expected graph_projection dict, got {type(graph)}"

    # Must have nodes
    assert isinstance(graph["nodes"], list)
    assert len(graph["nodes"]) > 0

    # Must have edges (not 0)
    assert isinstance(graph["edges"], list)
    assert len(graph["edges"]) > 0, "Graph must have real edges, not edge_count=0"

    # Each edge must have source, target, predicate
    for edge in graph["edges"]:
        assert "source" in edge, f"Edge missing 'source': {edge}"
        assert "target" in edge, f"Edge missing 'target': {edge}"
        assert "predicate" in edge, f"Edge missing 'predicate': {edge}"

    # Must contain at least one "contains" edge (artifact→fragment)
    predicates = {e["predicate"] for e in graph["edges"]}
    assert "contains" in predicates, f"Missing 'contains' edge, got predicates: {predicates}"

    # If we have lifted_from_map, must have "lifted-from" edges
    if surface_ids:
        assert "lifted-from" in predicates, f"Missing 'lifted-from' edge, got: {predicates}"

    # Metrics must reflect real counts
    assert metrics["details"]["node_count"] == len(graph["nodes"])
    assert metrics["details"]["edge_count"] == len(graph["edges"])
    assert metrics["details"]["edge_count"] > 0


def test_project_graph_uses_design_doc_terminology():
    """project_graph must use canonical design doc vocabulary.

    TDD RED: The current handler uses invented terminology:
    - Node type "concept_fragment" → must be "normalized_fragment"
    - Predicate "normalized-from" → must be "normalized-by" (Appendix A)
    - Edge-building for normalized→IR is dead (reads source_ir_id/concept_ids
      which don't exist in concatenate programs), so no normalized-by edges produced

    This test provides a normalized_from_map (like lifted_from_map) that the
    handler should read to build normalized-by edges. It asserts:
    1. Normalized fragment nodes have type "normalized_fragment"
    2. "normalized-by" edges connect normalized→IR (not "normalized-from")
    3. No invented predicates remain
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam.forja.cas import cas_fragment
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.fragments import CONCEPT_MIME
    from ikam.ir.mime_types import CLAIM_IR

    # Real decomposition
    source = _brand_guide_source()
    register_defaults()
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-terminology:art-1",
    )
    decomposition = decomposer.decompose(directive)
    surface_ids = [f.cas_id for f in decomposition.structural if f.cas_id]

    # Upstream fragment references
    ir_frag = cas_fragment({"subject": "revenue", "predicate": "projects", "object": "$5M"}, CLAIM_IR)
    norm_frag = cas_fragment({"normalized": "revenue_projection_5m"}, CONCEPT_MIME)

    # normalized_from_map: maps normalized CAS ID → IR CAS ID it was derived from
    normalized_from_map = {norm_frag.cas_id: ir_frag.cas_id}

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-terminology:art-1",
        outputs={
            "decomposition": decomposition,
            "commit": {
                "mode": "normalized",
                "committed_fragment_ids": [ir_frag.cas_id, norm_frag.cas_id],
            },
            "lifted_from_map": {ir_frag.cas_id: surface_ids[0]} if surface_ids else {},
            "ir_fragments": [ir_frag],
            "normalized_fragments": [norm_frag],
            "normalized_from_map": normalized_from_map,
        },
    )
    metrics = asyncio.run(execute_step("map.reconstructable.build_subgraph.reconstruction", state))

    graph = state.outputs["graph_projection"]
    nodes = graph["nodes"]
    edges = graph["edges"]

    # --- Assertion 1: Node type for normalized fragments must be "normalized_fragment" ---
    norm_node = [n for n in nodes if n["id"] == norm_frag.cas_id]
    assert len(norm_node) == 1, f"Expected 1 node for normalized fragment, got {len(norm_node)}"
    assert norm_node[0]["type"] == "normalized_fragment", (
        f"Normalized fragment node type must be 'normalized_fragment', "
        f"got '{norm_node[0]['type']}'"
    )

    # --- Assertion 2: "normalized-by" edges must exist (not "normalized-from") ---
    all_predicates = {e["predicate"] for e in edges}
    assert "normalized-from" not in all_predicates, (
        f"Invented predicate 'normalized-from' found — must use 'normalized-by' per design doc Appendix A"
    )
    assert "normalized-by" in all_predicates, (
        f"Missing 'normalized-by' predicate, got: {all_predicates}"
    )

    # --- Assertion 3: normalized-by edges connect normalized→IR correctly ---
    norm_by_edges = [e for e in edges if e["predicate"] == "normalized-by"]
    assert len(norm_by_edges) >= 1, "Must have at least one normalized-by edge"
    for edge in norm_by_edges:
        assert edge["source"] == norm_frag.cas_id, f"normalized-by source must be normalized frag, got {edge['source']}"
        assert edge["target"] == ir_frag.cas_id, f"normalized-by target must be IR frag, got {edge['target']}"

    # --- Assertion 4: Only canonical predicates from design doc Appendix A ---
    canonical_predicates = {
        "contains", "lifted-from", "normalized-by", "composed-by", "verified-by",
        "original-bytes-of",
    }
    for pred in all_predicates:
        assert pred in canonical_predicates, (
            f"Non-canonical predicate '{pred}' — must be one of {canonical_predicates}"
        )


def test_run_benchmark_is_prepare_only(case_fixtures_root, monkeypatch):
    """run_benchmark must return in <500ms with no pipeline computation.

    Design Decision 2: Run Cases loads fixture, stores source_bytes in
    runtime context, creates DebugRunState at prepare_case, and returns
    immediately.  No decomposition, no graph, no evaluation, no semantic.
    """
    from time import perf_counter

    # Trap: if run_benchmark calls run_semantic_pipeline, test fails immediately
    def _semantic_must_not_be_called(text):
        raise AssertionError("run_benchmark must not call run_semantic_pipeline in prepare-only mode")

    monkeypatch.setattr(runner, "run_semantic_pipeline", _semantic_must_not_be_called)

    STORE.reset()
    start = perf_counter()
    result = runner.run_benchmark(case_ids="s-construction-v01", include_evaluation=False)
    elapsed_ms = (perf_counter() - start) * 1000

    assert elapsed_ms < 500, f"run_benchmark took {elapsed_ms:.0f}ms, must be <500ms (prepare-only)"

    runs = result["runs"]
    assert len(runs) == 1
    run = runs[0]

    # Must have identifiers and case metadata
    assert run["run_id"].startswith("run-")
    assert run["case_id"] == "s-construction-v01"
    assert "project_id" in run

    # Must have DebugRunState at prepare_case with paused state
    assert "debug_state" in run, "run must include debug_state dict"
    assert run["debug_state"]["current_step_name"] == "init.initialize"
    assert run["debug_state"]["execution_state"] == "paused"

    # Must NOT have computation artifacts
    assert run.get("graph") is None, "prepare-only must not produce graph"
    assert run.get("semantic") is None, "prepare-only must not run semantic pipeline"
    assert run.get("answer_quality") is None, "prepare-only must not compute quality signals"
    assert run.get("stages") is None or run["stages"] == [], "prepare-only must not record computation stages"
    assert run.get("evaluation") is None or run.get("evaluation") == {}, "prepare-only must not run evaluation"




def test_lift_handler_consumes_surface_clusters():
    """Lift handler must consume surface_clusters from embed_decomposed.

    When state.outputs["surface_clusters"] is present (from embed_decomposed),
    the lift handler must:
    1. Pass cluster context to the lifter so the LLM knows which surface
       fragments are semantically similar
    2. Record in metrics that cluster context was used:
       metrics["details"]["cluster_context_used"] = True
       metrics["details"]["cluster_count"] = N
    3. Still produce valid ir_fragments and lifted_from_map as before

    Discriminating assertion: The current lift handler (line 238) calls
    lifter.lift(surface_frag) with NO cluster awareness. This test asserts
    metrics["details"]["cluster_context_used"] is True, which the current
    handler does not produce.

    Backward compat: When no surface_clusters exist, cluster_context_used
    must be False (tested separately by existing tests that don't provide clusters).
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam.fragments import Fragment
    from ikam.ir.mime_types import CLAIM_IR

    # Source with content that produces surface fragments
    source = b"# Revenue Model\n\nWe project $5M ARR by end of year two based on 200 enterprise contracts at $25K ACV."

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-lift-clusters:art-1",
        outputs={},
    )

    # Run map to get semantic segment candidates
    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))
    segment_candidates = state.outputs.get("map_segment_candidates")
    assert isinstance(segment_candidates, list)
    assert len(segment_candidates) >= 1, "Need at least 1 map segment candidate"

    # Run embed_decomposed to get surface_embeddings and surface_clusters
    asyncio.run(execute_step("map.conceptual.embed.discovery_index", state))
    surface_clusters = state.outputs.get("surface_clusters")
    assert isinstance(surface_clusters, list), "embed_decomposed must produce surface_clusters"

    # Run lift — this is what we're testing
    metrics = asyncio.run(execute_step("map.conceptual.normalize.discovery", state))

    # === Discriminating assertion ===
    # Current handler does NOT report cluster_context_used
    assert "details" in metrics
    assert metrics["details"].get("cluster_context_used") is True, (
        f"Lift handler must report cluster_context_used=True when surface_clusters "
        f"are available. Got details: {metrics['details']}"
    )
    assert metrics["details"].get("cluster_count") == len(surface_clusters), (
        f"Lift must report cluster_count matching surface_clusters length. "
        f"Expected {len(surface_clusters)}, got {metrics['details'].get('cluster_count')}"
    )

    # Standard lift assertions must still hold
    ir_fragments = state.outputs.get("ir_fragments")
    assert isinstance(ir_fragments, list)
    assert len(ir_fragments) > 0, "Lift must still produce IR fragments"
    for frag in ir_fragments:
        assert isinstance(frag, Fragment)
        assert frag.mime_type == CLAIM_IR

    lifted_from_map = state.outputs.get("lifted_from_map")
    assert isinstance(lifted_from_map, dict)
    assert len(lifted_from_map) == len(ir_fragments)


def test_lift_handler_reports_no_clusters_when_absent():
    """Lift handler backward compat: when no surface_clusters, cluster_context_used=False.

    Existing tests pass decomposition directly without running embed_decomposed.
    The lift handler must still work and report cluster_context_used=False.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam.fragments import Fragment
    from ikam.ir.mime_types import CLAIM_IR

    source = b"# Revenue Model\n\nWe project $5M ARR by end of year two."

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-lift-no-clusters:art-1",
        outputs={
            "map_segment_candidates": [
                {
                    "segment_id": "map:seg:test-lift-no-clusters:1",
                    "title": "Revenue projection",
                    "rationale": "Single segment candidate for lift fallback path",
                    "artifact_ids": ["test-lift-no-clusters:art-1"],
                }
            ]
        },
        # NO surface_clusters — backward compat path
    )
    metrics = asyncio.run(execute_step("map.conceptual.normalize.discovery", state))

    # Must report cluster_context_used=False
    assert metrics["details"].get("cluster_context_used") is False, (
        f"Without surface_clusters, cluster_context_used must be False. "
        f"Got details: {metrics['details']}"
    )
    assert metrics["details"].get("cluster_count") == 0

    # Standard lift still works
    ir_fragments = state.outputs.get("ir_fragments")
    assert isinstance(ir_fragments, list)
    assert len(ir_fragments) > 0


def test_embed_decomposed_produces_surface_embeddings_and_clusters():
    """embed_decomposed must embed surface fragments and produce clusters.

    After running decompose + embed_decomposed, state.outputs must contain:
    - surface_embeddings: dict[str, list[float]] mapping cas_id → vector
    - surface_clusters: list[dict] with member groupings
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState

    # Use multi-section markdown to get multiple surface fragments
    source = b"# Revenue\n\nWe project $5M ARR by 2027.\n\n# Costs\n\nOperating costs are $2M annually.\n\n# Growth\n\nRevenue growth rate is 30% year-over-year."

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-embed-decomposed:art-1",
        outputs={},
    )
    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))
    asyncio.run(execute_step("map.conceptual.embed.discovery_index", state))

    # Must have surface_embeddings
    surface_embeddings = state.outputs.get("surface_embeddings")
    assert isinstance(surface_embeddings, dict), (
        f"Expected surface_embeddings dict, got {type(surface_embeddings)}"
    )
    assert len(surface_embeddings) > 0, "Must embed at least one surface fragment"

    # Each embedding must be a list of floats (768-dim padded)
    for cas_id, vec in surface_embeddings.items():
        assert isinstance(vec, list), f"Embedding for {cas_id} must be a list"
        assert len(vec) == 768, f"Embedding for {cas_id} must be 768-dim, got {len(vec)}"

    # Must have surface_clusters
    surface_clusters = state.outputs.get("surface_clusters")
    assert isinstance(surface_clusters, list), (
        f"Expected surface_clusters list, got {type(surface_clusters)}"
    )
    # Each cluster must have members, centroid_id, avg_similarity
    for cluster in surface_clusters:
        assert "members" in cluster, "cluster must have members"
        assert "centroid_id" in cluster, "cluster must have centroid_id"
        assert "avg_similarity" in cluster, "cluster must have avg_similarity"
        assert isinstance(cluster["members"], list)
        assert len(cluster["members"]) >= 1


# ---------------------------------------------------------------------------
# C3: Semantic Predicates — Predicate Vocabulary
# ---------------------------------------------------------------------------


def test_predicate_vocabulary_default_fragment():
    """build_default_vocabulary() returns a CAS-addressed Fragment containing
    all 23 canonical predicates (6 structural + 13 knowledge + 4 semantic)
    with deterministic CAS ID."""
    from ikam.ir.mime_types import PREDICATE_VOCABULARY
    from ikam.forja.predicate_vocabulary import build_default_vocabulary

    vocab = build_default_vocabulary()

    # Must be a Fragment with the vocabulary MIME type
    assert vocab.mime_type == PREDICATE_VOCABULARY
    assert vocab.cas_id is not None, "Vocabulary fragment must be CAS-addressed"

    # Value structure: dict with "predicates" and "version"
    val = vocab.value
    assert isinstance(val, dict), f"Expected dict, got {type(val)}"
    assert "predicates" in val, "Missing 'predicates' key"
    assert "version" in val, "Missing 'version' key"
    assert val["version"] == "2026-02-10", f"Expected version 2026-02-10, got {val['version']}"

    predicates = val["predicates"]
    assert isinstance(predicates, list), f"Expected list, got {type(predicates)}"
    assert len(predicates) == 23, f"Expected 23 predicates, got {len(predicates)}"

    # Each predicate has name, category, description
    valid_categories = {"structural", "knowledge", "semantic"}
    for p in predicates:
        assert isinstance(p, dict), f"Each predicate must be a dict, got {type(p)}"
        assert "name" in p, f"Predicate missing 'name': {p}"
        assert "category" in p, f"Predicate missing 'category': {p}"
        assert "description" in p, f"Predicate missing 'description': {p}"
        assert isinstance(p["name"], str) and len(p["name"]) > 0
        assert p["category"] in valid_categories, (
            f"Invalid category '{p['category']}' for predicate '{p['name']}'"
        )
        assert isinstance(p["description"], str) and len(p["description"]) > 0

    # Structural predicates must match canonical names exactly
    structural = [p for p in predicates if p["category"] == "structural"]
    structural_names = {p["name"] for p in structural}
    expected_structural = {
        "contains", "lifted-from", "normalized-by",
        "composed-by", "verified-by", "original-bytes-of",
    }
    assert structural_names == expected_structural, (
        f"Structural predicates mismatch: got {structural_names}, "
        f"expected {expected_structural}"
    )

    # Knowledge predicates: 13 total
    knowledge = [p for p in predicates if p["category"] == "knowledge"]
    assert len(knowledge) == 13, f"Expected 13 knowledge predicates, got {len(knowledge)}"

    # Semantic predicates: 4 total
    semantic = [p for p in predicates if p["category"] == "semantic"]
    assert len(semantic) == 4, f"Expected 4 semantic predicates, got {len(semantic)}"
    semantic_names = {p["name"] for p in semantic}
    expected_semantic = {"shadows", "derived-from", "depends-on", "feeds"}
    assert semantic_names == expected_semantic, (
        f"Semantic predicates mismatch: got {semantic_names}, "
        f"expected {expected_semantic}"
    )


def test_predicate_vocabulary_deterministic_cas():
    """Calling build_default_vocabulary() twice yields identical CAS IDs —
    the vocabulary is deterministic."""
    from ikam.forja.predicate_vocabulary import build_default_vocabulary

    v1 = build_default_vocabulary()
    v2 = build_default_vocabulary()
    assert v1.cas_id == v2.cas_id, (
        f"Vocabulary CAS IDs must be deterministic: {v1.cas_id} != {v2.cas_id}"
    )


def test_predicate_resolver_similarity():
    """PredicateResolver embeds all predicates and computes similarity.

    Verifies:
    1. resolve() returns a PredicateResolver with embedded predicates
    2. similar("derived-from") returns ranked results with scores
    3. Semantically close predicates (derived-from ≈ lifted-from) score
       higher than distant ones (derived-from vs compose:generate)
    4. Each result has name, score, category
    """
    from ikam.forja.predicate_vocabulary import (
        build_default_vocabulary,
        PredicateResolver,
    )

    vocab = build_default_vocabulary()
    resolver = PredicateResolver.from_vocabulary(vocab)

    # Must have embeddings for all 23 predicates
    assert len(resolver.predicate_names) == 23

    # similar() returns ranked results
    results = resolver.similar("derived-from", top_k=5)
    assert isinstance(results, list)
    assert len(results) == 5  # top_k=5

    # Each result has name, score, category
    for r in results:
        assert "name" in r, f"Result missing 'name': {r}"
        assert "score" in r, f"Result missing 'score': {r}"
        assert "category" in r, f"Result missing 'category': {r}"
        assert isinstance(r["score"], float)
        assert 0.0 <= r["score"] <= 1.0, f"Score out of range: {r['score']}"

    # Top result for "derived-from" must be "derived-from" itself (score ~1.0)
    assert results[0]["name"] == "derived-from", (
        f"Top match for 'derived-from' should be itself, got '{results[0]['name']}'"
    )
    assert results[0]["score"] > 0.95, (
        f"Self-similarity should be >0.95, got {results[0]['score']}"
    )

    # Verify scores are monotonically non-increasing (sorted by similarity)
    for i in range(len(results) - 1):
        assert results[i]["score"] >= results[i + 1]["score"], (
            f"Results not sorted: {results[i]['name']} ({results[i]['score']}) < "
            f"{results[i + 1]['name']} ({results[i + 1]['score']})"
        )


def test_predicate_resolver_deterministic():
    """PredicateResolver produces identical similarity rankings across calls."""
    from ikam.forja.predicate_vocabulary import (
        build_default_vocabulary,
        PredicateResolver,
    )

    vocab = build_default_vocabulary()
    r1 = PredicateResolver.from_vocabulary(vocab)
    r2 = PredicateResolver.from_vocabulary(vocab)

    results1 = r1.similar("contains", top_k=5)
    results2 = r2.similar("contains", top_k=5)

    names1 = [r["name"] for r in results1]
    names2 = [r["name"] for r in results2]
    assert names1 == names2, f"Rankings differ: {names1} vs {names2}"

    scores1 = [r["score"] for r in results1]
    scores2 = [r["score"] for r in results2]
    for s1, s2 in zip(scores1, scores2):
        assert abs(s1 - s2) < 1e-6, f"Scores differ: {s1} vs {s2}"


# ---------------------------------------------------------------------------
# C3.3 — Emit missing predicates + vocabulary in graph_projection
# ---------------------------------------------------------------------------


def test_decompose_emits_original_bytes_of_edge():
    """decompose must emit an original-bytes-of edge from CAS(source_bytes) → artifact.

    Per design doc Appendix A, `original-bytes-of` records the CAS fragment
    holding the original imported bytes. After decompose, state.outputs["edges"]
    must contain: {source: <cas_id of raw bytes>, target: artifact_id,
    predicate: "original-bytes-of", step: "map.conceptual.lift.surface_fragments"}.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults

    source = _brand_guide_source()
    register_defaults()

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-original-bytes:art-1",
        outputs={},
    )
    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))

    edges = state.outputs.get("edges", [])
    ob_edges = [e for e in edges if e["predicate"] == "original-bytes-of"]
    assert len(ob_edges) == 1, (
        f"decompose must emit exactly one 'original-bytes-of' edge, "
        f"got {len(ob_edges)}: {ob_edges}"
    )
    edge = ob_edges[0]
    assert edge["target"] == "test-original-bytes:art-1", (
        f"original-bytes-of target must be artifact_id, got {edge['target']}"
    )
    assert edge["step"] == "map.conceptual.lift.surface_fragments", (
        f"original-bytes-of step must be 'map', got {edge['step']}"
    )
    # Source must be a CAS ID (64-char hex hash)
    assert len(edge["source"]) == 64 and all(c in "0123456789abcdef" for c in edge["source"]), (
        f"original-bytes-of source must be a CAS ID (blake3 hex), got {edge['source']}"
    )


def test_normalize_emits_composed_by_edge():
    """normalize must emit composed-by edges from reconstruction_program → normalized_fragment.

    Per design doc Appendix A, `composed-by` records that a fragment was
    produced by a ReconstructionProgram. After normalize, edges must contain
    at least one: {source: <program_cas_id>, target: <normalized_cas_id>,
    predicate: "composed-by", step: "map.reconstructable.normalize"}.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults

    source = _brand_guide_source()
    register_defaults()

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-composed-by:art-1",
        outputs={},
    )
    # Run decompose → lift, then simulate embed/candidate_search
    asyncio.run(execute_step("map.conceptual.lift.surface_fragments", state))
    asyncio.run(execute_step("map.conceptual.normalize.discovery", state))
    state.outputs["embeddings"] = {}
    state.outputs["candidates"] = []

    asyncio.run(execute_step("map.reconstructable.normalize", state))

    edges = state.outputs.get("edges", [])
    cb_edges = [e for e in edges if e["predicate"] == "composed-by"]
    assert len(cb_edges) >= 1, (
        f"normalize must emit at least one 'composed-by' edge, "
        f"got {len(cb_edges)}. All edges: {[e['predicate'] for e in edges]}"
    )

    # Each composed-by edge: source is a program CAS ID, target is a normalized CAS ID
    normalize_programs = state.outputs.get("normalize_reconstruction_programs", [])
    program_ids = {f.cas_id for f in normalize_programs if f.cas_id}
    normalized = state.outputs.get("normalized_fragments", [])
    normalized_ids = {f.cas_id for f in normalized if f.cas_id}

    for edge in cb_edges:
        assert edge["step"] == "map.reconstructable.normalize", (
            f"composed-by step must be 'normalize', got {edge['step']}"
        )
        assert edge["source"] in program_ids, (
            f"composed-by source must be a normalize program CAS ID, "
            f"got {edge['source']}, known programs: {program_ids}"
        )
        assert edge["target"] in normalized_ids, (
            f"composed-by target must be a normalized fragment CAS ID, "
            f"got {edge['target']}, known normalized: {normalized_ids}"
        )


def test_project_graph_includes_predicate_vocabulary():
    """project_graph must store the predicate vocabulary in state.outputs.

    The predicate vocabulary is a CAS-addressed Fragment with MIME type
    PREDICATE_VOCABULARY containing all canonical predicates. After
    project_graph, state.outputs["predicate_vocabulary"] must be a Fragment
    with a valid cas_id and the correct MIME type.
    """
    import asyncio
    from ikam.forja.debug_execution import execute_step, StepExecutionState
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.fragments import Fragment as _Fragment
    from ikam.ir.mime_types import PREDICATE_VOCABULARY

    source = _brand_guide_source()
    register_defaults()

    # Minimal state for project_graph (backward-compat path)
    decomposer = get_decomposer("text/markdown")
    directive = DecompositionDirective(
        source=source, mime_type="text/markdown", artifact_id="test-vocab:art-1",
    )
    decomposition = decomposer.decompose(directive)

    state = StepExecutionState(
        source_bytes=source,
        mime_type="text/markdown",
        artifact_id="test-vocab:art-1",
        outputs={
            "decomposition": decomposition,
            "commit": {"mode": "surface_only", "committed_fragment_ids": []},
        },
    )
    asyncio.run(execute_step("map.reconstructable.build_subgraph.reconstruction", state))

    vocab = state.outputs.get("predicate_vocabulary")
    assert vocab is not None, (
        "project_graph must store predicate_vocabulary in state.outputs"
    )
    assert isinstance(vocab, _Fragment), (
        f"predicate_vocabulary must be a Fragment, got {type(vocab)}"
    )
    assert vocab.cas_id is not None, (
        "predicate_vocabulary Fragment must have a CAS ID"
    )
    assert vocab.mime_type == PREDICATE_VOCABULARY, (
        f"predicate_vocabulary MIME must be {PREDICATE_VOCABULARY}, "
        f"got {vocab.mime_type}"
    )
