import { describe, expect, it } from 'vitest';
import { layoutNeighborhoods } from '../neighborhood_layout';

describe('layoutNeighborhoods', () => {
  it('separates neighborhoods while keeping local clusters compact', () => {
    const result = layoutNeighborhoods({
      groupOrder: ['g1', 'g2', 'g3'],
      groupNodes: new Map([
        ['g1', ['n1', 'n2', 'n3']],
        ['g2', ['n4', 'n5', 'n6']],
        ['g3', ['n7', 'n8']],
      ]),
    });

    const g1 = result.groupCenters.get('g1');
    const g2 = result.groupCenters.get('g2');
    const g3 = result.groupCenters.get('g3');
    const n1 = result.nodePositions.get('n1');
    const n2 = result.nodePositions.get('n2');
    const n4 = result.nodePositions.get('n4');
    const n7 = result.nodePositions.get('n7');

    expect(g1).toBeDefined();
    expect(g2).toBeDefined();
    expect(n1).toBeDefined();
    expect(n2).toBeDefined();
    expect(g3).toBeDefined();
    expect(n4).toBeDefined();
    expect(n7).toBeDefined();

    if (!g1 || !g2 || !g3 || !n1 || !n2 || !n4 || !n7) return;

    // Not all groups should lie on the exact same ring radius.
    const roundedRadii = [g1, g2, g3].map((group) => Math.round(Math.hypot(group.x, group.y)));
    expect(new Set(roundedRadii).size).toBeGreaterThan(1);

    // Groups should be separated from each other.
    const g1g2 = Math.hypot(g1.x - g2.x, g1.y - g2.y);
    const g2g3 = Math.hypot(g2.x - g3.x, g2.y - g3.y);
    expect(g1g2).toBeGreaterThan(120);
    expect(g2g3).toBeGreaterThan(120);

    // Node compactness around local group center.
    const g1n1 = Math.hypot(n1.x - g1.x, n1.y - g1.y);
    const g1n2 = Math.hypot(n2.x - g1.x, n2.y - g1.y);
    const g2n4 = Math.hypot(n4.x - g2.x, n4.y - g2.y);
    const g3n7 = Math.hypot(n7.x - g3.x, n7.y - g3.y);
    expect(g1n1).toBeLessThan(85);
    expect(g1n2).toBeLessThan(85);
    expect(g2n4).toBeLessThan(85);
    expect(g3n7).toBeLessThan(85);

    // Within a group, not all nodes should collapse to one point.
    const n1n2 = Math.hypot(n1.x - n2.x, n1.y - n2.y);
    expect(n1n2).toBeGreaterThan(12);
  });

  it('returns empty maps when group list is empty', () => {
    const result = layoutNeighborhoods({
      groupOrder: [],
      groupNodes: new Map(),
    });

    expect(result.groupCenters.size).toBe(0);
    expect(result.nodePositions.size).toBe(0);
  });

  it('returns an empty node map when a group has no nodes', () => {
    const result = layoutNeighborhoods({
      groupOrder: ['g1'],
      groupNodes: new Map([['g1', []]]),
    });

    expect(result.groupCenters.size).toBe(1);
    expect(result.nodePositions.size).toBe(0);
  });

  it('centers a single neighborhood near origin', () => {
    const result = layoutNeighborhoods({
      groupOrder: ['g1'],
      groupNodes: new Map([['g1', ['n1', 'n2', 'n3']]]),
    });

    const g1 = result.groupCenters.get('g1');
    expect(g1).toBeDefined();
    if (!g1) return;
    expect(g1.x).toBeCloseTo(0, 6);
    expect(g1.y).toBeCloseTo(0, 6);
  });

  it('distributes dense neighborhoods across interior, not only outer ring', () => {
    const nodeIds = Array.from({ length: 40 }, (_, index) => `n${index}`);
    const result = layoutNeighborhoods({
      groupOrder: ['g1'],
      groupNodes: new Map([['g1', nodeIds]]),
    });

    const center = result.groupCenters.get('g1');
    expect(center).toBeDefined();
    if (!center) return;

    const radii = nodeIds
      .map((nodeId) => result.nodePositions.get(nodeId))
      .filter((point): point is { x: number; y: number; z: number } => Boolean(point))
      .map((point) => Math.hypot(point.x - center.x, point.y - center.y));

    const uniqueRounded = new Set(radii.map((radius) => Math.round(radius)));
    const interiorPoints = radii.filter((radius) => radius < center.radius * 0.55).length;

    expect(uniqueRounded.size).toBeGreaterThan(10);
    expect(interiorPoints).toBeGreaterThan(8);
  });
});
