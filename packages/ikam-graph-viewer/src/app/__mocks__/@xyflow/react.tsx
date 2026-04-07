import React from 'react';
import { vi } from 'vitest';

export const mockFitView = vi.fn(() => Promise.resolve(true));

export const ReactFlow = ({ nodes, nodeTypes, children, nodesDraggable, minZoom }: any) => {
  return (
    <div data-testid="mock-react-flow" data-nodes-draggable={nodesDraggable ? 'true' : 'false'} data-min-zoom={String(minZoom ?? '')}>
      {nodes.map((node: any) => {
        const Component = nodeTypes?.[node.type];
        if (Component) {
          return <Component key={node.id} data={node.data} id={node.id} type={node.type} />;
        }
        return (
          <div key={node.id} data-testid={`node-${node.id}`}>
            {node.data?.label || node.id}
          </div>
        );
      })}
      {children}
    </div>
  );
};

export const Controls = () => <div data-testid="mock-controls" />;
export const Background = () => <div data-testid="mock-background" />;
export const useNodesInitialized = () => true;
export const useReactFlow = () => ({ fitView: mockFitView });
export const applyNodeChanges = (_changes: any, nodes: any) => nodes;
export const applyEdgeChanges = (_changes: any, edges: any) => edges;
export const useNodesState = (initialNodes: any) => {
  const [nodes, setNodes] = React.useState(initialNodes);
  return [nodes, setNodes, () => {}];
};
export const useEdgesState = (initialEdges: any) => {
  const [edges, setEdges] = React.useState(initialEdges);
  return [edges, setEdges, () => {}];
};
export const MarkerType = { ArrowClosed: 'ArrowClosed' };
export const Handle = () => <div data-testid="mock-handle" />;
export const Position = { Left: 'left', Right: 'right', Top: 'top', Bottom: 'bottom' };
