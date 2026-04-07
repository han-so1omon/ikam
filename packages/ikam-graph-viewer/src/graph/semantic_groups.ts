import type { GraphNode } from '../types';
import type { SemanticEntity, SemanticRelation } from '../app/api/client';

export type GroupInfo = { id: string; label: string; nodeIds: string[] };
export type GroupBuildResult = {
  groupsById: Map<string, GroupInfo>;
  groupOrder: string[];
  nodeGroup: Map<string, string>;
};

const UNGROUPED_BUCKETS = 10;

const stableBucket = (input: string) => {
  let hash = 0;
  for (let i = 0; i < input.length; i += 1) {
    hash = (hash * 31 + input.charCodeAt(i)) >>> 0;
  }
  return hash % UNGROUPED_BUCKETS;
};

export function buildSemanticGroups(
  nodes: GraphNode[],
  relations: SemanticRelation[],
  semantic?: { entities?: SemanticEntity[]; relations?: SemanticRelation[] }
): GroupBuildResult {
  const groupsById = new Map<string, GroupInfo>();
  const nodeGroup = new Map<string, string>();
  const groupOrder: string[] = [];

  const entityLabels = new Map((semantic?.entities ?? []).map((entity) => [entity.id, entity.label ?? entity.id]));

  for (const node of nodes) {
    const meta = (node as GraphNode).meta ?? {};
    const canonicalEntityId = typeof meta.semantic_entity_id === 'string' ? meta.semantic_entity_id : undefined;
    const arrayEntityId = Array.isArray((meta as any).semantic_entity_ids)
      ? ((meta as any).semantic_entity_ids.find((value: unknown) => typeof value === 'string') as string | undefined)
      : undefined;
    const entityId = canonicalEntityId ?? arrayEntityId;
    const fallbackType = node.type || 'fragment';
    const ungroupedBucket = stableBucket(`${fallbackType}:${node.id}`);
    const groupId = entityId
      ? `semantic-entity:${entityId}`
      : `semantic-entity:ungrouped:${fallbackType}:${ungroupedBucket}`;
    if (!groupsById.has(groupId)) {
      const label = entityId
        ? entityLabels.get(entityId) ?? entityId
        : `Ungrouped ${fallbackType} ${ungroupedBucket + 1}`;
      groupsById.set(groupId, { id: groupId, label, nodeIds: [] });
      groupOrder.push(groupId);
    }
    groupsById.get(groupId)!.nodeIds.push(node.id);
    nodeGroup.set(node.id, groupId);
  }

  return { groupsById, groupOrder, nodeGroup };
}
