"""Tests for relation evaluation modes (deterministic and non-deterministic).

Per IKAM_FRAGMENT_ALGEBRA_V3.md §3.3-§3.4:
- eval(r, g, env) = out
- Deterministic: same slots + same function spec => same output bytes
- Non-deterministic: output may vary unless environment is fixed
- Environment metadata SHALL be sufficient for replay
- Evaluation outcome persisted as provenance event
"""

import uuid
import pytest

from ikam.fragments import (
    Fragment,
    Relation,
    SlotBinding,
    BindingGroup,
    RELATION_MIME,
)


def _make_relation_fragment(
    predicate: str,
    binding_groups: list[BindingGroup],
    function_cas_id: str | None = None,
    output_mime_type: str | None = None,
) -> Fragment:
    rel = Relation(
        predicate=predicate,
        binding_groups=binding_groups,
        function_cas_id=function_cas_id,
        output_mime_type=output_mime_type,
    )
    return Fragment(
        cas_id=f"rel_{uuid.uuid4().hex[:12]}",
        value=rel.model_dump(),
        mime_type=RELATION_MIME,
    )


class TestDeterministicEvaluation:
    """Deterministic function: same inputs => same output bytes."""

    def test_deterministic_eval_returns_output_fragment(self):
        from ikam.relation_eval import evaluate_relation

        # A simple deterministic function that concatenates slot values
        def concat_fn(slots: dict[str, str], env: dict) -> bytes:
            return b"|".join(v.encode() for v in sorted(slots.values()))

        bg = BindingGroup(
            invocation_id="inv_det_1",
            slots=[
                SlotBinding(slot="a", fragment_id="frag_alpha"),
                SlotBinding(slot="b", fragment_id="frag_beta"),
            ],
        )
        rel_frag = _make_relation_fragment("concat", [bg], function_cas_id="fn_concat")

        result = evaluate_relation(
            rel_frag,
            invocation_id="inv_det_1",
            function_registry={"fn_concat": concat_fn},
        )

        assert result.output_bytes == b"frag_alpha|frag_beta"
        assert result.deterministic is True

    def test_deterministic_eval_same_inputs_same_output(self):
        from ikam.relation_eval import evaluate_relation

        def identity_fn(slots: dict[str, str], env: dict) -> bytes:
            return slots["x"].encode()

        bg = BindingGroup(
            invocation_id="inv_stable",
            slots=[SlotBinding(slot="x", fragment_id="frag_X")],
        )
        rel_frag = _make_relation_fragment("identity", [bg], function_cas_id="fn_id")
        registry = {"fn_id": identity_fn}

        r1 = evaluate_relation(rel_frag, invocation_id="inv_stable", function_registry=registry)
        r2 = evaluate_relation(rel_frag, invocation_id="inv_stable", function_registry=registry)

        assert r1.output_bytes == r2.output_bytes


class TestNonDeterministicEvaluation:
    """Non-deterministic: output varies unless environment is fixed."""

    def test_non_deterministic_captures_environment(self):
        from ikam.relation_eval import evaluate_relation

        call_count = 0

        def seeded_fn(slots: dict[str, str], env: dict) -> bytes:
            nonlocal call_count
            call_count += 1
            seed = env.get("seed", 0)
            return f"out_{seed}_{call_count}".encode()

        bg = BindingGroup(
            invocation_id="inv_nd",
            slots=[SlotBinding(slot="input", fragment_id="frag_in")],
        )
        rel_frag = _make_relation_fragment("generate", [bg], function_cas_id="fn_gen")

        result = evaluate_relation(
            rel_frag,
            invocation_id="inv_nd",
            function_registry={"fn_gen": seeded_fn},
            environment={"seed": 42},
            deterministic=False,
        )

        assert result.output_bytes is not None
        assert result.deterministic is False
        assert result.environment == {"seed": 42}

    def test_missing_invocation_id_raises(self):
        from ikam.relation_eval import evaluate_relation

        bg = BindingGroup(
            invocation_id="inv_only",
            slots=[SlotBinding(slot="x", fragment_id="f1")],
        )
        rel_frag = _make_relation_fragment("op", [bg], function_cas_id="fn_x")

        with pytest.raises(ValueError, match="invocation_id"):
            evaluate_relation(
                rel_frag,
                invocation_id="inv_WRONG",
                function_registry={"fn_x": lambda s, e: b"x"},
            )


class TestEvaluationProvenanceRecord:
    """Evaluation produces a provenance-compatible event dict."""

    def test_provenance_event_shape(self):
        from ikam.relation_eval import evaluate_relation

        def noop_fn(slots: dict[str, str], env: dict) -> bytes:
            return b"output"

        bg = BindingGroup(
            invocation_id="inv_prov",
            slots=[SlotBinding(slot="in", fragment_id="frag_p")],
        )
        rel_frag = _make_relation_fragment("prov_test", [bg], function_cas_id="fn_noop")

        result = evaluate_relation(
            rel_frag,
            invocation_id="inv_prov",
            function_registry={"fn_noop": noop_fn},
        )

        event = result.to_provenance_event()
        assert event["action"] == "evaluated"
        assert event["fragment_id"] == rel_frag.cas_id
        assert event["metadata"]["invocation_id"] == "inv_prov"
        assert event["metadata"]["output_cas_id"] is not None
        assert "environment" in event["metadata"]
