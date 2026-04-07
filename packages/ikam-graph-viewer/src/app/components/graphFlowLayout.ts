import dagre from 'dagre';
import { forceCenter, forceCollide, forceLink, forceManyBody, forceSimulation } from 'd3-force';
import { Position } from '@xyflow/react';

type LayoutDirection = 'TB' | 'LR';

type LayoutNode = {
  id: string;
  type?: string;
  position: { x: number; y: number };
  targetPosition?: string;
  sourcePosition?: string;
};

type LayoutEdge = {
  source: string;
  target: string;
};

type LayoutStrategy = 'layered' | 'organic';

type LayoutMetrics = {
  maxDegree: number;
  edgeCount: number;
  nodeCount: number;
  rootId: string | null;
  rootDegree: number;
};

const computeMetrics = <TNode extends LayoutNode, TEdge extends LayoutEdge>(nodes: TNode[], edges: TEdge[]): LayoutMetrics => {
  const degreeByNode = new Map<string, number>();
  for (const node of nodes) {
    degreeByNode.set(node.id, 0);
  }
  for (const edge of edges) {
    degreeByNode.set(edge.source, (degreeByNode.get(edge.source) ?? 0) + 1);
    degreeByNode.set(edge.target, (degreeByNode.get(edge.target) ?? 0) + 1);
  }
  const ranked = nodes
    .map((node) => ({ id: node.id, degree: degreeByNode.get(node.id) ?? 0 }))
    .sort((left, right) => right.degree - left.degree || left.id.localeCompare(right.id));
  return {
    maxDegree: ranked[0]?.degree ?? 0,
    edgeCount: edges.length,
    nodeCount: nodes.length,
    rootId: ranked[0]?.id ?? null,
    rootDegree: ranked[0]?.degree ?? 0,
  };
};

const chooseLayoutStrategy = (metrics: LayoutMetrics): LayoutStrategy => {
  if (metrics.nodeCount >= 10 && metrics.rootDegree >= Math.max(6, Math.floor(metrics.nodeCount * 0.45))) {
    return 'organic';
  }
  return 'layered';
};

const layeredLayout = <TNode extends LayoutNode, TEdge extends LayoutEdge>(
  nodes: TNode[],
  edges: TEdge[],
  direction: LayoutDirection,
  getNodeSize: (node: TNode) => { width: number; height: number },
  ranksep?: number,
  nodesep?: number,
) => {
  const dagreGraph = new dagre.graphlib.Graph();
  dagreGraph.setDefaultEdgeLabel(() => ({}));
  dagreGraph.setGraph({ rankdir: direction, ranksep, nodesep });

  const isHorizontal = direction === 'LR';
  nodes.forEach((node) => {
    const { width, height } = getNodeSize(node);
    dagreGraph.setNode(node.id, { width, height });
  });
  edges.forEach((edge) => {
    dagreGraph.setEdge(edge.source, edge.target);
  });
  dagre.layout(dagreGraph);

  return {
    nodes: nodes.map((node) => {
      const position = dagreGraph.node(node.id);
      const { width, height } = getNodeSize(node);
      return {
        ...node,
        position: { x: position.x - width / 2, y: position.y - height / 2 },
        targetPosition: isHorizontal ? Position.Left : Position.Top,
        sourcePosition: isHorizontal ? Position.Right : Position.Bottom,
      };
    }),
    edges,
  };
};

const organicLayout = <TNode extends LayoutNode, TEdge extends LayoutEdge>(
  nodes: TNode[],
  edges: TEdge[],
  rootId: string | null,
  getNodeSize: (node: TNode) => { width: number; height: number },
) => {
  const radius = Math.max(180, nodes.length * 12);
  const simulationNodes = nodes.map((node, index) => {
    const angle = (index / Math.max(nodes.length, 1)) * Math.PI * 2;
    return {
      id: node.id,
      x: Math.cos(angle) * radius,
      y: Math.sin(angle) * radius,
      fx: node.id === rootId ? 0 : undefined,
      fy: node.id === rootId ? 0 : undefined,
    };
  });
  const nodeById = new Map(simulationNodes.map((node) => [node.id, node]));
  const simulationLinks = edges
    .map((edge) => ({ source: nodeById.get(edge.source), target: nodeById.get(edge.target) }))
    .filter((edge): edge is { source: { id: string; x: number; y: number }; target: { id: string; x: number; y: number } } => Boolean(edge.source && edge.target));

  const simulation = forceSimulation(simulationNodes)
    .force('charge', forceManyBody().strength(-140))
    .force('center', forceCenter(0, 0))
    .force('link', forceLink(simulationLinks).distance(110).strength(0.55))
    .force('collide', forceCollide((node) => {
      const sourceNode = nodes.find((candidate) => candidate.id === node.id) ?? nodes[0]!;
      const size = getNodeSize(sourceNode);
      return Math.max(size.width, size.height) * 0.5;
    }))
    .stop();

  for (let index = 0; index < 220; index += 1) {
    simulation.tick();
  }

  return {
    nodes: nodes.map((node) => {
      const positioned = nodeById.get(node.id);
      const { width, height } = getNodeSize(node);
      return {
        ...node,
        position: {
          x: (positioned?.x ?? 0) - width / 2,
          y: (positioned?.y ?? 0) - height / 2,
        },
        targetPosition: Position.Left,
        sourcePosition: Position.Right,
      };
    }),
    edges,
  };
};

export function getLayoutedElements<TNode extends LayoutNode, TEdge extends LayoutEdge>(
  nodes: TNode[],
  edges: TEdge[],
  {
    direction = 'TB',
    getNodeSize,
    ranksep,
    nodesep,
  }: {
    direction?: LayoutDirection;
    getNodeSize: (node: TNode) => { width: number; height: number };
    ranksep?: number;
    nodesep?: number;
  },
) {
  const metrics = computeMetrics(nodes, edges);
  const strategy = chooseLayoutStrategy(metrics);
  if (strategy === 'organic') {
    return organicLayout(nodes, edges, metrics.rootId, getNodeSize);
  }
  return layeredLayout(nodes, edges, direction, getNodeSize, ranksep, nodesep);
}
