export function layoutNeighborhoods(args: {
  groupOrder: string[];
  groupNodes: Map<string, string[]>;
}) {
  const groupCenters = new Map<string, { x: number; y: number; z: number; radius: number }>();
  const nodePositions = new Map<string, { x: number; y: number; z: number }>();

  const groupCount = Math.max(args.groupOrder.length, 1);
  const columns = Math.max(1, Math.ceil(Math.sqrt(groupCount)));

  args.groupOrder.forEach((groupId, idx) => {
    const col = idx % columns;
    const row = Math.floor(idx / columns);
    const singleGroup = groupCount === 1;
    const rowOffset = (row % 2 === 0 ? 1 : -1) * 24;
    const x = singleGroup ? 0 : (col - (columns - 1) / 2) * 260 + rowOffset;
    const y = singleGroup ? 0 : (row - (Math.ceil(groupCount / columns) - 1) / 2) * 220 + (col % 2 === 0 ? -18 : 18);
    const nodes = args.groupNodes.get(groupId) ?? [];
    const localRadius = Math.max(singleGroup ? 56 : 32, Math.min(singleGroup ? 110 : 82, 26 + nodes.length * 5));
    const center = {
      x,
      y,
      z: 0,
      radius: Math.max(58, localRadius + 18),
    };
    groupCenters.set(groupId, center);

    const count = Math.max(nodes.length, 1);
    const goldenAngle = Math.PI * (3 - Math.sqrt(5));
    nodes.forEach((nodeId, nodeIdx) => {
      const localAngle = nodeIdx * goldenAngle;
      const normalizedRadius = Math.sqrt((nodeIdx + 0.5) / count);
      const spiral = 0.14 + normalizedRadius * 0.86;
      nodePositions.set(nodeId, {
        x: center.x + Math.cos(localAngle) * localRadius * spiral,
        y: center.y + Math.sin(localAngle) * localRadius * spiral,
        z: 0,
      });
    });
  });

  return { groupCenters, nodePositions };
}
