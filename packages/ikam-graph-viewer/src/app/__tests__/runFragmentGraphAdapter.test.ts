import { describe, expect, it } from 'vitest';

import { createRunFragmentGraphAdapter } from '../components/debug/runFragmentGraphAdapter';

describe('runFragmentGraphAdapter', () => {
  it('builds a focused neighborhood, expands one hop, and filters by node kind and relation', () => {
    const adapter = createRunFragmentGraphAdapter();

    adapter.ingest({
      nodes: [
        { id: 'subgraph:document-set', kind: 'subgraph', ir_kind: 'document_set', label: 'hot://run/document_set/load', payload: {} },
        { id: 'fragment:document', kind: 'fragment', ir_kind: 'document', label: 'alpha.md', payload: { value: { filename: 'alpha.md', document_id: 'doc-1' } } },
        { id: 'fragment:document-chunk-set', kind: 'fragment', ir_kind: 'json', label: 'document_chunk_set', payload: { value: { kind: 'document_chunk_set', document_id: 'doc-1' } } },
        { id: 'subgraph:chunk-set', kind: 'subgraph', ir_kind: 'chunk_extraction_set', label: 'hot://run/chunk_extraction_set/parse', payload: {} },
        { id: 'fragment:chunk', kind: 'fragment', ir_kind: 'chunk', label: 'alpha.md#chunk-1', payload: { value: { chunk_id: 'alpha.md#chunk-1', document_id: 'doc-1' } } },
        { id: 'fragment:note', kind: 'fragment', ir_kind: 'note', label: 'note-1', payload: {} },
      ],
      edges: [
        { id: 'edge:1', from: 'subgraph:document-set', to: 'fragment:document', relation: 'contains' },
        { id: 'edge:2', from: 'fragment:document', to: 'fragment:document-chunk-set', relation: 'references' },
        { id: 'edge:3', from: 'fragment:document-chunk-set', to: 'subgraph:chunk-set', relation: 'derives' },
        { id: 'edge:4', from: 'subgraph:chunk-set', to: 'fragment:chunk', relation: 'contains' },
        { id: 'edge:5', from: 'fragment:document', to: 'fragment:note', relation: 'annotates' },
      ],
      root_node_id: 'fragment:document',
    });

    const focused = adapter.getVisibleGraph({ focusNodeId: 'fragment:document' });
    expect(focused.nodes.map((node) => node.id)).toEqual([
      'fragment:document',
      'fragment:document-chunk-set',
      'fragment:note',
      'subgraph:document-set',
    ]);
    expect(focused.nodes.find((node) => node.id === 'fragment:document')).toEqual(
      expect.objectContaining({
        className: expect.stringContaining('inspection-graph-node-root'),
        data: expect.objectContaining({ isRoot: true }),
      })
    );
    expect(focused.edges.map((edge) => edge.id)).toEqual(['edge:1', 'edge:2', 'edge:5']);

    const expanded = adapter.expandVisibleGraph({
      focusNodeId: 'fragment:document',
      visibleNodeIds: focused.nodes.map((node) => node.id),
    });
    expect(expanded.nodes.map((node) => node.id)).toEqual([
      'fragment:document',
      'fragment:document-chunk-set',
      'fragment:note',
      'subgraph:chunk-set',
      'subgraph:document-set',
    ]);
    expect(expanded.edges.map((edge) => edge.id)).toEqual(['edge:1', 'edge:2', 'edge:3', 'edge:5']);

    const filtered = adapter.getVisibleGraph({
      focusNodeId: 'fragment:document',
      expandedNodeIds: expanded.nodes.map((node) => node.id),
      nodeKinds: ['chunk_extraction_set', 'document_set'],
      relations: ['contains'],
    });
    expect(filtered.nodes.map((node) => node.id)).toEqual(['subgraph:chunk-set', 'subgraph:document-set']);
    expect(filtered.edges).toEqual([]);
    expect(adapter.getNode('fragment:chunk')).toEqual(expect.objectContaining({ id: 'fragment:chunk' }));
  });
});
