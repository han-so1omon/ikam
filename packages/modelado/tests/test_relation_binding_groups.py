"""Tests for relation binding group resolution and slot binding.

Per IKAM_FRAGMENT_ALGEBRA_V3.md §2.2 and §3.3:
- A relation fragment can have multiple binding groups
- Each binding group has an invocation_id and slot bindings
- Slots bind named inputs to source fragment identities
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
    """Build a V3 relation fragment with a Relation payload."""
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


class TestRelationBindingGroupResolution:
    """Validate binding group extraction and slot resolution from relation fragments."""

    def test_extract_relation_payload(self):
        from ikam.relation_eval import extract_relation

        bg = BindingGroup(
            invocation_id="inv_1",
            slots=[SlotBinding(slot="input", fragment_id="frag_abc")],
        )
        frag = _make_relation_fragment("composes", [bg], function_cas_id="fn_hash")
        rel = extract_relation(frag)

        assert isinstance(rel, Relation)
        assert rel.predicate == "composes"
        assert len(rel.binding_groups) == 1
        assert rel.binding_groups[0].invocation_id == "inv_1"
        assert rel.function_cas_id == "fn_hash"

    def test_extract_relation_rejects_non_relation_fragment(self):
        from ikam.relation_eval import extract_relation

        frag = Fragment(cas_id="plain_frag", mime_type="text/plain")
        with pytest.raises(ValueError, match="not a relation fragment"):
            extract_relation(frag)

    def test_multiple_binding_groups(self):
        from ikam.relation_eval import extract_relation

        bgs = [
            BindingGroup(
                invocation_id="inv_A",
                slots=[SlotBinding(slot="x", fragment_id="f1")],
            ),
            BindingGroup(
                invocation_id="inv_B",
                slots=[
                    SlotBinding(slot="x", fragment_id="f2"),
                    SlotBinding(slot="y", fragment_id="f3"),
                ],
            ),
        ]
        frag = _make_relation_fragment("transforms", bgs, function_cas_id="fn_t")
        rel = extract_relation(frag)

        assert len(rel.binding_groups) == 2
        assert {bg.invocation_id for bg in rel.binding_groups} == {"inv_A", "inv_B"}
        assert len(rel.binding_groups[1].slots) == 2

    def test_resolve_slots_returns_fragment_ids_by_name(self):
        from ikam.relation_eval import resolve_slots

        bg = BindingGroup(
            invocation_id="inv_1",
            slots=[
                SlotBinding(slot="left", fragment_id="frag_L"),
                SlotBinding(slot="right", fragment_id="frag_R"),
            ],
        )
        resolved = resolve_slots(bg)
        assert resolved == {"left": "frag_L", "right": "frag_R"}
