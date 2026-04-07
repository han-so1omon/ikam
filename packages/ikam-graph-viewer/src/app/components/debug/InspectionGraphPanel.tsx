import { useMemo } from 'react';
import { Handle, Position, type NodeProps } from '@xyflow/react';

import type { InspectionSubgraphResponse } from '../../api/client';
import GraphFlowCanvas from '../GraphFlowCanvas';
import { adaptInspectionSubgraph } from './inspectionGraphAdapter';

const readableKind = (value: string | null) => (value ? value.replace(/_/g, ' ') : 'node');

type InspectionGraphPanelProps = {
  inspection: InspectionSubgraphResponse;
  onSelectNode: (selection: InspectionGraphNodeSelection) => void;
};

export type InspectionGraphNodeSelection = {
  id: string;
  kind: string;
  fragmentId: string | null;
  inspectionRef: string | null;
};

type InspectionNodeData = {
  label: string;
  secondaryLabel: string | null;
  icon: string;
  compact: boolean;
  kind: string;
  irKind: string | null;
  kindClassName: string;
  tone: {
    backgroundColor: string;
    borderColor: string;
    color: string;
  };
  isRoot: boolean;
  selection: InspectionGraphNodeSelection;
  onSelectNode: (selection: InspectionGraphNodeSelection) => void;
};

function InspectionNode({ data }: NodeProps<InspectionNodeData>) {
  return (
    <div className={`inspection-graph-node-shell ${data.kindClassName}${data.isRoot ? ' inspection-graph-node-shell-root' : ''}`}>
      <Handle type="target" position={Position.Left} />
      <button
        type="button"
        title={data.secondaryLabel ?? data.label}
        className={`inspection-graph-node-button ${data.kindClassName.replace('inspection-graph-node-kind-', 'inspection-graph-node-button-kind-')}${data.isRoot ? ' inspection-graph-node-button-root' : ''}`}
        style={data.tone}
        onClick={() => data.onSelectNode(data.selection)}
      >
        {data.compact ? (
          <>
            <strong>{data.icon}</strong>
            <span>{data.label}</span>
          </>
        ) : (
          <>
            <strong>{data.label}</strong>
            {data.secondaryLabel ? <span>{data.secondaryLabel}</span> : null}
            <span>{readableKind(data.irKind ?? data.kind)}</span>
          </>
        )}
      </button>
      <Handle type="source" position={Position.Right} />
    </div>
  );
}

const nodeTypes = { inspectionNode: InspectionNode };

export default function InspectionGraphPanel({ inspection, onSelectNode }: InspectionGraphPanelProps) {
  const graph = useMemo(() => {
    const adapted = adaptInspectionSubgraph(inspection);
    return {
      nodes: adapted.nodes.map((node) => ({
        ...node,
        type: 'inspectionNode',
        data: {
          label: node.data.label,
          secondaryLabel: node.data.secondaryLabel,
          icon: node.data.icon,
          compact: node.data.compact,
          kind: node.data.kind,
          irKind: node.data.irKind,
          kindClassName: node.data.kindClassName,
          tone: node.data.tone,
          isRoot: node.data.isRoot,
          selection: {
            id: node.id,
            kind: node.data.kind,
            fragmentId:
              (node.data.raw.payload && typeof node.data.raw.payload === 'object' && typeof (node.data.raw.payload as Record<string, unknown>).cas_id === 'string'
                ? (node.data.raw.payload as Record<string, unknown>).cas_id as string
                : null),
            inspectionRef:
              node.data.raw.refs && typeof node.data.raw.refs === 'object'
                ? (() => {
                    const selfRef = (node.data.raw.refs as Record<string, unknown>).self;
                    if (!selfRef || typeof selfRef !== 'object') return null;
                    const locator = (selfRef as Record<string, unknown>).locator;
                    if (!locator || typeof locator !== 'object') return null;
                    const locatorRecord = locator as Record<string, unknown>;
                    return typeof locatorRecord.cas_id === 'string'
                      ? `inspect://fragment/${locatorRecord.cas_id}`
                      : typeof locatorRecord.subgraph_ref === 'string'
                        ? `inspect://subgraph/${locatorRecord.subgraph_ref}`
                        : null;
                  })()
                : null,
          },
          onSelectNode,
        },
      })),
      edges: adapted.edges,
    };
  }, [inspection, onSelectNode]);

  if (graph.nodes.length === 0) {
    return null;
  }

  return (
    <section className="runs-fragment-drill-graph" data-testid="inspection-graph-panel">
      <div className="runs-fragment-drill-preview-head">
        <h6>Inspection Graph</h6>
      </div>
      <GraphFlowCanvas
        nodes={graph.nodes}
        edges={graph.edges}
        onNodesChange={() => {}}
        onEdgesChange={() => {}}
        nodeTypes={nodeTypes}
        ariaLabel="Inspection graph"
        testId="inspection-graph-canvas"
        height="320px"
      />
    </section>
  );
}
