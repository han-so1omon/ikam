"""
Comprehensive tests for generative scenario analysis (Story #20).

Tests cover:
1. Scenario intent parsing (5+ semantic variations)
2. Scenario analyzer generation (5 operation types + novel)
3. Full scenario analysis workflow with deltas
4. Edge cases and novel operations
5. Provenance and rationale generation
"""

import pytest
from datetime import datetime
from modelado.sequencer.simulator import (
    parse_scenario_intent,
    generate_scenario_analyzer,
    run_scenario_analysis,
    ScenarioFragment,
    PhaseModification,
    _extract_phases_from_description,
    _generate_scenario_rationale,
    _generate_recommendations,
)


# ============================================================================
# Mock Base Fragment for Testing
# ============================================================================

class MockPhase:
    """Mock PlanPhase for testing."""
    
    def __init__(self, phase_id: str, effort: float, assignees=None, risk=0.3):
        self.id = phase_id
        self.estimated_effort = effort
        self.assignees = assignees or ["dev"]
        self.risk_score = risk


class MockFragment:
    """Mock SequencerFragment for testing."""
    
    def __init__(self, phases=None):
        self.id = "base-fragment-1"
        # Only use default phases if phases is None, not if it's an empty list
        if phases is None:
            self.phases = [
                MockPhase("phase-1", 100.0, ["dev"], 0.2),
                MockPhase("phase-2", 150.0, ["dev", "qa"], 0.3),
                MockPhase("phase-3", 80.0, ["dev"], 0.4),
            ]
        else:
            self.phases = phases


# ============================================================================
# Tests: Scenario Intent Parsing
# ============================================================================

class TestScenarioIntentParsing:
    """Test semantic intent parsing from natural language."""
    
    def test_parse_hire_contractor_basic(self):
        """Parse 'hire a contractor' intent."""
        intent = parse_scenario_intent("What if we hire a single consultant for phase 2?")
        
        assert intent["operation"] == "resource_substitution"
        # Sync wrapper uses keyword fallback (lower confidence)
        assert intent["confidence"] <= 0.75
        assert "phase-2" in intent["target_phases"]
        assert intent["parameters"]["cost_multiplier"] == 1.4
        # Single contractor should have 0.85 effort reduction
        assert intent["parameters"]["effort_reduction"] == 0.85
    
    def test_parse_hire_contractor_offshore(self):
        """Parse 'hire offshore contractor' (cheaper variant)."""
        intent = parse_scenario_intent("Hire an offshore contractor from India")
        
        assert intent["operation"] == "resource_substitution"
        assert intent["parameters"]["cost_multiplier"] == 1.2  # cheaper offshore
        assert intent["parameters"]["effort_reduction"] == 0.85
    
    def test_parse_hire_contractor_senior(self):
        """Parse 'hire senior/expert contractor' (expensive variant)."""
        intent = parse_scenario_intent("Hire a senior expert contractor")
        
        assert intent["operation"] == "resource_substitution"
        assert intent["parameters"]["cost_multiplier"] == 1.6  # more expensive expert
    
    def test_parse_hire_multiple_contractors(self):
        """Parse 'hire 2 contractors' (distributed resources)."""
        intent = parse_scenario_intent("Hire 2 contractors for backend and QA")
        
        assert intent["operation"] == "resource_substitution"
        assert intent["parameters"]["effort_reduction"] == 0.75  # more efficient with multiple
    
    def test_parse_scope_reduction(self):
        """Parse 'reduce scope' intent."""
        intent = parse_scenario_intent("Cut scope by 25% to fit features")
        
        assert intent["operation"] == "scope_reduction"
        # Sync wrapper uses keyword fallback
        assert intent["confidence"] == 0.80
        assert intent["parameters"]["reduction_percent"] == 0.25
        assert intent["target_phases"] == ["all"]
    
    def test_parse_parallelize(self):
        """Parse 'parallelize phases' intent."""
        intent = parse_scenario_intent("Can we parallelize phases 1-3?")
        
        assert intent["operation"] == "parallelize"
        assert "phase-1" in intent["target_phases"]
        assert "phase-2" in intent["target_phases"]
        assert "phase-3" in intent["target_phases"]
    
    def test_parse_quality_enhancement(self):
        """Parse 'add testing' intent."""
        intent = parse_scenario_intent("Add comprehensive testing to the plan")
        
        assert intent["operation"] == "quality_enhancement"
        assert intent["parameters"]["testing_depth"] == "comprehensive"
        assert intent["parameters"]["effort_multiplier"] == 1.3
    
    def test_parse_cost_optimization(self):
        """Parse 'reduce cost' intent."""
        intent = parse_scenario_intent("Reduce total project cost by $50K")
        
        assert intent["operation"] == "cost_optimization"
        assert intent["parameters"]["target_savings"] == 50000
        assert intent["parameters"]["preserve_timeline"] == True
    
    def test_parse_novel_scenario(self):
        """Parse unrecognized scenario type (generic handler)."""
        intent = parse_scenario_intent("Use AI to auto-generate code where possible")
        
        assert intent["operation"] == "custom_adjustment"
        assert intent["confidence"] == 0.50  # lower confidence for novel (fallback)
        assert intent["parameters"]["description"] == "Use AI to auto-generate code where possible"
    
    def test_parse_empty_description(self):
        """Parse empty scenario (edge case)."""
        intent = parse_scenario_intent("")
        
        assert intent["operation"] == "custom_adjustment"
        assert intent["confidence"] == 0.50
        assert intent["target_phases"] == []
    
    def test_extract_phases_from_description_single(self):
        """Extract single phase reference."""
        phases = _extract_phases_from_description("for phase 2")
        assert phases == ["phase-2"]
    
    def test_extract_phases_from_description_range(self):
        """Extract phase range (1-3)."""
        phases = _extract_phases_from_description("parallelize phases 1-3")
        assert set(phases) == {"phase-1", "phase-2", "phase-3"}
    
    def test_extract_phases_from_description_multiple(self):
        """Extract multiple phase references."""
        phases = _extract_phases_from_description("phase 1, phase 3, phase 5")
        assert set(phases) == {"phase-1", "phase-3", "phase-5"}
    
    def test_extract_phases_no_matches(self):
        """Extract phases when none are mentioned."""
        phases = _extract_phases_from_description("some other scenario")
        assert phases == []


# ============================================================================
# Tests: Scenario Analyzer Generation
# ============================================================================

class TestScenarioAnalyzerGeneration:
    """Test generative creation of scenario analyzer functions."""
    
    def test_generate_resource_substitution_analyzer(self):
        """Generate analyzer for resource substitution."""
        intent = parse_scenario_intent("Hire a contractor")
        analyzer = generate_scenario_analyzer(intent)
        
        fragment = MockFragment()
        modifications = analyzer(fragment.phases)
        
        # Should modify some phases
        assert len(modifications) > 0
        
        # Check modification structure
        mod = modifications[0]
        assert mod.phase_id == "phase-1"
        assert mod.modified_cost_multiplier == 1.4
        assert mod.modified_effort < fragment.phases[0].estimated_effort  # Effort reduced
        assert mod.modified_risk_delta > 0  # Risk increased slightly
    
    def test_generate_scope_reduction_analyzer(self):
        """Generate analyzer for scope reduction."""
        intent = parse_scenario_intent("Reduce scope by 25%")
        analyzer = generate_scenario_analyzer(intent)
        
        fragment = MockFragment()
        modifications = analyzer(fragment.phases)
        
        # Should modify all phases (scope reduction is global)
        assert len(modifications) == len(fragment.phases)
        
        # Check effort reduction
        for i, mod in enumerate(modifications):
            assert mod.modified_effort == fragment.phases[i].estimated_effort * 0.75  # 1 - 0.25
            assert mod.modified_risk_delta < 0  # Risk decreases
    
    def test_generate_parallelize_analyzer(self):
        """Generate analyzer for parallelization."""
        intent = parse_scenario_intent("Parallelize phases 1-2")
        analyzer = generate_scenario_analyzer(intent)
        
        fragment = MockFragment()
        modifications = analyzer(fragment.phases)
        
        # Should create modifications for targeted phases
        assert len(modifications) >= 1
        for mod in modifications:
            if mod.phase_id in ["phase-1", "phase-2"]:
                assert mod.modified_risk_delta > 0  # Parallelization adds risk
    
    def test_generate_quality_enhancement_analyzer(self):
        """Generate analyzer for quality enhancement."""
        intent = parse_scenario_intent("Add comprehensive testing")
        analyzer = generate_scenario_analyzer(intent)
        
        fragment = MockFragment()
        modifications = analyzer(fragment.phases)
        
        # Should modify phases with testing additions
        for mod in modifications:
            assert mod.modified_effort > fragment.phases[0].estimated_effort  # Effort increased
            assert mod.modified_cost_multiplier == 1.2
            assert mod.modified_risk_delta < 0  # Quality reduces risk
    
    def test_generate_cost_optimization_analyzer(self):
        """Generate analyzer for cost optimization."""
        intent = parse_scenario_intent("Reduce cost by $50K")
        analyzer = generate_scenario_analyzer(intent)
        
        fragment = MockFragment()
        modifications = analyzer(fragment.phases)
        
        # Should modify costs
        for mod in modifications:
            assert mod.modified_cost_multiplier == 0.85
    
    def test_generate_novel_operation_analyzer(self):
        """Generate analyzer for novel/unknown operation."""
        intent = {
            "operation": "ai_code_generation",
            "target_phases": ["phase-2"],
            "parameters": {
                "effort_multiplier": 0.6,
                "cost_multiplier": 1.2,
                "risk_adjustment": 0.1,
            }
        }
        analyzer = generate_scenario_analyzer(intent)
        
        fragment = MockFragment()
        modifications = analyzer(fragment.phases)
        
        # Should apply generic modifications
        assert any(m.phase_id == "phase-2" for m in modifications)


# ============================================================================
# Tests: Full Scenario Analysis Workflow
# ============================================================================

class TestScenarioAnalysisWorkflow:
    """Test end-to-end scenario analysis."""
    
    def test_run_scenario_analysis_contractor(self):
        """Run full scenario analysis for contractor hiring."""
        fragment = MockFragment()
        scenario = run_scenario_analysis(
            fragment,
            "Hire a contractor for the implementation phase"
        )
        
        # Verify ScenarioFragment structure
        assert isinstance(scenario, ScenarioFragment)
        assert scenario.base_fragment_id == "base-fragment-1"
        assert scenario.scenario_description == "Hire a contractor for the implementation phase"
        assert scenario.scenario_intent["operation"] == "resource_substitution"
        
        # Verify deltas were computed
        assert isinstance(scenario.effort_delta, float)
        assert isinstance(scenario.cost_delta, float)
        assert isinstance(scenario.duration_delta, float)
        assert isinstance(scenario.risk_delta, float)
        
        # Verify modifications list
        assert len(scenario.modifications) > 0
        assert all(isinstance(m, PhaseModification) for m in scenario.modifications)
        
        # Verify rationale
        assert len(scenario.rationale) > 0
        assert "Scenario:" in scenario.rationale
        assert "Impact Summary:" in scenario.rationale
        
        # Verify recommendations
        assert isinstance(scenario.recommendations, list)
    
    def test_run_scenario_analysis_scope_reduction(self):
        """Run scenario analysis for scope reduction."""
        fragment = MockFragment()
        scenario = run_scenario_analysis(fragment, "Reduce scope by 20%")
        
        # Scope reduction should decrease effort
        assert scenario.effort_delta < 0
        # And decrease risk
        assert scenario.risk_delta < 0
    
    def test_run_scenario_analysis_cost_optimization(self):
        """Run scenario analysis for cost optimization."""
        fragment = MockFragment()
        scenario = run_scenario_analysis(fragment, "Reduce total cost by $50K")
        
        # Cost optimization should reduce cost
        assert scenario.cost_delta < 0
    
    def test_scenario_fragment_has_provenance(self):
        """Verify ScenarioFragment includes provenance fields."""
        fragment = MockFragment()
        scenario = run_scenario_analysis(fragment, "Hire contractor")
        
        # Provenance fields should be populated
        assert scenario.created_at is not None
        assert isinstance(scenario.created_at, datetime)
        assert scenario.scenario_intent is not None
        assert scenario.modifications is not None


# ============================================================================
# Tests: Rationale and Recommendations Generation
# ============================================================================

class TestRationaleAndRecommendations:
    """Test human-readable output generation."""
    
    def test_generate_scenario_rationale_basic(self):
        """Generate rationale for scenario impact."""
        rationale = _generate_scenario_rationale(
            operation="resource_substitution",
            modifications=[
                PhaseModification(
                    phase_id="phase-1",
                    modified_effort=85.0,
                    modified_risk_delta=0.05,
                    reason="Assigned contractor to accelerate"
                )
            ],
            effort_delta=-15.0,
            cost_delta=10000,
            duration_delta=-2.0,
            risk_delta=0.05,
        )
        
        assert "Resource Substitution" in rationale
        assert "Impact Summary:" in rationale
        assert "effort_delta" in rationale or "-15.0" in rationale
        assert "Modified Phases:" in rationale
    
    def test_generate_recommendations_high_cost_increase(self):
        """Generate recommendation for high cost increase."""
        recommendations = _generate_recommendations(
            operation="resource_substitution",
            effort_delta=50.0,
            cost_delta=100000,  # High cost increase
            duration_delta=5.0,
            risk_delta=0.05,
        )
        
        # Should recommend reconsidering scenario
        assert any("substantial" in r.lower() for r in recommendations)
    
    def test_generate_recommendations_risk_increase(self):
        """Generate recommendation for significant risk increase."""
        recommendations = _generate_recommendations(
            operation="parallelize",
            effort_delta=10.0,
            cost_delta=5000,
            duration_delta=-5.0,
            risk_delta=0.20,  # Significant risk increase
        )
        
        # Should recommend mitigation
        assert any("mitigation" in r.lower() for r in recommendations)
    
    def test_generate_recommendations_timeline_acceleration(self):
        """Generate recommendation for timeline acceleration."""
        recommendations = _generate_recommendations(
            operation="resource_substitution",
            effort_delta=-20.0,
            cost_delta=15000,
            duration_delta=-10.0,  # Meaningful acceleration
            risk_delta=-0.05,
        )
        
        # Should recommend verifying resource availability
        assert any("accelerated" in r.lower() or "acceleration" in r.lower() for r in recommendations)


# ============================================================================
# Tests: Edge Cases and Error Handling
# ============================================================================

class TestEdgeCases:
    """Test edge cases and boundary conditions."""
    
    def test_scenario_analysis_empty_phases(self):
        """Handle fragment with no phases."""
        fragment = MockFragment(phases=[])
        scenario = run_scenario_analysis(fragment, "Hire contractor")
        
        # Should handle gracefully even with empty phases
        # Since there are no phases to modify, modifications list should be empty
        assert isinstance(scenario, ScenarioFragment)
        assert scenario.modifications == []
        assert scenario.effort_delta == 0.0
    
    def test_scenario_analysis_single_phase(self):
        """Handle fragment with single phase."""
        fragment = MockFragment(phases=[MockPhase("phase-1", 100.0)])
        scenario = run_scenario_analysis(fragment, "Hire contractor for phase 1")
        
        # Should create modification for that phase
        assert len(scenario.modifications) >= 1
    
    def test_parse_very_long_description(self):
        """Parse scenario with very long description."""
        long_desc = "What if we " + "hire a contractor " * 50 + "for the project?"
        intent = parse_scenario_intent(long_desc)
        
        # Should still parse correctly (not error out)
        assert intent["operation"] == "resource_substitution"
    
    def test_parse_scenario_special_characters(self):
        """Parse scenario with special characters."""
        intent = parse_scenario_intent("Hire contractor @$150/day for phase #2!")
        
        # Should parse despite special characters
        assert intent["operation"] == "resource_substitution"
    
    def test_analyze_phases_with_None_assignees(self):
        """Handle phases with None assignees."""
        phase = MockPhase("phase-1", 100.0, assignees=None)
        fragment = MockFragment(phases=[phase])
        
        scenario = run_scenario_analysis(fragment, "Hire contractor")
        
        # Should handle gracefully
        assert scenario.effort_delta != 0


# ============================================================================
# Tests: Novel Scenarios (Unlimited Extensibility)
# ============================================================================

class TestNovelScenarios:
    """Test system's ability to handle novel/unseen scenarios."""
    
    def test_novel_scenario_ai_code_gen(self):
        """Handle novel scenario: AI code generation."""
        fragment = MockFragment()
        scenario = run_scenario_analysis(
            fragment,
            "Use AI to auto-generate boilerplate code"
        )
        
        # System should handle gracefully even though it's novel
        assert isinstance(scenario, ScenarioFragment)
        assert scenario.scenario_intent["operation"] == "custom_adjustment"
        assert scenario.scenario_intent["confidence"] < 0.95  # Lower confidence for novel
    
    def test_novel_scenario_outsource_qa(self):
        """Handle novel scenario: outsource QA (hybrid of resource + scope)."""
        fragment = MockFragment()
        scenario = run_scenario_analysis(
            fragment,
            "Outsource all QA to external testing firm"
        )
        
        # Should still produce valid ScenarioFragment
        assert isinstance(scenario, ScenarioFragment)
        assert scenario.modifications is not None
    
    def test_novel_scenario_skip_documentation(self):
        """Handle novel scenario: skip documentation phase."""
        fragment = MockFragment()
        scenario = run_scenario_analysis(
            fragment,
            "Skip detailed documentation to save time"
        )
        
        # Should handle gracefully and produce analysis
        assert isinstance(scenario, ScenarioFragment)


# ============================================================================
# Integration Tests
# ============================================================================

class TestIntegration:
    """Integration tests with realistic workflows."""
    
    def test_multiple_scenarios_comparison(self):
        """Analyze multiple scenarios for comparison."""
        fragment = MockFragment()
        
        scenarios = [
            run_scenario_analysis(fragment, "Hire a contractor"),
            run_scenario_analysis(fragment, "Reduce scope by 20%"),
            run_scenario_analysis(fragment, "Parallelize phases 1-2"),
        ]
        
        # All should produce valid results
        assert len(scenarios) == 3
        assert all(isinstance(s, ScenarioFragment) for s in scenarios)
        
        # Deltas should be different for each scenario
        effort_deltas = [s.effort_delta for s in scenarios]
        assert len(set(effort_deltas)) >= 2  # At least 2 different values
    
    def test_scenario_preserves_base_fragment(self):
        """Verify scenario analysis doesn't mutate base fragment."""
        fragment = MockFragment()
        original_effort = fragment.phases[0].estimated_effort
        
        scenario = run_scenario_analysis(fragment, "Hire contractor")
        
        # Base fragment should be unchanged
        assert fragment.phases[0].estimated_effort == original_effort
        
        # Scenario should have different values
        assert scenario.effort_delta != 0
    
    def test_scenario_chain_multiple_operations(self):
        """Test analyzing scenarios sequentially (chain operations)."""
        fragment = MockFragment()
        
        # First scenario: hire contractor
        scenario1 = run_scenario_analysis(fragment, "Hire contractor for phase 2")
        
        # Second scenario: reduce cost by 20 percent
        scenario2 = run_scenario_analysis(fragment, "Reduce cost to save money")
        
        # Both should produce valid results
        assert isinstance(scenario1, ScenarioFragment)
        assert isinstance(scenario2, ScenarioFragment)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
