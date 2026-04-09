from __future__ import annotations

from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/modelado/src"))

import pytest  # noqa: E402

from modelado.environment_scope import EnvironmentScope  # noqa: E402


def test_parse_explicit_artifact_locator() -> None:
    from modelado.history.head_locators import parse_head_locator

    locator = parse_head_locator("ref://refs/heads/main/artifact/artifact-semantic-1")

    assert locator.kind == "artifact"
    assert locator.ref == "refs/heads/main"
    assert locator.semantic_id == "artifact-semantic-1"
    assert locator.explicit_ref is True


def test_parse_shorthand_artifact_locator() -> None:
    from modelado.history.head_locators import parse_head_locator

    locator = parse_head_locator("artifact://artifact-semantic-1")

    assert locator.kind == "artifact"
    assert locator.ref is None
    assert locator.semantic_id == "artifact-semantic-1"
    assert locator.explicit_ref is False


def test_parse_explicit_subgraph_locator() -> None:
    from modelado.history.head_locators import parse_head_locator

    locator = parse_head_locator("ref://refs/heads/run/demo/subgraph/subgraph-semantic-1")

    assert locator.kind == "subgraph"
    assert locator.ref == "refs/heads/run/demo"
    assert locator.semantic_id == "subgraph-semantic-1"
    assert locator.explicit_ref is True


def test_parse_explicit_artifact_locator_with_subgraph_in_ref_name() -> None:
    from modelado.history.head_locators import parse_head_locator

    locator = parse_head_locator("ref://refs/heads/feature/subgraph/demo/artifact/artifact-semantic-1")

    assert locator.kind == "artifact"
    assert locator.ref == "refs/heads/feature/subgraph/demo"
    assert locator.semantic_id == "artifact-semantic-1"
    assert locator.explicit_ref is True


def test_parse_shorthand_subgraph_locator() -> None:
    from modelado.history.head_locators import parse_head_locator

    locator = parse_head_locator("subgraph://subgraph-semantic-1")

    assert locator.kind == "subgraph"
    assert locator.ref is None
    assert locator.semantic_id == "subgraph-semantic-1"
    assert locator.explicit_ref is False


def test_parse_explicit_fragment_locator() -> None:
    from modelado.history.head_locators import parse_head_locator

    locator = parse_head_locator("ref://refs/heads/main/fragment/frag-semantic-1")

    assert locator.kind == "fragment"
    assert locator.ref == "refs/heads/main"
    assert locator.semantic_id == "frag-semantic-1"
    assert locator.explicit_ref is True


def test_parse_shorthand_fragment_locator() -> None:
    from modelado.history.head_locators import parse_head_locator

    locator = parse_head_locator("fragment://frag-semantic-1")

    assert locator.kind == "fragment"
    assert locator.ref is None
    assert locator.semantic_id == "frag-semantic-1"
    assert locator.explicit_ref is False


def test_shorthand_artifact_locator_uses_current_env_ref_only() -> None:
    from modelado.history.head_locators import resolve_head_locator

    resolved = resolve_head_locator(
        "artifact://artifact-semantic-1",
        env_scope=EnvironmentScope(ref="refs/heads/feature/demo"),
    )

    assert resolved.kind == "artifact"
    assert resolved.ref == "refs/heads/feature/demo"
    assert resolved.semantic_id == "artifact-semantic-1"
    assert resolved.explicit_ref is False


def test_shorthand_fragment_locator_uses_current_env_ref_only() -> None:
    from modelado.history.head_locators import resolve_head_locator

    resolved = resolve_head_locator(
        "fragment://frag-semantic-1",
        env_scope=EnvironmentScope(ref="refs/heads/feature/demo"),
    )

    assert resolved.kind == "fragment"
    assert resolved.ref == "refs/heads/feature/demo"
    assert resolved.semantic_id == "frag-semantic-1"
    assert resolved.explicit_ref is False


def test_canonicalize_locator_ref_preserves_explicit_ref() -> None:
    from modelado.history.head_locators import canonicalize_locator_ref

    assert canonicalize_locator_ref("ref://refs/heads/main/subgraph/run-1") == "ref://refs/heads/main/subgraph/run-1"


def test_canonicalize_locator_ref_normalizes_shorthand_fragment() -> None:
    from modelado.history.head_locators import canonicalize_locator_ref

    assert canonicalize_locator_ref("fragment://frag-semantic-1") == "fragment://frag-semantic-1"


def test_try_canonicalize_locator_ref_returns_none_for_non_matching_kind() -> None:
    from modelado.history.head_locators import try_canonicalize_locator_ref

    assert try_canonicalize_locator_ref("artifact://artifact-semantic-1", kind="subgraph") is None


def test_resolve_locator_identity_falls_back_for_non_locator_raw_id() -> None:
    from modelado.history.head_locators import resolve_locator_identity

    assert resolve_locator_identity("plain-artifact-id", fallback_kind="artifact") == ("artifact", "plain-artifact-id")


def test_resolve_graph_target_input_preserves_plain_artifact_id() -> None:
    from modelado.history.head_locators import resolve_graph_target_input

    target = resolve_graph_target_input(artifact_id="plain-artifact-id", target_ref=None, env_scope=None, cx=None)

    assert target is not None
    assert target.kind == "artifact"
    assert target.target_id == "plain-artifact-id"
    assert target.target_ref == "plain-artifact-id"


def test_shorthand_artifact_locator_does_not_fallback_to_base_refs() -> None:
    from modelado.history.head_locators import resolve_head_locator

    with pytest.raises(TypeError):
        resolve_head_locator(
            "artifact://artifact-semantic-1",
            env_scope=EnvironmentScope(ref="refs/heads/feature/demo"),
            base_refs=["refs/heads/main"],
        )
