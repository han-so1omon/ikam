import React, { useMemo } from 'react';
import {
  MarkerType,
  Handle,
  Position,
  NodeProps,
  Edge,
  useNodesState,
  useEdgesState,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import GraphFlowCanvas from '../GraphFlowCanvas';
import { getLayoutedElements } from '../graphFlowLayout';

export type TransformationFlowNode = {
  id: string;
  label: string;
  caption?: string;
  nodeType?: 'data' | 'step';
  kind: 'artifact' | 'surface' | 'ir' | 'normalized' | 'operation';
  stage: 'artifact' | 'surface' | 'ir' | 'normalized';
  summary: string;
  state: 'complete' | 'attention' | 'neutral';
  linkedNodeId?: string;
};

export type TransformationFlowEdge = {
  id: string;
  from: string;
  to: string;
};

type TransformationFlowGraphProps = {
  nodes: TransformationFlowNode[];
  edges: TransformationFlowEdge[];
  ariaLabel: string;
  onNodeClick?: (nodeId: string) => void;
};

const abbreviateNodeLabel = (label: string, kind: TransformationFlowNode['kind']): string => {
  if (label.length <= 22) {
    return label;
  }
  if (kind === 'artifact') {
    return `${label.slice(0, 10)}...${label.slice(-8)}`;
  }
  return `${label.slice(0, 12)}...${label.slice(-6)}`;
};

const CustomNode = ({ data }: NodeProps) => {
  const isStep = data.nodeType === 'step';
  const stateAccent = data.state === 'complete' ? (isStep ? '#0f172a' : '#2563eb') : data.state === 'attention' ? '#f59e0b' : '#9ca3af';
  const stateSurface = data.state === 'complete' ? (isStep ? '#0f172a' : '#eff6ff') : data.state === 'attention' ? (isStep ? '#78350f' : '#fef3c7') : (isStep ? '#334155' : '#f3f4f6');
  const stateText = data.state === 'complete' ? (isStep ? '#f8fafc' : '#1d4ed8') : data.state === 'attention' ? (isStep ? '#fef3c7' : '#b45309') : (isStep ? '#e2e8f0' : '#4b5563');
  const shadowColor = data.state === 'attention' ? 'rgba(245, 158, 11, 0.18)' : isStep ? 'rgba(15, 23, 42, 0.18)' : 'rgba(37, 99, 235, 0.16)';
  return (
    <div
      className={`flow-node flow-node-${data.state}`}
      data-testid={`transformation-flow-node-${data.id}`}
      data-kind={data.kind}
      data-state={data.state}
      style={{
        minWidth: '110px',
        maxWidth: '160px',
        minHeight: '44px',
        padding: '8px 12px',
        borderRadius: '4px',
        border: `2px solid ${stateAccent}`,
        backgroundColor: stateSurface,
        color: stateText,
        cursor: data.clickable ? 'pointer' : 'default',
        boxSizing: 'border-box',
        transition: 'all 0.2s ease',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        position: 'relative',
        fontSize: '12px',
        fontWeight: 700,
        textAlign: 'center',
      }}
      title={`${data.label} (${data.kind}): ${data.summary}`}
      onClick={data.onClick}
    >
      <Handle type="target" position={Position.Left} style={{ visibility: 'hidden' }} />
      {data.nodeType ? (
        <div style={{ position: 'absolute', top: 4, left: 8, fontSize: '9px', letterSpacing: '0.08em', textTransform: 'uppercase', opacity: 0.8 }}>
          {data.nodeType}
        </div>
      ) : null}
      <div className="flow-node-title" style={{ width: '100%', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', padding: '0 4px' }}>
        {data.abbreviatedLabel}
      </div>
      {data.caption ? (
        <div style={{ position: 'absolute', bottom: -18, left: 0, right: 0, fontSize: '10px', lineHeight: 1.1, color: 'var(--muted-text-color, #64748b)', whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', textAlign: 'center' }}>
          {data.caption}
        </div>
      ) : null}
      <Handle type="source" position={Position.Right} style={{ visibility: 'hidden' }} />
    </div>
  );
};

const nodeTypes = {
  custom: CustomNode,
};

const TransformationFlowGraph = ({ nodes, edges, ariaLabel, onNodeClick }: TransformationFlowGraphProps) => {
  const { nodes: initialNodes, edges: initialEdges } = useMemo(() => {
    const nextNodes = nodes.map((node) => ({
      id: node.id,
      type: 'custom',
      data: {
        ...node,
        abbreviatedLabel: abbreviateNodeLabel(node.label, node.kind),
        clickable: node.nodeType === 'step' || !!node.linkedNodeId,
        onClick: () => {
          if (onNodeClick) {
            onNodeClick(node.id);
          }
        },
      },
      position: { x: 0, y: 0 },
    }));

    const nextEdges: Edge[] = edges.map((edge) => ({
      id: edge.id,
      source: edge.from,
      target: edge.to,
      type: 'smoothstep',
      animated: false,
      style: { stroke: '#9ca3af', strokeWidth: 2 },
      markerEnd: {
        type: MarkerType.ArrowClosed,
        color: '#9ca3af',
      },
    }));

    return getLayoutedElements(nextNodes, nextEdges, {
      direction: 'LR',
      ranksep: 44,
      nodesep: 18,
      getNodeSize: () => ({ width: 140, height: 44 }),
    });
  }, [nodes, edges, onNodeClick]);

  const [rfNodes, setNodes, onNodesChange] = useNodesState(initialNodes);
  const [rfEdges, setEdges, onEdgesChange] = useEdgesState(initialEdges);

  React.useEffect(() => {
    setNodes(initialNodes);
    setEdges(initialEdges);
  }, [initialNodes, initialEdges, setNodes, setEdges]);

  if (nodes.length === 0) {
    return null;
  }

  return (
    <GraphFlowCanvas
      testId="transformation-flow-graph"
      ariaLabel={ariaLabel}
      height="330px"
      nodes={rfNodes}
      edges={rfEdges}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      nodeTypes={nodeTypes}
    />
  );
};

export default TransformationFlowGraph;
