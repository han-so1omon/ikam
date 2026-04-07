import { describe, expect, it } from 'vitest';
import {
  buildCameraTweenFrames,
  computeCameraDistance,
  computeCenterFromBounds,
  computeBoundsForNodeIds,
  easeOutCubic,
} from '../camera_fit';

describe('camera_fit', () => {
  it('computes deterministic bounds for node subsets', () => {
    const positions = new Float32Array([
      0, 0, 0, // n1
      100, 0, 0, // n2
      0, 80, 0, // n3
      300, 200, 0, // n4
    ]);
    const nodeIndex = new Map([
      ['n1', 0],
      ['n2', 1],
      ['n3', 2],
      ['n4', 3],
    ]);

    const bounds = computeBoundsForNodeIds(['n1', 'n2', 'n3'], nodeIndex, positions);
    expect(bounds).toEqual({ minX: 0, minY: 0, minZ: 0, maxX: 100, maxY: 80, maxZ: 0 });

    const center = computeCenterFromBounds(bounds);
    expect(center).toEqual({ x: 50, y: 40, z: 0 });
  });

  it('returns larger camera distance for larger bounds span', () => {
    const near = computeCameraDistance({ minX: 0, minY: 0, minZ: 0, maxX: 120, maxY: 80, maxZ: 0 }, 60);
    const far = computeCameraDistance({ minX: -300, minY: -200, minZ: 0, maxX: 300, maxY: 200, maxZ: 0 }, 60);
    expect(far).toBeGreaterThan(near);
    expect(near).toBeGreaterThan(0);
  });

  it('builds deterministic ease-out camera frames', () => {
    const start = { x: 0, y: 0, z: 320, targetX: 0, targetY: 0, targetZ: 0 };
    const end = { x: 60, y: 30, z: 180, targetX: 60, targetY: 30, targetZ: 0 };

    const first = buildCameraTweenFrames(start, end, 6);
    const second = buildCameraTweenFrames(start, end, 6);

    expect(first).toEqual(second);
    expect(first.length).toBe(6);
    expect(first[0].x).toBeGreaterThan(start.x);
    expect(first[first.length - 1]).toEqual(end);
    expect(easeOutCubic(0)).toBe(0);
    expect(easeOutCubic(1)).toBe(1);
  });

  it('keeps final camera frame aligned with bounds center and distance', () => {
    const bounds = { minX: -100, minY: -50, minZ: -10, maxX: 120, maxY: 90, maxZ: 10 };
    const center = computeCenterFromBounds(bounds);
    const distance = computeCameraDistance(bounds, 60);

    const frames = buildCameraTweenFrames(
      { x: 0, y: 0, z: 400, targetX: 0, targetY: 0, targetZ: 0 },
      {
        x: center.x,
        y: center.y,
        z: center.z + distance,
        targetX: center.x,
        targetY: center.y,
        targetZ: center.z,
      },
      8
    );

    const last = frames[frames.length - 1];
    expect(last.x).toBe(center.x);
    expect(last.y).toBe(center.y);
    expect(last.targetX).toBe(center.x);
    expect(last.targetY).toBe(center.y);
    expect(last.z).toBe(center.z + distance);
  });
});
