from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest
from dotenv import load_dotenv

# Load the package .env (same file Docker uses via env_file:) so that
# OPENAI_API_KEY is available when pytest collects test modules that
# import ``ikam_perf_report.main`` (which validates the key at import time).
load_dotenv(Path(__file__).resolve().parent.parent / ".env")


@pytest.fixture
def case_fixtures_root(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> Path:
    root = tmp_path / "cases"
    root.mkdir(parents=True)
    registry = {
        "version": "test",
        "cases": [
            {
                "case_id": "s-construction-v01",
                "domain": "construction",
                "size_tier": "s",
                "idea_file": "idea.md",
            }
        ],
    }
    (root / "_case-registry.json").write_text(json.dumps(registry), encoding="utf-8")
    (root / ".ikamignore").write_text(
        "*.py\n*.pyc\n__pycache__/\n.venv/\nidea.md\n.DS_Store\n",
        encoding="utf-8",
    )

    case_dir = root / "s-construction-v01"
    case_dir.mkdir()
    (case_dir / "idea.md").write_text("Revenue assumptions and plan.", encoding="utf-8")
    (case_dir / "revenue_plan.md").write_text(
        "# Revenue Plan\n\nCore revenue assumptions and growth plan projections.",
        encoding="utf-8",
    )
    (case_dir / "metrics.json").write_text('{"arr": 100}', encoding="utf-8")
    (case_dir / "msa.pdf").write_bytes(b"%PDF-1.4 fake")
    (case_dir / "assets").mkdir()
    (case_dir / "assets" / "image.png").write_bytes(b"\x89PNG\r\n")

    monkeypatch.setenv("IKAM_CASES_ROOT", str(root))
    return root


@pytest.fixture
def oracle_text(case_fixtures_root: Path) -> str:
    """Load idea.md oracle text for evaluation (never ingested)."""
    idea_path = case_fixtures_root / "s-construction-v01" / "idea.md"
    return idea_path.read_text(encoding="utf-8")


@pytest.fixture(autouse=True)
def stub_mcp_map_generation(monkeypatch: pytest.MonkeyPatch) -> None:
    """Use deterministic in-process MCP payload for perf-report tests.

    The integration path remains MCP-shaped (`_invoke_mcp_map_generation`),
    while tests avoid network/service dependencies.
    """

    from ikam.forja import debug_execution

    def _stub(
        *,
        artifact_id: str,
        assets: list[dict[str, Any]],
        **kwargs: Any,
    ) -> dict[str, Any]:
        root_id = f"map:{artifact_id}:root"
        map_nodes: list[dict[str, Any]] = [
            {
                "id": root_id,
                "title": "Corpus Outline",
                "kind": "corpus",
                "level": 0,
                "parent_id": None,
                "artifact_ids": [],
            }
        ]
        outline_nodes: list[dict[str, Any]] = [
            {
                "id": root_id,
                "title": "Corpus Outline",
                "kind": "corpus",
                "level": 0,
                "parent_id": None,
                "artifact_ids": [],
            }
        ]
        node_summaries: dict[str, str] = {root_id: "Corpus outline generated via MCP contract."}
        node_constituents: dict[str, list[str]] = {root_id: []}
        relationships: list[dict[str, str]] = []
        segment_candidates: list[dict[str, Any]] = []
        segment_anchors: dict[str, list[dict[str, Any]]] = {}
        profile_candidates: dict[str, list[str]] = {}

        grouped: dict[str, list[dict[str, Any]]] = {}
        for asset in assets:
            if not isinstance(asset, dict):
                continue
            fid = str(asset.get("artifact_id") or "").strip()
            if not fid:
                continue
            grouped.setdefault(fid, []).append(asset)

        for owner, owner_assets in sorted(grouped.items()):
            artifact_node_id = f"map:artifact:{owner}"
            owner_title = str(owner_assets[0].get("filename") or owner)
            outline_nodes.append(
                {
                    "id": artifact_node_id,
                    "title": owner_title,
                    "kind": "artifact",
                    "level": 1,
                    "parent_id": root_id,
                    "artifact_ids": [owner],
                }
            )
            map_nodes.append(
                {
                    "id": artifact_node_id,
                    "title": owner_title,
                    "kind": "segment",
                    "level": 1,
                    "parent_id": root_id,
                    "artifact_ids": [owner],
                }
            )
            outline_nodes[0]["artifact_ids"].append(owner)
            node_summaries[artifact_node_id] = f"Artifact {owner} with semantic segment candidate(s)."
            node_constituents[artifact_node_id] = [owner]
            node_constituents[root_id].append(owner)
            relationships.append({"type": "map_contains", "source": root_id, "target": artifact_node_id})
            relationships.append({"type": "map_to_artifact", "source": artifact_node_id, "target": f"artifact:{owner}"})
            segment_candidates.append(
                {
                    "segment_id": artifact_node_id,
                    "title": owner_title,
                    "artifact_ids": [owner],
                    "rationale": "stub segment candidate for map similarity stage",
                }
            )
            segment_anchors[artifact_node_id] = [
                {
                    "artifact_id": owner,
                    "locator_type": "artifact",
                    "locator": owner_title,
                    "confidence": 0.9,
                }
            ]
            profile_candidates[artifact_node_id] = ["modelado/prose-backbone@1", "modelado/reasoning@1"]

        return {
            "map_subgraph": {
                "root_node_id": root_id,
                "nodes": map_nodes,
                "relationships": [
                    item for item in relationships if item.get("type") == "map_contains"
                ],
            },
            "map_dna": {
                "fingerprint": f"stub:{artifact_id}",
                "structural_hashes": [],
                "version": "1",
            },
            "segment_anchors": segment_anchors,
            "segment_candidates": segment_candidates,
            "profile_candidates": profile_candidates,
            "generation_provenance": {
                "provider": "test-stub",
                "model": "gpt-4o-mini",
                "prompt_version": "map-v2",
                "temperature": 0.0,
                "seed": 0,
            },
        }

    monkeypatch.setattr(debug_execution, "_invoke_mcp_map_generation", _stub, raising=False)

@pytest.fixture(autouse=True)
def disable_async_next_step(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("IKAM_ASYNC_NEXT_STEP", "0")
