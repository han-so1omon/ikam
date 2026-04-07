import { describe, expect, it } from 'vitest';

import { buildGraphStats, toInspectableEdge, toInspectableNode } from '../../explainability/adapters';
import { deriveSelectionDetails } from '../../explainability/inspector';

describe('explainability adapters', () => {
  it('maps node and edge payload to inspectable types', () => {
    const node = toInspectableNode({
      id: 'n1',
      type: 'fragment',
      label: 'Node 1',
      meta: {
        artifact_id: 'a1',
        origin: 'map',
        semantic_entity_ids: ['se1'],
      },
    });
    const edge = toInspectableEdge({
      id: 'e1',
      source: 'n1',
      target: 'n2',
      kind: 'composition',
      meta: {
        origin: 'semantic',
        semantic_relation_ids: ['sr1'],
      },
    });

    expect(node.artifactIds).toEqual(['a1']);
    expect(node.provenance.origin).toBe('map');
    expect(node.semanticLinks.entityIds).toEqual(['se1']);

    expect(edge.kind).toBe('composition');
    expect(edge.provenance.origin).toBe('semantic');
    expect(edge.semanticLinks.relationIds).toEqual(['sr1']);
  });

  it('builds graph stats by kinds', () => {
    const stats = buildGraphStats({
      nodes: [
        { id: '1', type: 'artifact' },
        { id: '2', type: 'fragment' },
        { id: '3', type: 'fragment' },
      ],
      edges: [
        { source: '1', target: '2', kind: 'composition' },
        { source: '2', target: '3', kind: 'semantic_relation' },
      ],
    });

    expect(stats.nodes).toBe(3);
    expect(stats.edges).toBe(2);
    expect(stats.nodeKinds).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ kind: 'artifact', count: 1 }),
        expect.objectContaining({ kind: 'fragment', count: 2 }),
      ])
    );
  });

  it('derives selection details for node and edge', () => {
    const details = deriveSelectionDetails(
      {
        nodes: [{ id: 'n1', type: 'fragment', meta: { artifact_id: 'a1' } }],
        edges: [{ id: 'e1', source: 'n1', target: 'n1', kind: 'composition' }],
      },
      { selectedNodeId: 'n1', selectedEdgeId: 'e1' }
    );

    expect(details.selectedNode?.id).toBe('n1');
    expect(details.selectedEdge?.id).toBe('e1');
  });
});
