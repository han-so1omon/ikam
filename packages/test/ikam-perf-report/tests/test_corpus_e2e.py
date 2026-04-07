"""End-to-end corpus ingestion for s-local-retail-v01."""
import pytest
from pathlib import Path

CORPUS_DIR = Path(__file__).resolve().parents[4] / "tests" / "fixtures" / "cases" / "s-local-retail-v01"

# All document files (exclude .venv, _sources, assets/images, scripts)
CORPUS_EXTENSIONS = {".md", ".xlsx", ".pptx", ".pdf", ".json"}


def _corpus_files() -> list[Path]:
    """Collect all ingestible files from the corpus directory."""
    files = []
    for ext in CORPUS_EXTENSIONS:
        for f in CORPUS_DIR.glob(f"*{ext}"):
            if f.name.startswith(".") or f.name.startswith("_"):
                continue
            # Skip generator scripts
            if f.name in ("generate_artifacts.py", "generate_images_comfy.py", "build_full_prompts.py", "enhance_artifacts.py"):
                continue
            files.append(f)
    return sorted(files)


def test_corpus_files_exist():
    """Sanity: corpus has expected file count."""
    files = _corpus_files()
    assert len(files) >= 20, f"Expected ≥20 corpus files, got {len(files)}: {[f.name for f in files]}"


@pytest.mark.parametrize("corpus_file", _corpus_files(), ids=lambda f: f.name)
def test_decompose_corpus_file(corpus_file: Path):
    """Each corpus file decomposes without error and produces ≥1 fragment."""
    import mimetypes
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective

    register_defaults()

    mime_type, _ = mimetypes.guess_type(str(corpus_file))
    if mime_type is None:
        mime_type = "application/octet-stream"

    decomposer = get_decomposer(mime_type)
    payload = corpus_file.read_bytes()
    directive = DecompositionDirective(
        source=payload,
        mime_type=mime_type,
        artifact_id=f"corpus/{corpus_file.name}",
    )
    result = decomposer.decompose(directive)
    assert len(result.structural) >= 1, f"No fragments from {corpus_file.name}"


@pytest.mark.parametrize("corpus_file", [f for f in _corpus_files() if f.suffix in (".md",)], ids=lambda f: f.name)
def test_verify_markdown_roundtrip(corpus_file: Path):
    """Markdown corpus files pass byte-identity verification (lossless round-trip)."""
    import base64
    from ikam_perf_report.benchmarks.decomposition_compat import register_defaults
    from ikam_perf_report.benchmarks.decomposition_compat import get_decomposer
    from ikam_perf_report.benchmarks.decomposition_compat import DecompositionDirective
    from ikam.forja.verifier import ByteIdentityVerifier, DriftSpec

    register_defaults()

    mime_type = "text/markdown"
    decomposer = get_decomposer(mime_type)
    payload = corpus_file.read_bytes()
    directive = DecompositionDirective(
        source=payload,
        mime_type=mime_type,
        artifact_id=f"corpus/{corpus_file.name}",
    )
    result = decomposer.decompose(directive)

    # Verify canonical fragment roundtrips
    assert result.canonical is not None, f"No canonical fragment for {corpus_file.name}"
    canonical_b64 = result.canonical.value.get("bytes_b64", "")
    reconstructed = base64.b64decode(canonical_b64)

    verifier = ByteIdentityVerifier()
    drift_spec = DriftSpec(metric="byte-identity")
    vr = verifier.verify(result.canonical, reconstructed, drift_spec)
    assert vr.value["passed"], f"Verification failed for {corpus_file.name}"
