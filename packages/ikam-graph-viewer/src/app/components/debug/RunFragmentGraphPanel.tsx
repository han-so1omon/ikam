import { useEffect, useMemo, useState } from 'react';
import { Handle, Position, applyNodeChanges, type NodeProps } from '@xyflow/react';

import type { InspectionSubgraphResponse } from '../../api/client';
import GraphFlowCanvas from '../GraphFlowCanvas';
import { adaptInspectionSubgraph } from './inspectionGraphAdapter';
import { createRunFragmentGraphAdapter } from './runFragmentGraphAdapter';

type RunFragmentGraphPanelProps = {
  inspections: InspectionSubgraphResponse[];
  focusNodeId: string | null;
  onSelectNode?: (selection: RunFragmentGraphNodeSelection) => void;
};

export type RunFragmentGraphNodeSelection = {
  id: string;
  kind: string;
  fragmentId: string | null;
  inspectionRef: string | null;
};

type RunGraphNodeData = {
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
  selection: RunFragmentGraphNodeSelection;
  onSelectNode?: (selection: RunFragmentGraphNodeSelection) => void;
};

const readableKind = (value: string | null) => (value ? value.replace(/_/g, ' ') : 'node');

const nodeFilterValue = (node: { data: { kind: string; irKind: string | null } }) => node.data.irKind ?? node.data.kind;

const inspectionIdentity = (inspection: InspectionSubgraphResponse) => {
  const nodeIds = (inspection.nodes ?? [])
    .map((node) => (node && typeof node === 'object' && typeof (node as Record<string, unknown>).id === 'string' ? (node as Record<string, unknown>).id as string : ''))
    .filter((value) => value.length > 0)
    .sort()
    .join(',');
  const edgeIds = (inspection.edges ?? [])
    .map((edge) => {
      if (!edge || typeof edge !== 'object') return '';
      const record = edge as Record<string, unknown>;
      if (typeof record.id === 'string') return record.id;
      const from = typeof record.from === 'string' ? record.from : 'source';
      const to = typeof record.to === 'string' ? record.to : 'target';
      const relation = typeof record.relation === 'string' ? record.relation : 'edge';
      return `${from}:${to}:${relation}`;
    })
    .filter((value) => value.length > 0)
    .sort()
    .join(',');
  return `${inspection.root_node_id ?? ''}|${nodeIds}|${edgeIds}`;
};

function RunGraphNode({ data }: NodeProps<RunGraphNodeData>) {
  return (
    <div className={`inspection-graph-node-shell ${data.kindClassName}${data.isRoot ? ' inspection-graph-node-shell-root' : ''}`}>
      <Handle type="target" position={Position.Left} />
      <button
        type="button"
        title={data.secondaryLabel ?? data.label}
        className={`inspection-graph-node-button ${data.kindClassName.replace('inspection-graph-node-kind-', 'inspection-graph-node-button-kind-')}${data.isRoot ? ' inspection-graph-node-button-root' : ''}`}
        style={data.tone}
        onClick={() => data.onSelectNode?.(data.selection)}
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

const nodeTypes = { runGraphNode: RunGraphNode };
const TOPOLOGY_SETTLE_DELAY_MS = 120;

export default function RunFragmentGraphPanel({ inspections, focusNodeId, onSelectNode }: RunFragmentGraphPanelProps) {
  const expansionLimit = 4;
  const [expandedNodeIds, setExpandedNodeIds] = useState<string[]>([]);
  const [nodeKindFilter, setNodeKindFilter] = useState('all');
  const [relationFilter, setRelationFilter] = useState('all');
  const [searchQuery, setSearchQuery] = useState('');
  const [expansionMessage, setExpansionMessage] = useState<string | null>(null);
  const [nodePositions, setNodePositions] = useState<Record<string, { x: number; y: number }>>({});

  const adaptedInspections = useMemo(() => inspections.map((inspection) => adaptInspectionSubgraph(inspection)), [inspections]);

  const resetKey = useMemo(
    () => `${focusNodeId ?? ''}::${inspections.map((inspection) => inspectionIdentity(inspection)).sort().join('::')}`,
    [focusNodeId, inspections]
  );

  useEffect(() => {
    setExpandedNodeIds([]);
    setNodeKindFilter('all');
    setRelationFilter('all');
    setSearchQuery('');
    setExpansionMessage(null);
    setNodePositions({});
  }, [resetKey]);

  useEffect(() => {
    setExpansionMessage(null);
  }, [nodeKindFilter, relationFilter, searchQuery]);

  const adapter = useMemo(() => {
    const next = createRunFragmentGraphAdapter();
    for (const inspection of inspections) {
      next.ingest(inspection);
    }
    return next;
  }, [inspections]);

  const baseGraph = useMemo(() => {
    if (!focusNodeId) {
      return { nodes: [], edges: [] };
    }
    return expandedNodeIds.length > 0
      ? adapter.getVisibleGraph({ focusNodeId, expandedNodeIds })
      : adapter.getVisibleGraph({ focusNodeId });
  }, [adapter, expandedNodeIds, focusNodeId]);

  const topologyKey = useMemo(
    () => `${focusNodeId ?? ''}::${baseGraph.nodes.map((node) => node.id).join('|')}::${baseGraph.edges.map((edge) => edge.id).join('|')}::${nodeKindFilter}::${relationFilter}::${searchQuery.trim().toLowerCase()}`,
    [baseGraph.edges, baseGraph.nodes, focusNodeId, nodeKindFilter, relationFilter, searchQuery]
  );

  const [settledTopologyKey, setSettledTopologyKey] = useState(topologyKey);
  const [settledBaseGraph, setSettledBaseGraph] = useState(baseGraph);

  useEffect(() => {
    const handle = window.setTimeout(() => {
      setSettledTopologyKey(topologyKey);
      setSettledBaseGraph(baseGraph);
    }, TOPOLOGY_SETTLE_DELAY_MS);
    return () => window.clearTimeout(handle);
  }, [baseGraph, topologyKey]);

  const nodeKindOptions = useMemo(() => {
    const values = new Set<string>();
    for (const graph of adaptedInspections) {
      for (const node of graph.nodes) {
        const value = nodeFilterValue(node);
        if (value) values.add(value);
      }
    }
    return Array.from(values).sort((left, right) => left.localeCompare(right));
  }, [adaptedInspections]);

  const relationOptions = useMemo(() => {
    const values = new Set<string>();
    for (const graph of adaptedInspections) {
      for (const edge of graph.edges) {
        if (edge.data.relation) values.add(edge.data.relation);
      }
    }
    return Array.from(values).sort((left, right) => left.localeCompare(right));
  }, [adaptedInspections]);

  const visibleNodeIds = useMemo(() => {
    const query = searchQuery.trim().toLowerCase();
    const ids = new Set(
      settledBaseGraph.nodes
        .filter((node) => {
          if (node.id === focusNodeId) {
            return true;
          }
          if (nodeKindFilter !== 'all' && nodeFilterValue(node) !== nodeKindFilter) {
            return false;
          }
          if (!query) {
            return true;
          }
          const text = `${node.data.label} ${node.data.kind} ${node.data.irKind ?? ''}`.toLowerCase();
          return text.includes(query);
        })
        .map((node) => node.id)
    );
    if (focusNodeId) {
      ids.add(focusNodeId);
    }
    return ids;
  }, [focusNodeId, nodeKindFilter, searchQuery, settledBaseGraph.nodes]);

  const activeNodeKinds = nodeKindFilter === 'all' ? undefined : [nodeKindFilter];
  const activeRelations = relationFilter === 'all' ? undefined : [relationFilter];

  const graphHeight = useMemo(() => {
    if (settledBaseGraph.nodes.length >= 16) {
      return '520px';
    }
    if (settledBaseGraph.nodes.length >= 10) {
      return '420px';
    }
    return '320px';
  }, [settledBaseGraph.nodes.length]);

  const fitPadding = settledBaseGraph.nodes.length >= 16 ? 0.4 : settledBaseGraph.nodes.length >= 10 ? 0.3 : 0.2;
  const fitMaxZoom = settledBaseGraph.nodes.length >= 16 ? 0.75 : settledBaseGraph.nodes.length >= 10 ? 0.9 : undefined;
  const minZoom = settledBaseGraph.nodes.length >= 16 ? 0.15 : settledBaseGraph.nodes.length >= 10 ? 0.25 : 0.5;

  const graph = useMemo(() => {
    const edges = settledBaseGraph.edges.filter((edge) => {
      if (!visibleNodeIds.has(edge.source) || !visibleNodeIds.has(edge.target)) {
        return false;
      }
      if (relationFilter !== 'all' && edge.data.relation !== relationFilter) {
        return false;
      }
      return true;
    });
    const edgeNodeIds = new Set<string>();
    for (const edge of edges) {
      edgeNodeIds.add(edge.source);
      edgeNodeIds.add(edge.target);
    }
    if (focusNodeId) {
      edgeNodeIds.add(focusNodeId);
    }
    const nodes = settledBaseGraph.nodes
      .filter((node) => visibleNodeIds.has(node.id) && (relationFilter === 'all' || edgeNodeIds.has(node.id)))
      .map((node) => ({
        ...node,
        position: nodePositions[node.id] ?? node.position,
        type: 'runGraphNode' as const,
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
              node.data.raw.payload && typeof node.data.raw.payload === 'object' && typeof (node.data.raw.payload as Record<string, unknown>).cas_id === 'string'
                ? (node.data.raw.payload as Record<string, unknown>).cas_id as string
                : null,
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
      }));
    return { nodes, edges };
  }, [focusNodeId, nodePositions, onSelectNode, relationFilter, settledBaseGraph.edges, settledBaseGraph.nodes, settledTopologyKey, visibleNodeIds]);

  const onNodesChange = (changes: any[]) => {
    setNodePositions((current) => {
      const next = { ...current };
      for (const change of changes) {
        if (change?.type === 'position' && change.position && typeof change.id === 'string') {
          next[change.id] = change.position;
        }
      }
      applyNodeChanges(changes, graph.nodes);
      return next;
    });
  };

  if (!focusNodeId || settledBaseGraph.nodes.length === 0) {
    return null;
  }

  return (
    <section className="runs-fragment-drill-graph" data-testid="run-fragment-graph-panel">
      <div className="runs-fragment-drill-preview-head">
        <h6>Fragment Graph</h6>
      </div>
      <div className="runs-fragment-drill-fields">
        <label>
          <span>Search graph</span>
          <input
            aria-label="Search graph"
            type="search"
            value={searchQuery}
            onChange={(event) => setSearchQuery(event.target.value)}
          />
        </label>
        <label>
          <span>Node kind filter</span>
          <select aria-label="Node kind filter" value={nodeKindFilter} onChange={(event) => setNodeKindFilter(event.target.value)}>
            <option value="all">all</option>
            {nodeKindOptions.map((option) => (
              <option key={option} value={option}>{option}</option>
            ))}
          </select>
        </label>
        <label>
          <span>Relation filter</span>
          <select aria-label="Relation filter" value={relationFilter} onChange={(event) => setRelationFilter(event.target.value)}>
            <option value="all">all</option>
            {relationOptions.map((option) => (
              <option key={option} value={option}>{option}</option>
            ))}
          </select>
        </label>
      </div>
      <div className="runs-fragment-drill-links-head">
        <button
          type="button"
          className="runs-fragment-drill-link"
          onClick={() => {
            const expanded = adapter.expandVisibleGraph({
              focusNodeId,
              visibleNodeIds: graph.nodes.map((node) => node.id),
              nodeKinds: activeNodeKinds,
              relations: activeRelations,
              searchQuery,
              maxNewNeighbors: expansionLimit,
            });
            if (expanded.expansionLimited) {
              setExpansionMessage('Too many neighbors to expand at once. Narrow with filters or search first.');
              return;
            }
            setExpansionMessage(null);
            setExpandedNodeIds(expanded.nodes.map((node) => node.id));
          }}
        >
          Expand one hop
        </button>
        <button
          type="button"
          className="runs-fragment-drill-link"
          onClick={() => {
            setExpansionMessage(null);
            setExpandedNodeIds([]);
          }}
        >
          Collapse to focus
        </button>
      </div>
      {expansionMessage ? <p>{expansionMessage}</p> : null}
      <GraphFlowCanvas
        nodes={graph.nodes}
        edges={graph.edges}
        onNodesChange={onNodesChange}
        onEdgesChange={() => {}}
        nodeTypes={nodeTypes}
        ariaLabel="Run fragment graph"
        testId="run-fragment-graph-canvas"
        height={graphHeight}
        fitPadding={fitPadding}
        fitMaxZoom={fitMaxZoom}
        minZoom={minZoom}
        fitDelayMs={TOPOLOGY_SETTLE_DELAY_MS}
        nodesDraggable
      />
    </section>
  );
}
