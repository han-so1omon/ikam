import { describe, expect, it } from 'vitest';

import {
  computeEdgePulseIntensity,
  getNodeDimmedScalar,
  getSoftGlassGroupBubbleStyle,
  resolveHitPriority,
  shouldEnableNodeHalo,
} from '../../GraphView';

describe('GraphView soft glass contract', () => {
  it('enables node halo for highlighted or selected node states', () => {
    expect(shouldEnableNodeHalo({ highlighted: true })).toBe(true);
    expect(shouldEnableNodeHalo({ selected: true })).toBe(true);
    expect(shouldEnableNodeHalo({ pulse: true })).toBe(true);
    expect(shouldEnableNodeHalo({ dimmed: true })).toBe(false);
  });

  it('uses a light translucent group bubble style', () => {
    const bubble = getSoftGlassGroupBubbleStyle();
    expect(bubble.color).toMatch(/^#[0-9a-fA-F]{6}$/);
    expect(bubble.opacity).toBeGreaterThan(0.15);
    expect(bubble.opacity).toBeLessThan(0.35);
  });

  it('keeps dimmed nodes above minimum visibility threshold', () => {
    expect(getNodeDimmedScalar()).toBeGreaterThanOrEqual(0.35);
  });

  it('resolves hit priority: node wins over edge when both under cursor', () => {
    // Both hit → node wins
    expect(resolveHitPriority(true, true)).toBe('node');
    // Only node hit
    expect(resolveHitPriority(true, false)).toBe('node');
    // Only edge hit
    expect(resolveHitPriority(false, true)).toBe('edge');
    // Neither hit
    expect(resolveHitPriority(false, false)).toBeNull();
  });

  it('computes edge pulse intensity as a subtle oscillation', () => {
    // Should return values in [0, 1] range
    const v0 = computeEdgePulseIntensity(0);
    const v1 = computeEdgePulseIntensity(500);
    const v2 = computeEdgePulseIntensity(1000);

    // All values in valid range
    expect(v0).toBeGreaterThanOrEqual(0);
    expect(v0).toBeLessThanOrEqual(1);
    expect(v1).toBeGreaterThanOrEqual(0);
    expect(v1).toBeLessThanOrEqual(1);
    expect(v2).toBeGreaterThanOrEqual(0);
    expect(v2).toBeLessThanOrEqual(1);

    // Should oscillate: value at 500ms differs from value at 0ms
    expect(v1).not.toBeCloseTo(v0, 1);
  });
});
