import { describe, expect, it } from 'vitest';
import { getGroupDecorations } from '../../GraphView';
import { buildSemanticGroups } from '../semantic_groups';

describe('buildSemanticGroups', () => {
  it('groups nodes by semantic entity id when present', () => {
    const nodes = [
      { id: 'n1', type: 'fragment', label: 'A', meta: { semantic_entity_id: 'e1' } },
      { id: 'n2', type: 'fragment', label: 'B', meta: { semantic_entity_id: 'e1' } },
    ];
    const groups = buildSemanticGroups(nodes, [], { entities: [{ id: 'e1', label: 'Revenue' }] });
    expect(groups.groupsById.get('semantic-entity:e1')?.nodeIds).toEqual(['n1', 'n2']);
    expect(groups.groupOrder[0]).toBe('semantic-entity:e1');
  });

  it('assigns nodes without semantic entity id to ungrouped', () => {
    const nodes = [
      { id: 'n1', type: 'fragment', label: 'A', meta: { semantic_entity_id: 'e1' } },
      { id: 'n2', type: 'fragment', label: 'B' },
    ];
    const groups = buildSemanticGroups(nodes, [], { entities: [{ id: 'e1', label: 'Revenue' }] });
    const ungroupedGroupId = groups.nodeGroup.get('n2');
    expect(ungroupedGroupId).toBeDefined();
    expect(ungroupedGroupId).toMatch(/^semantic-entity:ungrouped:/);
    expect(groups.groupsById.get(ungroupedGroupId!)?.nodeIds).toEqual(['n2']);
  });

  it('uses semantic_entity_ids array when canonical semantic_entity_id is missing', () => {
    const nodes = [
      { id: 'n1', type: 'fragment', label: 'A', meta: { semantic_entity_ids: ['e9', 'e2'] } },
      { id: 'n2', type: 'fragment', label: 'B', meta: { semantic_entity_ids: ['e9'] } },
    ];
    const groups = buildSemanticGroups(nodes, [], {
      entities: [{ id: 'e9', label: 'Unit Economics' }],
    });

    expect(groups.groupsById.get('semantic-entity:e9')?.nodeIds).toEqual(['n1', 'n2']);
    expect(groups.groupsById.get('semantic-entity:e9')?.label).toBe('Unit Economics');
    expect(groups.nodeGroup.get('n1')).toBe('semantic-entity:e9');
  });

  it('orders groups by first appearance and preserves grouped entries', () => {
    const nodes = [
      { id: 'n1', type: 'fragment', label: 'A', meta: { semantic_entity_id: 'e1' } },
      { id: 'n2', type: 'fragment', label: 'B' },
      { id: 'n3', type: 'fragment', label: 'C', meta: { semantic_entity_id: 'e2' } },
      { id: 'n4', type: 'fragment', label: 'D' },
    ];
    const groups = buildSemanticGroups(nodes, [], {
      entities: [
        { id: 'e1', label: 'Revenue' },
        { id: 'e2', label: 'Cost' },
      ],
    });
    expect(groups.groupOrder[0]).toBe('semantic-entity:e1');
    expect(groups.groupOrder.some((groupId) => groupId.startsWith('semantic-entity:ungrouped:'))).toBe(true);
    expect(groups.groupOrder.some((groupId) => groupId === 'semantic-entity:e2')).toBe(true);
  });

  it('renders only the active group label when activeGroupId is set', () => {
    const groupCenters = new Map([
      [
        'g1',
        {
          x: 0,
          y: 0,
          z: 0,
          radius: 12,
          label: 'Revenue',
          count: 4,
        },
      ],
      [
        'g2',
        {
          x: 20,
          y: 10,
          z: 0,
          radius: 8,
          label: 'Costs',
          count: 2,
        },
      ],
    ]);

    expect(getGroupDecorations(groupCenters, {})).toEqual([]);
    expect(getGroupDecorations(groupCenters, { showGroups: true, activeGroupId: 'g2' }).map(([id]) => id)).toEqual([
      'g2',
    ]);
  });
});
