import type { GraphData, GraphEdge, GraphNode } from '../types';
import type { InspectableEdge, InspectableNode } from './types';

const toStringList = (value: unknown): string[] => {
  if (Array.isArray(value)) {
    return value.map((item) => String(item)).filter(Boolean);
  }
  if (typeof value === 'string' && value.trim()) {
    return [value.trim()];
  }
  return [];
};

export const toInspectableNode = (node: GraphNode): InspectableNode => {
  const meta = (node.meta ?? {}) as Record<string, unknown>;
  const artifactIds = toStringList(meta.artifact_ids ?? meta.artifact_id);
  const fragmentIds = toStringList(meta.fragment_ids ?? meta.fragment_id ?? node.id);
  return {
    id: node.id,
    type: node.type,
    label: node.label ?? node.id,
    level: node.level,
    salience: node.salience,
    artifactIds,
    fragmentIds,
    provenance: {
      origin: (meta.origin as any) ?? 'unknown',
      runId: typeof meta.run_id === 'string' ? meta.run_id : undefined,
      decisionRef: typeof meta.decision_ref === 'string' ? meta.decision_ref : undefined,
      caseId: typeof meta.case_id === 'string' ? meta.case_id : undefined,
    },
    semanticLinks: {
      entityIds: toStringList(meta.semantic_entity_ids),
      relationIds: toStringList(meta.semantic_relation_ids),
    },
    meta,
  };
};

export const toInspectableEdge = (edge: GraphEdge): InspectableEdge => {
  const meta = ((edge as unknown as { meta?: Record<string, unknown> }).meta ?? {}) as Record<string, unknown>;
  return {
    id: edge.id,
    source: edge.source,
    target: edge.target,
    kind: edge.kind ?? (typeof meta.kind === 'string' ? meta.kind : 'unknown'),
    provenance: {
      origin: (meta.origin as any) ?? 'unknown',
      runId: typeof meta.run_id === 'string' ? meta.run_id : undefined,
      decisionRef: typeof meta.decision_ref === 'string' ? meta.decision_ref : undefined,
      caseId: typeof meta.case_id === 'string' ? meta.case_id : undefined,
    },
    semanticLinks: {
      entityIds: toStringList(meta.semantic_entity_ids),
      relationIds: toStringList(meta.semantic_relation_ids),
    },
    meta,
  };
};

export const buildGraphStats = (graph: GraphData) => {
  const nodeKinds = new Map<string, number>();
  const edgeKinds = new Map<string, number>();
  for (const node of graph.nodes) {
    nodeKinds.set(node.type, (nodeKinds.get(node.type) ?? 0) + 1);
  }
  for (const edge of graph.edges) {
    const kind = edge.kind ?? 'unknown';
    edgeKinds.set(kind, (edgeKinds.get(kind) ?? 0) + 1);
  }
  return {
    nodes: graph.nodes.length,
    edges: graph.edges.length,
    nodeKinds: Array.from(nodeKinds.entries()).map(([kind, count]) => ({ kind, count })),
    edgeKinds: Array.from(edgeKinds.entries()).map(([kind, count]) => ({ kind, count })),
  };
};
