"""Contract-level tests for simulator ScenarioFragment shape and deltas."""

from modelado.sequencer.simulator import run_scenario_analysis, ScenarioFragment, PhaseModification


class _MockPhase:
    def __init__(self, phase_id: str, effort: float, assignees=None):
        self.id = phase_id
        self.estimated_effort = effort
        self.assignees = assignees or ["dev"]
        self.risk_score = 0.3


class _MockFragment:
    def __init__(self):
        self.id = "seq-frag-test"
        self.phases = [
            _MockPhase("phase-1", 10.0, ["dev"]),
            _MockPhase("phase-2", 12.0, ["dev", "qa"]),
        ]


def test_run_scenario_analysis_returns_fragment_with_modifications():
    base_fragment = _MockFragment()

    result = run_scenario_analysis(
        base_fragment=base_fragment,
        scenario_description="Hire a contractor for phase 2 to parallelize",
    )

    assert isinstance(result, ScenarioFragment)
    assert result.modifications, "Expected at least one phase modification"
    assert all(isinstance(m, PhaseModification) for m in result.modifications)
    assert "operation" in result.scenario_intent
    assert result.effort_delta != 0 or result.cost_delta != 0 or result.duration_delta != 0
    assert isinstance(result.rationale, str) and result.rationale
    assert isinstance(result.recommendations, list)
