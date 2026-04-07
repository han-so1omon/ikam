"""Oracle-based evaluation for s-local-retail-v01 corpus."""
import json
import pytest
from pathlib import Path

CORPUS_DIR = Path(__file__).resolve().parents[4] / "tests" / "fixtures" / "cases" / "s-local-retail-v01"
ORACLE_PATH = CORPUS_DIR / "oracle.json"


@pytest.fixture(scope="module")
def oracle() -> dict:
    return json.loads(ORACLE_PATH.read_text())


@pytest.fixture(scope="module")
def ingested_fragments() -> list:
    """Ingest all corpus files and return all fragments."""
    import mimetypes
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective

    register_defaults()

    all_fragments = []
    corpus_extensions = {".md", ".xlsx", ".pptx", ".pdf", ".json"}
    for ext in corpus_extensions:
        for f in sorted(CORPUS_DIR.glob(f"*{ext}")):
            if f.name.startswith(".") or f.name.startswith("_"):
                continue
            if f.name in ("generate_artifacts.py", "generate_images_comfy.py", "build_full_prompts.py", "enhance_artifacts.py"):
                continue
            mime_type, _ = mimetypes.guess_type(str(f))
            if mime_type is None:
                mime_type = "application/octet-stream"
            decomposer = get_decomposer(mime_type)
            payload = f.read_bytes()
            directive = DecompositionDirective(source=payload, mime_type=mime_type, artifact_id=f"oracle/{f.name}")
            result = decomposer.decompose(directive)
            all_fragments.extend(result.structural)
    return all_fragments


def test_oracle_loads(oracle):
    """Oracle file is valid and has expected sections."""
    assert oracle["case_id"] == "s-local-retail-v01"
    assert len(oracle["entities"]) >= 10
    assert len(oracle["predicates"]) >= 5
    assert len(oracle.get("contradictions", [])) >= 1


def test_entity_extraction_coverage(oracle, ingested_fragments):
    """At least 70% of oracle entities appear in fragment text content."""
    from ikam.ir.text_conversion import fragment_to_text

    all_text = " ".join(fragment_to_text(f) for f in ingested_fragments).lower()
    oracle_entities = oracle["entities"]
    found = 0
    for entity in oracle_entities:
        names_to_check = [entity["name"].lower()] + [a.lower() for a in entity.get("aliases", [])]
        if any(name in all_text for name in names_to_check):
            found += 1
    coverage = found / len(oracle_entities)
    assert coverage >= 0.70, f"Entity coverage {coverage:.0%} < 70% ({found}/{len(oracle_entities)})"


def test_predicate_evidence_coverage(oracle, ingested_fragments):
    """At least 60% of oracle predicates have evidence fragments."""
    from ikam.ir.text_conversion import fragment_to_text

    all_text = " ".join(fragment_to_text(f) for f in ingested_fragments).lower()
    oracle_predicates = oracle["predicates"]
    found = 0
    for pred in oracle_predicates:
        # Check if source and target entities appear in fragment text
        chain = pred.get("chain", [])
        if not chain:
            continue
        source = chain[0].get("source", "").lower()
        target = chain[0].get("target", "").lower()
        if source in all_text and target in all_text:
            found += 1
    coverage = found / len(oracle_predicates) if oracle_predicates else 0
    assert coverage >= 0.60, f"Predicate evidence coverage {coverage:.0%} < 60% ({found}/{len(oracle_predicates)})"
