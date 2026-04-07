import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import RunFragmentGraphPanel from '../components/debug/RunFragmentGraphPanel';

describe('RunFragmentGraphPanel', () => {
  test('debounces topology-driven graph expansion before showing new nodes', async () => {
    render(
      <RunFragmentGraphPanel
        focusNodeId="subgraph:document-set"
        inspections={[
          {
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
              {
                id: 'fragment:document-1',
                kind: 'fragment',
                ir_kind: 'document',
                label: 'alpha.md',
                payload: { cas_id: 'frag-doc-1', value: { document_id: 'doc-1', filename: 'alpha.md' } },
                refs: { self: { backend: 'hot', locator: { cas_id: 'frag-doc-1' } } },
              },
              {
                id: 'fragment:chunk-1',
                kind: 'fragment',
                ir_kind: 'chunk',
                label: 'alpha.md#chunk-1',
                payload: { cas_id: 'frag-chunk-1', value: { chunk_id: 'alpha.md#chunk-1', document_id: 'doc-1' } },
                refs: { self: { backend: 'hot', locator: { cas_id: 'frag-chunk-1' } } },
              },
            ],
            edges: [
              { id: 'edge-root-doc', from: 'subgraph:document-set', to: 'fragment:document-1', relation: 'contains' },
              { id: 'edge-doc-chunk', from: 'fragment:document-1', to: 'fragment:chunk-1', relation: 'contains' },
            ],
          },
        ]}
      />
    );

    const graphPanel = await screen.findByTestId('run-fragment-graph-panel');
    vi.useFakeTimers();
    try {
      expect(within(graphPanel).getByRole('button', { name: /alpha\.md/i })).toBeInTheDocument();
      expect(within(graphPanel).queryByRole('button', { name: /chunk-1/i })).not.toBeInTheDocument();

      fireEvent.click(within(graphPanel).getByRole('button', { name: /expand one hop/i }));

      expect(within(graphPanel).queryByRole('button', { name: /chunk-1/i })).not.toBeInTheDocument();

      await act(async () => {
        vi.advanceTimersByTime(100);
      });
      expect(within(graphPanel).queryByRole('button', { name: /chunk-1/i })).not.toBeInTheDocument();

      await act(async () => {
        vi.advanceTimersByTime(40);
      });
      expect(within(graphPanel).getByRole('button', { name: /chunk-1/i })).toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });
});
