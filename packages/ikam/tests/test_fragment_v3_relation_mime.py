"""Tests for V3 relation-as-fragment MIME semantics.

Covers conformance requirement V3-FRAG-2:
- Relation payload round-trip through Fragment value
- RELATION_MIME constant exists and equals 'application/ikam-relation+json'
- is_relation_fragment() helper works correctly
- SlotBinding, BindingGroup, Relation models exist and validate
"""
from __future__ import annotations

import json
import pytest


def test_relation_mime_constant():
    """RELATION_MIME is defined as 'application/ikam-relation+json'."""
    from ikam.fragments import RELATION_MIME

    assert RELATION_MIME == "application/ikam-relation+json"


def test_relation_payload_roundtrip():
    """A relation payload round-trips through Fragment value."""
    from ikam.fragments import Fragment, Relation, SlotBinding, BindingGroup, RELATION_MIME

    rel = Relation(
        predicate="depends_on",
        directed=True,
        confidence_score=0.9,
        binding_groups=[
            BindingGroup(
                invocation_id="inv-001",
                slots=[
                    SlotBinding(slot="source", fragment_id="frag-a"),
                    SlotBinding(slot="target", fragment_id="frag-b"),
                ],
            )
        ],
    )

    # Store relation as fragment value
    frag = Fragment(
        value=rel.model_dump(mode="json"),
        mime_type=RELATION_MIME,
    )

    # Round-trip: parse the value back
    recovered = Relation.model_validate(frag.value)
    assert recovered.predicate == "depends_on"
    assert recovered.directed is True
    assert recovered.confidence_score == 0.9
    assert len(recovered.binding_groups) == 1
    assert recovered.binding_groups[0].invocation_id == "inv-001"
    assert len(recovered.binding_groups[0].slots) == 2


def test_is_relation_fragment():
    """is_relation_fragment() identifies relation fragments by MIME type."""
    from ikam.fragments import Fragment, is_relation_fragment, RELATION_MIME

    rel_frag = Fragment(value={"predicate": "test"}, mime_type=RELATION_MIME)
    plain_frag = Fragment(cas_id="abc123", mime_type="text/plain")

    assert is_relation_fragment(rel_frag) is True
    assert is_relation_fragment(plain_frag) is False


def test_slot_binding_model():
    """SlotBinding requires slot and fragment_id."""
    from ikam.fragments import SlotBinding

    sb = SlotBinding(slot="input", fragment_id="frag-x")
    assert sb.slot == "input"
    assert sb.fragment_id == "frag-x"


def test_binding_group_model():
    """BindingGroup requires invocation_id and list of slots."""
    from ikam.fragments import BindingGroup, SlotBinding

    bg = BindingGroup(
        invocation_id="inv-001",
        slots=[SlotBinding(slot="a", fragment_id="frag-1")],
    )
    assert bg.invocation_id == "inv-001"
    assert len(bg.slots) == 1


def test_relation_with_function_cas_id():
    """Relation with function_cas_id references an executable spec."""
    from ikam.fragments import Relation

    rel = Relation(
        predicate="transform",
        function_cas_id="deadbeef" * 8,
        output_mime_type="application/json",
    )
    assert rel.function_cas_id == "deadbeef" * 8
    assert rel.output_mime_type == "application/json"


def test_relation_pure_semantic():
    """Relation without function_cas_id is a pure semantic relation."""
    from ikam.fragments import Relation

    rel = Relation(predicate="supports")
    assert rel.function_cas_id is None
    assert rel.output_mime_type is None
    assert rel.binding_groups == []
