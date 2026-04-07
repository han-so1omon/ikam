from __future__ import annotations

from pathlib import Path
import sys

import yaml

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))
sys.path.insert(0, str(ROOT / "packages/ikam/src"))

from ikam import cli as ikam_cli
from modelado.plans.preload import default_preseed_root
from modelado.preseed_paths import preseed_source_dirs


def test_default_preseed_root_points_to_perf_report_preseed() -> None:
    assert default_preseed_root() == ROOT / "packages/test/ikam-perf-report/preseed"


def test_consolidated_preseed_layout_exists() -> None:
    preseed_root = ROOT / "packages/test/ikam-perf-report/preseed"

    assert (preseed_root / "README.md").is_file()
    assert (preseed_root / "declarations").is_dir()
    assert (preseed_root / "workflows").is_dir()
    assert (preseed_root / "operators").is_dir()
    assert (preseed_root / "compiled").is_dir()
    assert (preseed_root / "support").is_dir()


def test_consolidated_preseed_layout_has_yaml_sources_and_compiled_outputs() -> None:
    preseed_root = ROOT / "packages/test/ikam-perf-report/preseed"

    assert (preseed_root / "declarations" / "agent-executor.yaml").is_file()
    assert list((preseed_root / "operators").glob("*.yaml"))
    assert list((preseed_root / "compiled").glob("*.yaml"))


def test_fixture_compiler_cli_defaults_to_consolidated_preseed_layout() -> None:
    assert ikam_cli.PRESEED_OPERATORS_DIR == ROOT / "packages/test/ikam-perf-report/preseed/operators"
    assert ikam_cli.PRESEED_COMPILED_DIR == ROOT / "packages/test/ikam-perf-report/preseed/compiled"
    assert ikam_cli.PRESEED_OPERATORS_DIR.is_dir()
    assert ikam_cli.PRESEED_COMPILED_DIR.is_dir()


def test_preseed_source_dirs_cover_documented_source_subdirs() -> None:
    assert preseed_source_dirs() == (
        ROOT / "packages/test/ikam-perf-report/preseed/declarations",
        ROOT / "packages/test/ikam-perf-report/preseed/workflows",
        ROOT / "packages/test/ikam-perf-report/preseed/operators",
    )


def test_compile_graph_rejects_unknown_ref_targets(tmp_path: Path) -> None:
    bad_yaml = tmp_path / "bad.yaml"
    bad_yaml.write_text(
        """graph_id: test\nfragments:\n  - name: test.fragment\n    mime_type: application/json\n    value:\n      subject: \"${ref:missing.fragment}\"\n""",
        encoding="utf-8",
    )

    try:
        ikam_cli.compile_graph(bad_yaml, known_names={"test.fragment"})
    except ValueError as exc:
        assert str(exc) == "Unknown ref target: missing.fragment"
    else:
        raise AssertionError("expected unknown ref target to be rejected")


def test_legacy_modelado_fixture_root_is_removed() -> None:
    assert not (ROOT / "packages/modelado/fixtures/graphs").exists()


def test_ingestion_operator_preseed_declares_runnable_step_operators() -> None:
    operators_dir = ROOT / "packages/test/ikam-perf-report/preseed/operators"
    ml_executor_document = yaml.safe_load((operators_dir / "ml_executor.yaml").read_text(encoding="utf-8"))
    ml_fragments = {fragment["name"]: fragment for fragment in ml_executor_document["fragments"]}

    assert ml_fragments["exec_ml_claim"]["value"]["subject"] == "${ref:exec_ml_config}"

    operator_files = {
        "load.documents": operators_dir / "load_documents_operator.yaml",
        "parse.chunk": operators_dir / "parse_chunk_operator.yaml",
        "parse.entities_and_relationships": operators_dir / "entities_and_relationships_operator.yaml",
        "parse.claims": operators_dir / "claims_operator.yaml",
    }
    expected_operator_ids = {
        "load.documents": "modelado/operators/load_documents",
        "parse.chunk": "modelado/operators/chunking",
        "parse.entities_and_relationships": "modelado/operators/entities_and_relationships",
        "parse.claims": "modelado/operators/claims",
    }

    for step_name, operator_file in operator_files.items():
        document = yaml.safe_load(operator_file.read_text(encoding="utf-8"))
        fragments = {fragment["name"]: fragment for fragment in document["fragments"]}
        expr_name = f"op_{step_name.replace('.', '_')}_expr"

        assert fragments[expr_name]["value"]["operator_id"] == expected_operator_ids[step_name]

    legacy_operator_files = {
        "parse_artifacts": operators_dir / "parse_artifacts_operator.yaml",
        "lift_fragments": operators_dir / "lift_fragments_operator.yaml",
    }
    for operator_name, operator_file in legacy_operator_files.items():
        document = yaml.safe_load(operator_file.read_text(encoding="utf-8"))
        fragments = {fragment["name"]: fragment for fragment in document["fragments"]}
        assert fragments[f"op_{operator_name}_expr"]["value"]["operator_id"] == f"modelado/operators/{operator_name}"

    net_document = yaml.safe_load((operators_dir / "ingestion_net_v2.yaml").read_text(encoding="utf-8"))
    net_fragments = {fragment["name"]: fragment for fragment in net_document["fragments"]}

    assert net_fragments["net.ingestion.link.load_documents_exec"]["value"]["object"] == "${ref:op_load_documents_expr}"
    assert net_fragments["net.ingestion.link.parse_chunk_exec"]["value"]["object"] == "${ref:op_parse_chunk_expr}"
    assert (
        net_fragments["net.ingestion.link.parse_entities_and_relationships_exec"]["value"]["object"]
        == "${ref:op_parse_entities_and_relationships_expr}"
    )
    assert net_fragments["net.ingestion.link.parse_claims_exec"]["value"]["object"] == "${ref:op_parse_claims_expr}"
