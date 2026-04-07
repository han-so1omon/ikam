"""Tests for V3 provenance projection: latest-evaluation lookup.

Per IKAM_FRAGMENT_ALGEBRA_V3.md §5:
- Source of truth: append-only ProvenanceEvent log with action='evaluated'
- Fast lookup: latest_output(relation_fragment_id, invocation_id) -> Optional[output_cas_id]
- Ordering key: provenance timestamp (monotonic)
- Rebuildability: projection can be reconstructed from provenance log only

These tests validate the projection module without requiring a database —
the projection operates over in-memory event lists.
"""

import datetime as dt
import uuid

import pytest


def _ts(offset_ms: int = 0) -> dt.datetime:
    """Create a UTC timestamp with optional millisecond offset for ordering."""
    return dt.datetime(2026, 2, 8, 12, 0, 0, tzinfo=dt.timezone.utc) + dt.timedelta(milliseconds=offset_ms)


def _eval_event(
    relation_fragment_id: str,
    invocation_id: str,
    output_cas_id: str,
    timestamp: dt.datetime | None = None,
    environment: dict | None = None,
) -> dict:
    """Build a provenance event dict matching V3 'evaluated' action shape."""
    return {
        "id": str(uuid.uuid4()),
        "fragment_id": relation_fragment_id,
        "action": "evaluated",
        "timestamp": timestamp or _ts(),
        "metadata": {
            "invocation_id": invocation_id,
            "output_cas_id": output_cas_id,
            "environment": environment or {},
        },
    }


class TestLatestOutputProjection:
    """Projection returns the most recent output_cas_id for a (relation, invocation) pair."""

    def test_single_evaluation_returns_output(self):
        from modelado.provenance_views import LatestOutputProjection

        proj = LatestOutputProjection()
        rel_id = "rel_abc"
        inv_id = "inv_001"
        event = _eval_event(rel_id, inv_id, "out_hash_1", _ts(0))
        proj.apply(event)

        result = proj.latest_output(rel_id, inv_id)
        assert result == "out_hash_1"

    def test_later_evaluation_overwrites_earlier(self):
        from modelado.provenance_views import LatestOutputProjection

        proj = LatestOutputProjection()
        rel_id = "rel_abc"
        inv_id = "inv_001"
        proj.apply(_eval_event(rel_id, inv_id, "out_v1", _ts(0)))
        proj.apply(_eval_event(rel_id, inv_id, "out_v2", _ts(100)))

        assert proj.latest_output(rel_id, inv_id) == "out_v2"

    def test_different_invocations_independent(self):
        from modelado.provenance_views import LatestOutputProjection

        proj = LatestOutputProjection()
        rel_id = "rel_abc"
        proj.apply(_eval_event(rel_id, "inv_A", "out_A", _ts(0)))
        proj.apply(_eval_event(rel_id, "inv_B", "out_B", _ts(10)))

        assert proj.latest_output(rel_id, "inv_A") == "out_A"
        assert proj.latest_output(rel_id, "inv_B") == "out_B"

    def test_missing_key_returns_none(self):
        from modelado.provenance_views import LatestOutputProjection

        proj = LatestOutputProjection()
        assert proj.latest_output("no_such_rel", "no_such_inv") is None

    def test_ignores_non_evaluated_events(self):
        from modelado.provenance_views import LatestOutputProjection

        proj = LatestOutputProjection()
        non_eval = {
            "id": str(uuid.uuid4()),
            "fragment_id": "rel_xyz",
            "action": "Created",
            "timestamp": _ts(0),
            "metadata": {},
        }
        proj.apply(non_eval)
        assert proj.latest_output("rel_xyz", "any") is None


class TestRebuildProjection:
    """Projection rebuilds correctly from an ordered event stream."""

    def test_rebuild_from_event_list(self):
        from modelado.provenance_views import LatestOutputProjection, rebuild_projection

        rel_id = "rel_rebuild"
        events = [
            _eval_event(rel_id, "inv_1", "out_old", _ts(0)),
            _eval_event(rel_id, "inv_1", "out_new", _ts(50)),
            _eval_event(rel_id, "inv_2", "out_two", _ts(100)),
        ]
        proj = rebuild_projection(events)
        assert isinstance(proj, LatestOutputProjection)
        assert proj.latest_output(rel_id, "inv_1") == "out_new"
        assert proj.latest_output(rel_id, "inv_2") == "out_two"

    def test_rebuild_empty_events_produces_empty_projection(self):
        from modelado.provenance_views import LatestOutputProjection, rebuild_projection

        proj = rebuild_projection([])
        assert isinstance(proj, LatestOutputProjection)
        assert proj.latest_output("any", "any") is None

    def test_rebuild_preserves_environment_metadata(self):
        from modelado.provenance_views import LatestOutputProjection, rebuild_projection

        rel_id = "rel_env"
        env = {"seed": 42, "model_version": "v1.0"}
        events = [
            _eval_event(rel_id, "inv_1", "out_1", _ts(0), environment=env),
        ]
        proj = rebuild_projection(events)
        entry = proj.get_entry(rel_id, "inv_1")
        assert entry is not None
        assert entry.output_cas_id == "out_1"
        assert entry.environment == env
