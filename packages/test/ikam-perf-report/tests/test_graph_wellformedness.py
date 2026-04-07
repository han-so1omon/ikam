"""Graph well-formedness checks for fragment pipeline output.

Uses validation patterns from stagehand_validations.py as structural reference.
"""
import pytest
from pathlib import Path

CORPUS_DIR = Path(__file__).resolve().parents[4] / "tests" / "fixtures" / "cases" / "s-local-retail-v01"


@pytest.fixture(scope="module")
def corpus_graph() -> dict:
    """Build a fragment graph from all corpus files."""
    import mimetypes
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.forja.cas import cas_fragment

    register_defaults()

    artifacts = {}  # artifact_id -> list[Fragment]
    all_cas_ids = set()
    fragment_by_cas = {}

    corpus_extensions = {".md", ".xlsx", ".pptx", ".pdf", ".json"}
    for ext in corpus_extensions:
        for f in sorted(CORPUS_DIR.glob(f"*{ext}")):
            if f.name.startswith(".") or f.name.startswith("_") or f.name.endswith(".py"):
                continue
            mime_type, _ = mimetypes.guess_type(str(f))
            if mime_type is None:
                continue
            artifact_id = f"graph/{f.name}"
            decomposer = get_decomposer(mime_type)
            payload = f.read_bytes()
            directive = DecompositionDirective(source=payload, mime_type=mime_type, artifact_id=artifact_id)
            result = decomposer.decompose(directive)
            artifacts[artifact_id] = result.structural
            for frag in result.structural:
                # cas_fragment takes (value, mime_type), not a Fragment object
                cas = cas_fragment(frag.value, frag.mime_type)
                all_cas_ids.add(cas.cas_id)
                fragment_by_cas[cas.cas_id] = frag

    return {
        "artifacts": artifacts,
        "all_cas_ids": all_cas_ids,
        "fragment_by_cas": fragment_by_cas,
    }


def test_no_empty_artifacts(corpus_graph):
    """Every artifact has at least one fragment (no orphan artifacts)."""
    for artifact_id, fragments in corpus_graph["artifacts"].items():
        assert len(fragments) >= 1, f"Artifact {artifact_id} has 0 fragments"


def test_all_fragments_have_content(corpus_graph):
    """No fragment has empty/null value."""
    for cas_id, frag in corpus_graph["fragment_by_cas"].items():
        assert frag.value is not None, f"Fragment {cas_id} has None value"
        if isinstance(frag.value, dict):
            # At least one key should have content
            assert len(frag.value) > 0, f"Fragment {cas_id} has empty value dict"


def test_all_fragments_have_mime_type(corpus_graph):
    """Every fragment has a non-empty MIME type."""
    for cas_id, frag in corpus_graph["fragment_by_cas"].items():
        assert frag.mime_type, f"Fragment {cas_id} has no MIME type"
        assert "/" in frag.mime_type, f"Fragment {cas_id} has invalid MIME type: {frag.mime_type}"


def test_root_fragment_identifiable(corpus_graph):
    """Each artifact's first fragment can serve as root (position 0)."""
    for artifact_id, fragments in corpus_graph["artifacts"].items():
        # Root is always fragments[0] from decomposition
        root = fragments[0]
        assert root.value is not None, f"Root fragment of {artifact_id} has no value"


def test_cas_id_uniqueness(corpus_graph):
    """CAS IDs are deterministic — same content produces same ID."""
    from ikam.forja.cas import cas_fragment

    for cas_id, frag in corpus_graph["fragment_by_cas"].items():
        # cas_fragment takes (value, mime_type), not a Fragment object
        recomputed = cas_fragment(frag.value, frag.mime_type)
        assert recomputed.cas_id == cas_id, f"CAS ID mismatch for fragment: expected {cas_id}, got {recomputed.cas_id}"


def test_evaluation_report_structure():
    """Validate evaluation report structure using stagehand patterns."""
    from ikam_perf_report.benchmarks.stagehand_validations import validate_evaluation_report

    # Construct a minimal valid report to verify the validator itself works
    valid_payload = {
        "report": {
            "compression": {"ratio": 0.85},
            "entities": {"count": 10},
            "predicates": {"count": 5},
            "exploration": {"depth": 3},
            "query": {"answered": 2},
            "passed": True,
        },
        "rendered": "# Report\n\nCompression: 85%\nEntities: 10\nPredicates: 5\nExploration: depth 3\nQuery: 2 answered",
    }
    result = validate_evaluation_report(valid_payload)
    assert result["status"] == "ok"

    # Invalid report should raise
    with pytest.raises(AssertionError):
        validate_evaluation_report({"report": None, "rendered": ""})
