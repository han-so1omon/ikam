from __future__ import annotations

from typing import List

from .models import AgentInfo, AgentMatch, MatchQuery, AgentStatus


class CapabilityMatcher:
    """Rule-based matcher that scores agents based on requested capabilities.

    Scoring:
      - Base: +5 if domain matches, +4 if action matches, +1 per matching tag (up to +5)
      - Weights: Apply domain_weights, action_weights, tag_weights as multipliers
      - Priority: Add agent.capabilities.priority as a bonus
      - Status penalties: -2 if DEGRADED, -5 if DRAINING/UNHEALTHY
      - Capacity: -0.5 per in_flight task when nearing max_concurrency
      - Cost: Slight boost for lower cost_hint
    """

    def score(self, agent: AgentInfo, q: MatchQuery) -> AgentMatch | None:
        if q.require_healthy and agent.status in {AgentStatus.UNHEALTHY, AgentStatus.DRAINING}:
            return None

        reasons: List[str] = []
        score = 0.0

        # Domain match with optional weight
        if q.domain and q.domain in agent.capabilities.domains:
            weight = agent.capabilities.domain_weights.get(q.domain, 1.0)
            boost = 5.0 * weight
            score += boost
            reasons.append(f"domain:{q.domain}(w={weight:.1f})")

        # Action match with optional weight
        if q.action and q.action in agent.capabilities.actions:
            weight = agent.capabilities.action_weights.get(q.action, 1.0)
            boost = 4.0 * weight
            score += boost
            reasons.append(f"action:{q.action}(w={weight:.1f})")

        # Tag matches with optional weights
        if q.tags:
            matching_tags = set(q.tags) & set(agent.capabilities.tags)
            if matching_tags:
                total_tag_boost = 0.0
                for tag in matching_tags:
                    weight = agent.capabilities.tag_weights.get(tag, 1.0)
                    total_tag_boost += weight
                total_tag_boost = min(total_tag_boost, 5.0)  # cap at +5
                score += total_tag_boost
                reasons.append(f"tags:{len(matching_tags)}(boost={total_tag_boost:.1f})")

        # Priority bonus
        if agent.capabilities.priority != 0:
            score += agent.capabilities.priority
            reasons.append(f"priority:{agent.capabilities.priority:+d}")

        # Status penalties
        if agent.status == AgentStatus.DEGRADED:
            score -= 2
            reasons.append("penalty:degraded")
        elif agent.status in {AgentStatus.DRAINING, AgentStatus.UNHEALTHY}:
            score -= 5
            reasons.append("penalty:not-available")

        # Capacity awareness
        max_c = agent.capabilities.max_concurrency
        if max_c is not None and max_c > 0:
            # Penalize as we approach capacity
            penalty = 0.5 * max(0, agent.in_flight - max(0, max_c - 2))
            if penalty:
                score -= penalty
                reasons.append(f"penalty:capacity-{penalty:.1f}")

        # Cost hint as tie-breaker (lower cost => slight boost)
        if agent.capabilities.cost_hint is not None:
            boost = max(0.0, 1.0 - min(agent.capabilities.cost_hint, 1e6) / 100.0)
            score += boost

        # If no signals matched at all, return None
        if score == 0 and not reasons:
            return None

        return AgentMatch(agent=agent, score=score, reasons=reasons)

    def rank(self, agents: List[AgentInfo], q: MatchQuery) -> List[AgentMatch]:
        results: List[AgentMatch] = []
        for a in agents:
            m = self.score(a, q)
            if m is not None:
                results.append(m)
        results.sort(key=lambda m: m.score, reverse=True)
        return results[: q.limit]
