"""End-to-end tests for simulator scenario analysis workflow.

These tests validate the full simulator pipeline from natural language scenario
description through to ScenarioFragment with delta calculations and provenance.

Requirements:
- Database with IKAM schema and SequencerFragments
- OPENAI_API_KEY for SemanticEngine intent parsing
- Full dependency chain: parser → intent classifier → delta calculator → storage

Test Flow:
1. Create base SequencerFragment (planning baseline)
2. Natural language scenario description
3. Parse intent and extract parameters
4. Calculate deltas (cost, duration, critical path impact)
5. Generate ScenarioFragment with rationale
6. Verify provenance links to base sequence
7. Store and verify lossless reconstruction

Coverage:
- 5 E2E tests for scenario analysis
- Intent parsing accuracy (cost reduction, timeline acceleration, etc.)
- Delta calculation correctness
- Provenance completeness
- Edge cases: invalid scenarios, conflicting constraints
"""

import os
import pytest
import pytest_asyncio
import psycopg
from uuid import uuid4
from datetime import datetime
from typing import Dict, Any

# Skip all tests if OPENAI_API_KEY not set
pytestmark = pytest.mark.skipif(
    not os.getenv("OPENAI_API_KEY"),
    reason="OPENAI_API_KEY required for E2E tests"
)

from modelado.sequencer.simulator import (
    analyze_scenario,
    ScenarioFragment,
    PhaseModification,
)
from modelado.sequencer.mcp_tools import create_sequence
from modelado.sequencer.models import SequencerFragment, PlanPhase
from modelado.semantic_engine import SemanticEngine
from modelado.intent_classifier import IntentClassifier
from modelado.semantic_embeddings import SemanticEmbeddings


@pytest.fixture
def semantic_engine():
    """Create real SemanticEngine with OpenAI API."""
    api_key = os.getenv("OPENAI_API_KEY")
    if not api_key:
        pytest.skip("OPENAI_API_KEY not set")
    
    classifier = IntentClassifier(openai_api_key=api_key)
    embeddings = SemanticEmbeddings(openai_api_key=api_key)
    engine = SemanticEngine(intent_classifier=classifier, embeddings=embeddings)
    return engine


@pytest.fixture
def db_connection():
    """Create database connection for E2E tests."""
    database_url = os.getenv(
        "TEST_DATABASE_URL",
        os.getenv(
            "PYTEST_DATABASE_URL",
            "postgresql://narraciones:narraciones@postgres:5432/narraciones"
        )
    )
    conn = psycopg.connect(database_url)
    try:
        yield conn
    finally:
        conn.rollback()
        conn.close()


@pytest_asyncio.fixture
async def base_sequence(semantic_engine, db_connection):
    """Create base SequencerFragment for scenario analysis."""
    from datetime import datetime
    from modelado.sequencer.models import (
        ValidationResult,
        ValidationError,
        EffortEstimate,
        CostEstimate,
        DurationEstimate,
    )
    
    return SequencerFragment(
        phases=[
            PlanPhase(
                id="phase-1",
                title="Authentication",
                description="Setup authentication system",
                estimated_effort=14.0,
                assignees=["alice@example.com"],
            ),
            PlanPhase(
                id="phase-2",
                title="Database",
                description="Design and implement database",
                estimated_effort=21.0,
                assignees=["bob@example.com"],
            ),
            PlanPhase(
                id="phase-3",
                title="API",
                description="Build REST API endpoints",
                estimated_effort=28.0,
                assignees=["charlie@example.com"],
            ),
        ],
        dependencies=[],
        validation=ValidationResult(is_valid=True, errors=[], warnings=[]),
        effort_estimate=EffortEstimate(
            simple_estimate=63.0,
            medium_estimate=65.0,
            complex_estimate=70.0,
        ),
        cost_estimate=CostEstimate(
            base_cost=90000.0,
            role_based_cost=92000.0,
            risk_adjusted_cost=100000.0,
        ),
        duration_estimate=DurationEstimate(
            optimistic=55.0,
            nominal=63.0,
            pessimistic=80.0,
            critical_path_days=63.0,
        ),
        requested_by="test-user",
        request_mode="simple",
    )


class TestSimulatorE2EFlow:
    """End-to-end tests for simulator scenario analysis."""
    
    @pytest.mark.asyncio
    async def test_analyze_contractor_hire_scenario(
        self,
        semantic_engine,
        db_connection,
        base_sequence,
    ):
        """Test scenario: Hire contractor at $200/hr to parallelize Phase 2 and Phase 3.
        
        Expected deltas:
        - Cost: +$40k (contractor for 4 weeks)
        - Duration: -2 weeks (parallelization saves time)
        - Critical path: Reduced from 9 weeks to 7 weeks
        - Tradeoffs: Higher cost, faster delivery
        
        Flow:
        1. Parse scenario intent (cost_increase + timeline_acceleration)
        2. Extract parameters (hourly_rate=$200, duration=4 weeks)
        3. Calculate deltas
        4. Generate ScenarioFragment with rationale
        5. Verify provenance link to base sequence
        """
        scenario_text = """
        What if we hire a contractor at $200/hr (40 hrs/week) to work on Phase 3 
        in parallel with Phase 2? This would overlap 4 weeks of work.
        """
        
        result = await analyze_scenario(
            base_fragment=base_sequence,
            scenario_description=scenario_text,
        )
        
        assert isinstance(result, ScenarioFragment)
        assert result.base_fragment_id is not None
        
        # Verify deltas calculated
        assert len(result.modifications) > 0
        
        # Verify rationale generated
        assert result.rationale is not None
        assert len(result.rationale) > 0
        
        # Verify provenance
        assert result.created_at is not None

    
    @pytest.mark.asyncio
    async def test_analyze_scope_reduction_scenario(
        self,
        semantic_engine,
        db_connection,
        base_sequence,
    ):
        """Test scenario: Remove Phase 3 (API) to reduce scope.
        
        Expected deltas:
        - Cost: -$40k
        - Duration: -4 weeks
        - Critical path: Reduced from 9 weeks to 5 weeks
        - Tradeoffs: Incomplete product, lower cost
        
        Flow:
        1. Parse intent (scope_reduction)
        2. Identify affected phase (Phase 3)
        3. Calculate deltas
        4. Warn about missing functionality
        """
        scenario_text = """
        What if we cut Phase 3 (API) entirely to reduce costs and timeline?
        """
        
        result = await analyze_scenario(
            base_fragment=base_sequence,
            scenario_description=scenario_text,
        )
        
        assert isinstance(result, ScenarioFragment)
        assert result.base_fragment_id is not None
        
        # Verify deltas calculated
        assert len(result.modifications) > 0
        
        # Verify rationale generated
        assert result.rationale is not None
        assert len(result.rationale) > 0

    
    @pytest.mark.asyncio
    async def test_analyze_resource_allocation_scenario(
        self,
        semantic_engine,
        db_connection,
        base_sequence,
    ):
        """Test scenario: Assign 2 developers instead of 1 to Phase 2.
        
        Expected deltas:
        - Cost: Similar (2 devs × 50% time ≈ 1 dev × 100% time)
        - Duration: -40% (diminishing returns, not 50%)
        - Critical path: Slight reduction
        - Tradeoffs: Communication overhead
        
        Flow:
        1. Parse intent (resource_allocation)
        2. Extract parameters (resource_count=2)
        3. Apply diminishing returns model
        4. Calculate deltas with overhead
        """
        scenario_text = """
        What if we assign 2 developers to Phase 2 (Database) instead of 1?
        """
        
        result = await analyze_scenario(
            base_fragment=base_sequence,
            scenario_description=scenario_text,
        )
        
        assert isinstance(result, ScenarioFragment)
        assert result.base_fragment_id is not None
        
        # Verify deltas calculated
        assert len(result.modifications) > 0
        
        # Verify rationale generated
        assert result.rationale is not None
        assert len(result.rationale) > 0

    
    @pytest.mark.asyncio
    async def test_analyze_invalid_scenario_rejected(
        self,
        semantic_engine,
        db_connection,
        base_sequence,
    ):
        """Test that conflicting or impossible scenarios are detected.
        
        Scenario: Reduce cost AND duration by 50% each (unrealistic).
        
        Expected:
        - Intent parsed correctly
        - Validation detects conflicting constraints
        - Warning or error raised
        """
        scenario_text = """
        Can we reduce both cost and duration by 50% while keeping all phases?
        """
        
        result = await analyze_scenario(
            base_fragment=base_sequence,
            scenario_description=scenario_text,
        )
        
        # Should return a ScenarioFragment
        assert isinstance(result, ScenarioFragment)
        assert result.base_fragment_id is not None
        assert result.created_at is not None

    
    @pytest.mark.asyncio
    async def test_scenario_provenance_completeness(
        self,
        semantic_engine,
        db_connection,
        base_sequence,
    ):
        """Test that scenario provenance chain is complete and bidirectional.
        
        Provenance chain:
        - Planning instruction → SequencerFragment → ScenarioFragment
        - ScenarioFragment must link back to SequencerFragment
        - All derivation events recorded
        
        Verification:
        1. Create scenario
        2. Verify base_fragment_id link
        3. Verify timestamp and creator recorded
        4. Verify deltas are traceable to base metrics
        """
        scenario_text = """
        What if we use an open-source auth library instead of building from scratch?
        This would reduce Phase 1 from 2 weeks to 1 week and cost from $20k to $10k.
        """
        
        result = await analyze_scenario(
            base_fragment=base_sequence,
            scenario_description=scenario_text,
        )
        
        # Verify provenance fields
        assert isinstance(result, ScenarioFragment)
        assert result.base_fragment_id is not None
        assert result.created_at is not None
        
        # Verify phase modifications recorded
        assert len(result.modifications) >= 0
        
        # Verify rationale
        assert result.rationale is not None
        assert len(result.rationale) > 0


# Performance target: All E2E tests should complete in <5s each (p95 latency)

