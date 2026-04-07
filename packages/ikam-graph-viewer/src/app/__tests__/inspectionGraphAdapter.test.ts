import { describe, expect, it } from 'vitest';

import type { InspectionSubgraphResponse } from '../api/client';
import { adaptInspectionSubgraph } from '../components/debug/inspectionGraphAdapter';

describe('inspectionGraphAdapter', () => {
  it('maps an inspection subgraph to stable XYFlow-like nodes and edges with readable labels', () => {
    const inspection: InspectionSubgraphResponse = {
      schema_version: 'v1',
      root_node_id: 'subgraph:document-set',
      nodes: [
        {
          id: 'subgraph:document-set',
          kind: 'subgraph',
          ir_kind: 'document_set',
          label: 'hot://run/document_set/load',
          payload: {},
        },
        {
          id: 'subgraph:chunk-set',
          kind: 'subgraph',
          ir_kind: 'chunk_extraction_set',
          label: 'hot://run/chunk_extraction_set/parse',
          payload: {},
        },
        {
          id: 'fragment:document-chunk-set',
          kind: 'fragment',
          ir_kind: 'json',
          label: 'hot://fragment/document_chunk_set',
          payload: {
            value: {
              kind: 'document_chunk_set',
              document_id: 'doc-1',
            },
          },
        },
        {
          id: 'fragment:document',
          kind: 'fragment',
          ir_kind: 'document',
          label: 'hot://fragment/document',
          payload: {
            value: {
              document_id: 'doc-1',
              filename: 'alpha.md',
            },
          },
        },
        {
          id: 'fragment:chunk',
          kind: 'fragment',
          ir_kind: 'chunk',
          label: 'hot://fragment/chunk',
          payload: {
            value: {
              chunk_id: 'alpha.md#chunk-1',
              document_id: 'doc-1',
            },
          },
        },
      ],
      edges: [
        { id: 'edge:1', from: 'subgraph:document-set', to: 'fragment:document', relation: 'contains' },
        { id: 'edge:2', from: 'fragment:document', to: 'fragment:document-chunk-set', relation: 'references' },
        { id: 'edge:3', from: 'fragment:document-chunk-set', to: 'subgraph:chunk-set', relation: 'derives' },
        { id: 'edge:4', from: 'subgraph:chunk-set', to: 'fragment:chunk', relation: 'contains' },
      ],
    };

    const first = adaptInspectionSubgraph(inspection);
    const second = adaptInspectionSubgraph(inspection);

    expect(first).toEqual(second);
    expect(first.nodes.map((node) => node.id)).toEqual([
      'fragment:chunk',
      'fragment:document',
      'fragment:document-chunk-set',
      'subgraph:chunk-set',
      'subgraph:document-set',
    ]);
    expect(first.edges).toEqual([
      expect.objectContaining({ id: 'edge:1', source: 'subgraph:document-set', target: 'fragment:document', data: { relation: 'contains' } }),
      expect.objectContaining({ id: 'edge:2', source: 'fragment:document', target: 'fragment:document-chunk-set', data: { relation: 'references' } }),
      expect.objectContaining({ id: 'edge:3', source: 'fragment:document-chunk-set', target: 'subgraph:chunk-set', data: { relation: 'derives' } }),
      expect.objectContaining({ id: 'edge:4', source: 'subgraph:chunk-set', target: 'fragment:chunk', data: { relation: 'contains' } }),
    ]);

    const labelById = Object.fromEntries(first.nodes.map((node) => [node.id, node.data.label]));
    expect(labelById).toEqual({
      'fragment:chunk': 'chunk-1',
      'fragment:document': 'alpha.md',
      'fragment:document-chunk-set': 'Document chunk set',
      'subgraph:chunk-set': 'Chunk extraction set',
      'subgraph:document-set': 'Document set',
    });

    const rootNode = first.nodes.find((node) => node.id === 'subgraph:document-set');
    expect(rootNode).toEqual(
      expect.objectContaining({
        className: expect.stringContaining('inspection-graph-node-root'),
        data: expect.objectContaining({ isRoot: true, label: 'Document set' }),
        position: expect.objectContaining({ x: expect.any(Number), y: expect.any(Number) }),
      })
    );
  });

  it('falls back to title-cased kind labels for generic nodes without explicit labels', () => {
    const graph = adaptInspectionSubgraph({
      schema_version: 'v1',
      root_node_id: 'node:custom',
      nodes: [{ id: 'node:custom', kind: 'custom_kind', payload: {} }],
      edges: [],
    });

    expect(graph.nodes).toHaveLength(1);
    expect(graph.nodes[0]?.data.label).toBe('Custom Kind');
  });

  it('maps fragment nodes to icon-first captions with hover detail metadata', () => {
    const graph = adaptInspectionSubgraph({
      schema_version: 'v1',
      root_node_id: 'fragment:chunk',
      nodes: [
        {
          id: 'fragment:document',
          kind: 'fragment',
          ir_kind: 'document',
          label: 'very-long-source-bookkeeping-q4-2025-summary.md',
          payload: {
            value: {
              document_id: 'doc-1',
              filename: 'very-long-source-bookkeeping-q4-2025-summary.md',
            },
          },
        },
        {
          id: 'fragment:chunk',
          kind: 'fragment',
          ir_kind: 'chunk',
          label: 'very-long-source-bookkeeping-q4-2025-summary.md#chunk-17',
          payload: {
            value: {
              chunk_id: 'very-long-source-bookkeeping-q4-2025-summary.md#chunk-17',
              document_id: 'doc-1',
            },
          },
        },
      ],
      edges: [{ id: 'edge:1', from: 'fragment:document', to: 'fragment:chunk', relation: 'contains' }],
    });

    const labelById = Object.fromEntries(graph.nodes.map((node) => [node.id, node.data.label]));
    const iconById = Object.fromEntries(graph.nodes.map((node) => [node.id, node.data.icon]));
    const secondaryById = Object.fromEntries(graph.nodes.map((node) => [node.id, node.data.secondaryLabel]));

    expect(labelById).toEqual({
      'fragment:document': 'very-lo...',
      'fragment:chunk': 'chunk-17',
    });
    expect(iconById).toEqual({
      'fragment:document': 'MD',
      'fragment:chunk': 'CHK',
    });
    expect(secondaryById).toEqual({
      'fragment:document': 'very-long-source-bookkeeping-q4-2025-summary.md',
      'fragment:chunk': 'very-long-source-bookkeeping-q4-2025-summary.md#chunk-17',
    });
  });

  it('derives readable shorthand badges for set and mime-based document nodes', () => {
    const graph = adaptInspectionSubgraph({
      schema_version: 'v1',
      root_node_id: 'subgraph:document-set',
      nodes: [
        {
          id: 'subgraph:document-set',
          kind: 'subgraph',
          ir_kind: 'document_set',
          label: 'hot://run/document_set/step-load',
          payload: {},
          refs: { self: { locator: { subgraph_ref: 'hot://run/document_set/step-load' } } },
        },
        {
          id: 'subgraph:chunk-set',
          kind: 'subgraph',
          ir_kind: 'chunk_extraction_set',
          label: 'hot://run/chunk_extraction_set/step-parse',
          payload: {},
          refs: { self: { locator: { subgraph_ref: 'hot://run/chunk_extraction_set/step-parse' } } },
        },
        {
          id: 'subgraph:document-chunk-set',
          kind: 'subgraph',
          ir_kind: 'document_chunk_set',
          label: 'hot://run/document_chunk_set/step-join',
          payload: {},
          refs: { self: { locator: { subgraph_ref: 'hot://run/document_chunk_set/step-join' } } },
        },
        {
          id: 'fragment:image',
          kind: 'fragment',
          ir_kind: 'document',
          label: 'asset-without-extension',
          payload: {
            mime_type: 'image/png',
            value: {
              document_id: 'img-1',
            },
          },
        },
      ],
      edges: [],
    });

    const iconById = Object.fromEntries(graph.nodes.map((node) => [node.id, node.data.icon]));

    expect(iconById).toEqual({
      'subgraph:document-set': 'DOCS',
      'subgraph:chunk-set': 'CHKS',
      'subgraph:document-chunk-set': 'DCHS',
      'fragment:image': 'PNG',
    });
  });

  it('chooses organic surrounding layout for irregular dense hub graphs', () => {
    const denseDocuments = Array.from({ length: 18 }, (_, index) => ({
      id: `fragment:document-${index + 1}`,
      kind: 'fragment',
      ir_kind: 'document',
      label: `dense-document-${index + 1}.md`,
      payload: {
        value: {
          document_id: `doc-${index + 1}`,
          filename: `dense-document-${index + 1}.md`,
        },
      },
      refs: { self: { backend: 'hot', locator: { cas_id: `frag-${index + 1}` } } },
    }));

    const graph = adaptInspectionSubgraph({
      schema_version: 'v1',
      root_node_id: 'subgraph:document-set',
      nodes: [
        {
          id: 'subgraph:document-set',
          kind: 'subgraph',
          ir_kind: 'document_set',
          label: 'hot://run/document_set/step-load',
          payload: {},
          refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run/document_set/step-load' } } },
        },
        ...denseDocuments,
      ],
      edges: denseDocuments.map((node, index) => index < 12
        ? {
            id: `edge-in:${index + 1}`,
            from: node.id,
            to: 'subgraph:document-set',
            relation: 'contains',
          }
        : {
            id: `edge-out:${index + 1}`,
            from: 'subgraph:document-set',
            to: node.id,
            relation: 'contains',
          }),
    });

    const documentNodes = graph.nodes.filter((node) => node.id.startsWith('fragment:document-'));
    const rootNode = graph.nodes.find((node) => node.id === 'subgraph:document-set');
    const leftOfRoot = documentNodes.filter((node) => (node.position.x + 1) < (rootNode?.position.x ?? 0)).length;
    const rightOfRoot = documentNodes.filter((node) => node.position.x > ((rootNode?.position.x ?? 0) + 1)).length;
    const aboveRoot = documentNodes.filter((node) => (node.position.y + 1) < (rootNode?.position.y ?? 0)).length;
    const belowRoot = documentNodes.filter((node) => node.position.y > ((rootNode?.position.y ?? 0) + 1)).length;
    const distinctXBands = new Set(documentNodes.map((node) => Math.round(node.position.x / 40))).size;

    expect(rootNode).toBeTruthy();
    expect(leftOfRoot).toBeGreaterThan(0);
    expect(rightOfRoot).toBeGreaterThan(0);
    expect(aboveRoot).toBeGreaterThan(0);
    expect(belowRoot).toBeGreaterThan(0);
    expect(distinctXBands).toBeGreaterThan(2);
  });

  it('keeps chain-heavy graphs on a layered layout', () => {
    const graph = adaptInspectionSubgraph({
      schema_version: 'v1',
      root_node_id: 'node:1',
      nodes: Array.from({ length: 5 }, (_, index) => ({
        id: `node:${index + 1}`,
        kind: 'fragment',
        ir_kind: index === 0 ? 'document_set' : index === 4 ? 'chunk' : 'document',
        label: `node-${index + 1}`,
        payload: {
          value: index === 4
            ? { chunk_id: `node-${index + 1}#chunk-1`, document_id: `doc-${index}` }
            : { filename: `node-${index + 1}.md`, document_id: `doc-${index + 1}` },
        },
      })),
      edges: Array.from({ length: 4 }, (_, index) => ({
        id: `edge:${index + 1}`,
        from: `node:${index + 1}`,
        to: `node:${index + 2}`,
        relation: 'contains',
      })),
    });

    const ordered = graph.nodes
      .slice()
      .sort((left, right) => Number(left.id.split(':')[1]) - Number(right.id.split(':')[1]));
    const xs = ordered.map((node) => node.position.x);
    const isMonotonic = xs.every((value, index) => index === 0 || value >= xs[index - 1]!);

    expect(isMonotonic).toBe(true);
  });

  it('keeps dense organic hub bounds compact enough for fit view', () => {
    const denseDocuments = Array.from({ length: 18 }, (_, index) => ({
      id: `fragment:document-${index + 1}`,
      kind: 'fragment',
      ir_kind: 'document',
      label: `dense-document-${index + 1}.md`,
      payload: {
        value: {
          document_id: `doc-${index + 1}`,
          filename: `dense-document-${index + 1}.md`,
        },
      },
      refs: { self: { backend: 'hot', locator: { cas_id: `frag-${index + 1}` } } },
    }));

    const graph = adaptInspectionSubgraph({
      schema_version: 'v1',
      root_node_id: 'subgraph:document-set',
      nodes: [
        {
          id: 'subgraph:document-set',
          kind: 'subgraph',
          ir_kind: 'document_set',
          label: 'hot://run/document_set/step-load',
          payload: {},
          refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run/document_set/step-load' } } },
        },
        ...denseDocuments,
      ],
      edges: denseDocuments.map((node, index) => index < 12
        ? {
            id: `edge-in:${index + 1}`,
            from: node.id,
            to: 'subgraph:document-set',
            relation: 'contains',
          }
        : {
            id: `edge-out:${index + 1}`,
            from: 'subgraph:document-set',
            to: node.id,
            relation: 'contains',
          }),
    });

    const xs = graph.nodes.map((node) => node.position.x);
    const ys = graph.nodes.map((node) => node.position.y);
    const width = Math.max(...xs) - Math.min(...xs);
    const height = Math.max(...ys) - Math.min(...ys);

    expect(width).toBeLessThan(780);
    expect(height).toBeLessThan(780);
  });
});
