import { act, render } from '@testing-library/react';
import { vi } from 'vitest';

import GraphFlowCanvas from '../components/GraphFlowCanvas';
import { mockFitView } from '../__mocks__/@xyflow/react';

describe('GraphFlowCanvas', () => {
  beforeEach(() => {
    mockFitView.mockReset();
  });

  test('debounces fit view until after the layout settle window', async () => {
    vi.useFakeTimers();
    try {
      render(
        <GraphFlowCanvas
          nodes={[{ id: 'node-1', position: { x: 0, y: 0 }, data: { label: 'Node 1' } }]}
          edges={[]}
          nodeTypes={{}}
          onNodesChange={() => {}}
          onEdgesChange={() => {}}
          testId="graph-flow-canvas"
          fitPadding={0.4}
          fitMaxZoom={0.75}
        />
      );

      expect(mockFitView).not.toHaveBeenCalled();

      await act(async () => {
        vi.advanceTimersByTime(100);
      });
      expect(mockFitView).not.toHaveBeenCalled();

      await act(async () => {
        vi.advanceTimersByTime(60);
      });
      expect(mockFitView).toHaveBeenCalledWith(expect.objectContaining({ padding: 0.4, maxZoom: 0.75 }));
    } finally {
      vi.useRealTimers();
    }
  });

  test('passes the configured minimum zoom through to React Flow', () => {
    render(
      <GraphFlowCanvas
        nodes={[{ id: 'node-1', position: { x: 0, y: 0 }, data: { label: 'Node 1' } }]}
        edges={[]}
        nodeTypes={{}}
        onNodesChange={() => {}}
        onEdgesChange={() => {}}
        testId="graph-flow-canvas"
        fitPadding={0.4}
        fitMaxZoom={0.75}
        minZoom={0.15}
      />
    );

    expect(document.querySelector('[data-testid="mock-react-flow"]')).toHaveAttribute('data-min-zoom', '0.15');
  });
});
