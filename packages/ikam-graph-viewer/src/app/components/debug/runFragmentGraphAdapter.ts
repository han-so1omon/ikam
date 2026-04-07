import type { InspectionSubgraphResponse } from '../../api/client';
import { adaptInspectionSubgraph, type InspectionGraphEdge, type InspectionGraphNode } from './inspectionGraphAdapter';

type CachedNode = Record<string, unknown>;
type CachedEdge = Record<string, unknown>;

const asRecordArray = (value: unknown): Record<string, unknown>[] =>
  Array.isArray(value) ? value.filter((item): item is Record<string, unknown> => Boolean(item) && typeof item === 'object') : [];

const asString = (value: unknown): string | null =>
  typeof value === 'string' && value.trim().length > 0 ? value : null;

const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === 'object' ? (value as Record<string, unknown>) : {};

const subgraphRefFromNode = (node: CachedNode): string | null => {
  const refs = asRecord(node.refs);
  const selfRef = asRecord(refs.self);
  const locator = asRecord(selfRef.locator);
  return asString(locator.subgraph_ref) ?? asString(node.label);
};

const normalizeNodeKind = (node: CachedNode): string => {
  const payload = asRecord(node.payload);
  const value = asRecord(payload.value);
  const directKind = asString(node.ir_kind) ?? asString(value.kind) ?? asString(payload.kind);
  if (directKind) return directKind;

  const subgraphRef = subgraphRefFromNode(node);
  if (subgraphRef) {
    if (subgraphRef.includes('/document_set/')) return 'document_set';
    if (subgraphRef.includes('/chunk_extraction_set/')) return 'chunk_extraction_set';
    if (subgraphRef.includes('/document_chunk_set/')) return 'document_chunk_set';
  }

  return asString(node.kind) ?? 'node';
};

const edgeKey = (edge: CachedEdge): string => {
  return asString(edge.id) ?? `${asString(edge.from) ?? 'source'}:${asString(edge.to) ?? 'target'}:${asString(edge.relation) ?? 'edge'}`;
};

const sortStrings = (values: Iterable<string>): string[] => Array.from(new Set(values)).sort((left, right) => left.localeCompare(right));

export function createRunFragmentGraphAdapter() {
  const nodeCache = new Map<string, CachedNode>();
  const edgeCache = new Map<string, CachedEdge>();

  const neighborsOf = (nodeIds: Iterable<string>): string[] => {
    const nodeIdSet = new Set(nodeIds);
    const visible = new Set<string>();
    for (const edge of edgeCache.values()) {
      const from = asString(edge.from);
      const to = asString(edge.to);
      if (!from || !to) continue;
      if (nodeIdSet.has(from)) visible.add(to);
      if (nodeIdSet.has(to)) visible.add(from);
    }
    return sortStrings(visible);
  };

  const graphFromNodeIds = (
    nodeIds: Iterable<string>,
    focusNodeId: string,
    relations?: string[]
  ): { nodes: InspectionGraphNode[]; edges: InspectionGraphEdge[] } => {
    const visibleNodeIds = new Set(sortStrings(nodeIds).filter((nodeId) => nodeCache.has(nodeId)));
    const allowedRelations = relations && relations.length > 0 ? new Set(relations) : null;
    const response: InspectionSubgraphResponse = {
      root_node_id: visibleNodeIds.has(focusNodeId) ? focusNodeId : undefined,
      nodes: Array.from(visibleNodeIds).map((nodeId) => nodeCache.get(nodeId) ?? { id: nodeId }),
      edges: Array.from(edgeCache.values()).filter((edge) => {
        const from = asString(edge.from);
        const to = asString(edge.to);
        const relation = asString(edge.relation);
        if (!from || !to || !visibleNodeIds.has(from) || !visibleNodeIds.has(to)) return false;
        if (allowedRelations && !allowedRelations.has(relation ?? '')) return false;
        return true;
      }),
    };
    return adaptInspectionSubgraph(response);
  };

  return {
    ingest(response: InspectionSubgraphResponse) {
      for (const node of asRecordArray(response.nodes)) {
        const id = asString(node.id);
        if (id) nodeCache.set(id, node);
      }
      for (const edge of asRecordArray(response.edges)) {
        const from = asString(edge.from);
        const to = asString(edge.to);
        if (!from || !to) continue;
        edgeCache.set(edgeKey(edge), edge);
      }
    },

    getNode(nodeId: string): CachedNode | null {
      return nodeCache.get(nodeId) ?? null;
    },

    getVisibleGraph(args: {
      focusNodeId: string;
      expandedNodeIds?: string[];
      nodeKinds?: string[];
      relations?: string[];
    }): { nodes: InspectionGraphNode[]; edges: InspectionGraphEdge[] } {
      const seedNodeIds = args.expandedNodeIds && args.expandedNodeIds.length > 0
        ? sortStrings([...args.expandedNodeIds, args.focusNodeId])
        : sortStrings([args.focusNodeId, ...neighborsOf([args.focusNodeId])]);

      const allowedNodeKinds = args.nodeKinds && args.nodeKinds.length > 0 ? new Set(args.nodeKinds) : null;
      const filteredNodeIds = allowedNodeKinds
        ? seedNodeIds.filter((nodeId) => {
            const node = nodeCache.get(nodeId);
            return node ? allowedNodeKinds.has(normalizeNodeKind(node)) : false;
          })
        : seedNodeIds;

      return graphFromNodeIds(filteredNodeIds, args.focusNodeId, args.relations);
    },

    expandVisibleGraph(args: {
      focusNodeId: string;
      visibleNodeIds: string[];
      nodeKinds?: string[];
      relations?: string[];
      searchQuery?: string;
      maxNewNeighbors?: number;
    }): { nodes: InspectionGraphNode[]; edges: InspectionGraphEdge[]; expansionLimited: boolean } {
      const currentNodeIds = sortStrings([...args.visibleNodeIds, args.focusNodeId]);
      const currentNodeIdSet = new Set(currentNodeIds);
      const allowedNodeKinds = args.nodeKinds && args.nodeKinds.length > 0 ? new Set(args.nodeKinds) : null;
      const allowedRelations = args.relations && args.relations.length > 0 ? new Set(args.relations) : null;
      const query = args.searchQuery?.trim().toLowerCase() ?? '';
      const neighboringNodeIds = neighborsOf(args.visibleNodeIds).filter((nodeId) => {
        if (currentNodeIdSet.has(nodeId)) return false;
        const node = nodeCache.get(nodeId);
        if (!node) return false;
        if (allowedNodeKinds && !allowedNodeKinds.has(normalizeNodeKind(node))) return false;
        if (query) {
          const text = `${asString(node.label) ?? ''} ${asString(node.kind) ?? ''} ${asString(node.ir_kind) ?? ''}`.toLowerCase();
          if (!text.includes(query)) return false;
        }
        if (allowedRelations) {
          const hasAllowedRelation = Array.from(edgeCache.values()).some((edge) => {
            const from = asString(edge.from);
            const to = asString(edge.to);
            const relation = asString(edge.relation) ?? '';
            if (!allowedRelations.has(relation)) return false;
            return (from && to) && ((currentNodeIdSet.has(from) && to === nodeId) || (currentNodeIdSet.has(to) && from === nodeId));
          });
          if (!hasAllowedRelation) return false;
        }
        return true;
      });

      if (typeof args.maxNewNeighbors === 'number' && neighboringNodeIds.length > args.maxNewNeighbors) {
        return {
          ...this.getVisibleGraph({
            focusNodeId: args.focusNodeId,
            expandedNodeIds: currentNodeIds,
            nodeKinds: args.nodeKinds,
            relations: args.relations,
          }),
          expansionLimited: true,
        };
      }

      const expandedNodeIds = sortStrings([...currentNodeIds, ...neighboringNodeIds]);

      return {
        ...this.getVisibleGraph({
          focusNodeId: args.focusNodeId,
          expandedNodeIds,
          nodeKinds: args.nodeKinds,
          relations: args.relations,
        }),
        expansionLimited: false,
      };
    },
  };
}
