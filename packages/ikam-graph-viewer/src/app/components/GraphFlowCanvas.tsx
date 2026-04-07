import { ReactNode, useEffect, useMemo } from 'react';
import { Background, Controls, ReactFlow, useNodesInitialized, useReactFlow } from '@xyflow/react';

type GraphFlowCanvasProps = {
  nodes: any[];
  edges: any[];
  nodeTypes: Record<string, any>;
  onNodesChange: (changes: any) => void;
  onEdgesChange: (changes: any) => void;
  ariaLabel?: string;
  testId?: string;
  height?: string;
  nodesDraggable?: boolean;
  fitPadding?: number;
  fitMaxZoom?: number;
  minZoom?: number;
  fitDelayMs?: number;
  children?: ReactNode;
};

function AutoFitView({ fitKey, fitPadding, fitMaxZoom, fitDelayMs }: { fitKey: string; fitPadding: number; fitMaxZoom?: number; fitDelayMs: number }) {
  const nodesInitialized = useNodesInitialized();
  const { fitView } = useReactFlow();

  useEffect(() => {
    if (!nodesInitialized) {
      return;
    }
    const handle = window.setTimeout(() => {
      void fitView({ padding: fitPadding, duration: 150, maxZoom: fitMaxZoom });
    }, fitDelayMs);
    return () => window.clearTimeout(handle);
  }, [fitDelayMs, fitKey, fitMaxZoom, fitPadding, fitView, nodesInitialized]);

  return null;
}

export default function GraphFlowCanvas({
  nodes,
  edges,
  nodeTypes,
  onNodesChange,
  onEdgesChange,
  ariaLabel,
  testId,
  height = '100%',
  nodesDraggable = true,
  fitPadding = 0.2,
  fitMaxZoom,
  minZoom,
  fitDelayMs = 120,
  children,
}: GraphFlowCanvasProps) {
  const fitKey = useMemo(
    () => `${nodes.map((node) => node.id).join('|')}::${edges.map((edge) => edge.id).join('|')}`,
    [edges, nodes],
  );

  return (
    <div
      className="graph-flow-canvas"
      data-testid={testId}
      data-nodes-draggable={nodesDraggable ? 'true' : 'false'}
      role={ariaLabel ? 'figure' : undefined}
      aria-label={ariaLabel}
      style={{ width: '100%', height }}
    >
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        nodeTypes={nodeTypes}
        nodesDraggable={nodesDraggable}
        fitView
        minZoom={minZoom}
        attributionPosition="bottom-right"
      >
        <Background gap={12} size={1} />
        <Controls />
        <AutoFitView fitKey={fitKey} fitPadding={fitPadding} fitMaxZoom={fitMaxZoom} fitDelayMs={fitDelayMs} />
        {children}
      </ReactFlow>
    </div>
  );
}
