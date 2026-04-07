export type Bounds3D = {
  minX: number;
  minY: number;
  minZ: number;
  maxX: number;
  maxY: number;
  maxZ: number;
};

export type CameraFrame = {
  x: number;
  y: number;
  z: number;
  targetX: number;
  targetY: number;
  targetZ: number;
};

export function computeBoundsForNodeIds(
  nodeIds: string[],
  nodeIndex: Map<string, number>,
  positions: Float32Array
): Bounds3D {
  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let minZ = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  let maxZ = Number.NEGATIVE_INFINITY;

  for (const nodeId of nodeIds) {
    const idx = nodeIndex.get(nodeId);
    if (idx == null) continue;
    const x = positions[idx * 3];
    const y = positions[idx * 3 + 1];
    const z = positions[idx * 3 + 2];
    if (x < minX) minX = x;
    if (y < minY) minY = y;
    if (z < minZ) minZ = z;
    if (x > maxX) maxX = x;
    if (y > maxY) maxY = y;
    if (z > maxZ) maxZ = z;
  }

  if (!Number.isFinite(minX)) {
    return { minX: 0, minY: 0, minZ: 0, maxX: 0, maxY: 0, maxZ: 0 };
  }
  return { minX, minY, minZ, maxX, maxY, maxZ };
}

export function computeCenterFromBounds(bounds: Bounds3D) {
  return {
    x: (bounds.minX + bounds.maxX) / 2,
    y: (bounds.minY + bounds.maxY) / 2,
    z: (bounds.minZ + bounds.maxZ) / 2,
  };
}

export function computeCameraDistance(bounds: Bounds3D, fovDegrees: number) {
  const sizeX = Math.max(1, bounds.maxX - bounds.minX);
  const sizeY = Math.max(1, bounds.maxY - bounds.minY);
  const sizeZ = Math.max(1, bounds.maxZ - bounds.minZ);
  const maxSpan = Math.max(sizeX, sizeY, sizeZ);
  const fovRad = (fovDegrees * Math.PI) / 180;
  const fitDistance = maxSpan / (2 * Math.tan(fovRad / 2));
  return Math.max(80, fitDistance * 1.35);
}

export function easeOutCubic(t: number) {
  const clamped = Math.min(1, Math.max(0, t));
  return 1 - Math.pow(1 - clamped, 3);
}

export function buildCameraTweenFrames(start: CameraFrame, end: CameraFrame, frameCount = 8): CameraFrame[] {
  const steps = Math.max(2, Math.floor(frameCount));
  const frames: CameraFrame[] = [];
  for (let i = 1; i <= steps; i += 1) {
    const t = easeOutCubic(i / steps);
    frames.push({
      x: start.x + (end.x - start.x) * t,
      y: start.y + (end.y - start.y) * t,
      z: start.z + (end.z - start.z) * t,
      targetX: start.targetX + (end.targetX - start.targetX) * t,
      targetY: start.targetY + (end.targetY - start.targetY) * t,
      targetZ: start.targetZ + (end.targetZ - start.targetZ) * t,
    });
  }
  return frames;
}
