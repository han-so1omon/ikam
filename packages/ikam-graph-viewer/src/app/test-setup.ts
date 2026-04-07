import '@testing-library/jest-dom';

class ResizeObserverMock {
  observe() {}
  unobserve() {}
  disconnect() {}
}
global.ResizeObserver = ResizeObserverMock as any;

// Mock getBoundingClientRect for React Flow
const originalGetBoundingClientRect = Element.prototype.getBoundingClientRect;
Element.prototype.getBoundingClientRect = function () {
  return {
    width: 1000,
    height: 1000,
    top: 0,
    left: 0,
    bottom: 1000,
    right: 1000,
    x: 0,
    y: 0,
    toJSON: () => {}
  };
};

import { vi } from 'vitest';
import React from 'react';
import { mockFitView } from './__mocks__/@xyflow/react';

vi.mock('@xyflow/react', () => {
  return {
    ReactFlow: ({ nodes, nodeTypes, children, nodesDraggable, minZoom }: any) => {
      return React.createElement('div', { 'data-testid': 'mock-react-flow', 'data-nodes-draggable': nodesDraggable ? 'true' : 'false', 'data-min-zoom': String(minZoom ?? '') },
        (nodes || []).map((node: any) => {
          const Component = nodeTypes?.[node.type];
          if (Component) {
            return React.createElement(Component, { key: node.id, data: node.data, id: node.id, type: node.type });
          }
          return React.createElement('div', { key: node.id, 'data-testid': `node-${node.id}` }, node.data?.label || node.id);
        }),
        children
      );
    },
    Controls: () => React.createElement('div', { 'data-testid': 'mock-controls' }),
    Background: () => React.createElement('div', { 'data-testid': 'mock-background' }),
    useNodesInitialized: () => true,
    useReactFlow: () => ({ fitView: mockFitView }),
    useNodesState: (initialNodes: any) => {
      const [nodes, setNodes] = React.useState(initialNodes);
      React.useEffect(() => {
        setNodes(initialNodes);
      }, [initialNodes]);
      return [nodes, setNodes, () => {}];
    },
    useEdgesState: (initialEdges: any) => {
      const [edges, setEdges] = React.useState(initialEdges);
      React.useEffect(() => {
        setEdges(initialEdges);
      }, [initialEdges]);
      return [edges, setEdges, () => {}];
    },
    MarkerType: { ArrowClosed: 'ArrowClosed' },
    Handle: () => React.createElement('div', { 'data-testid': 'mock-handle' }),
    Position: { Left: 'left', Right: 'right', Top: 'top', Bottom: 'bottom' },
  };
});
