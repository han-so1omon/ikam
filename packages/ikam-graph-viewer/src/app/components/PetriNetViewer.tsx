import { useMemo, useRef, useEffect } from 'react';
import { 
  MarkerType, 
  Node, 
  Edge,
  Handle,
  Position,
  useNodesState,
  useEdgesState
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import GraphFlowCanvas from './GraphFlowCanvas';
import { getLayoutedElements } from './graphFlowLayout';

const layoutPetriElements = (nodes: Node[], edges: Edge[]) => {
  try {
    return getLayoutedElements(nodes, edges, {
      direction: 'TB',
      getNodeSize: (node) => ({ width: node.type === 'place' ? 40 : 100, height: node.type === 'place' ? 40 : 40 }),
    });
  } catch {
    return { nodes, edges };
  }
};

type PetriNetViewerProps = {
  head: any;
  childrenFragments: Record<string, any>;
  executedTransitions?: Set<string>;
  runningTransitionId?: string | null;
  selectedTransitionId?: string | null;
  validationStates?: Record<string, 'passed' | 'failed' | 'mixed'>;
  onNodeClick?: (id: string) => void;
};

// Custom Node for Place (Circle)
const PlaceNode = ({ data }: { data: any }) => {
  const validationStatus = data.validationStatus;
  const borderColor = validationStatus === 'failed' ? '#b42318' : validationStatus === 'passed' ? '#027a48' : validationStatus === 'mixed' ? '#b54708' : '#3b82f6';
  const bgColor = validationStatus === 'failed' ? '#fef3f2' : validationStatus === 'passed' ? '#ecfdf3' : validationStatus === 'mixed' ? '#fff7ed' : '#eff6ff';
  return (
    <div style={{
      width: 40, 
      height: 40, 
      borderRadius: '50%', 
      border: `2px solid ${borderColor}`,
      backgroundColor: bgColor,
      display: 'flex',
      alignItems: 'center',
      justifyContent: 'center',
      position: 'relative'
    }} data-testid={`place-${data.id}`} data-validation-status={validationStatus ?? 'none'}>
      <Handle type="target" position={Position.Top} style={{ visibility: 'hidden' }} />
      <span style={{ fontSize: '10px', color: '#1d4ed8', fontWeight: 'bold' }}>P</span>
      <div style={{ position: 'absolute', bottom: -20, fontSize: '10px', whiteSpace: 'nowrap', color: 'var(--text-color)' }}>
        {data.label}
      </div>
      <Handle type="source" position={Position.Bottom} style={{ visibility: 'hidden' }} />
    </div>
  );
};

// Custom Node for Transition (Rectangle)
const TransitionNode = ({ data }: { data: any }) => {
  const isExecuted = data.isExecuted;
  const isRunning = data.isRunning;
  const isSelected = data.isSelected;
  const isClickable = data.isClickable;
  const validationStatus = data.validationStatus;

  let borderColor = '#10b981';
  let bgColor = '#ecfdf5';
  let textColor = '#047857';

  if (!isExecuted && isClickable !== undefined) {
    borderColor = '#9ca3af';
    bgColor = '#f3f4f6';
    textColor = '#4b5563';
  }

  if (isRunning) {
    borderColor = '#2563eb';
    bgColor = '#dbeafe';
    textColor = '#1d4ed8';
  }

  if (isSelected) {
    borderColor = '#f59e0b';
    bgColor = '#fef3c7';
    textColor = '#b45309';
  }

  if (validationStatus === 'failed') {
    borderColor = '#b42318';
  } else if (validationStatus === 'passed') {
    borderColor = '#027a48';
  } else if (validationStatus === 'mixed') {
    borderColor = '#b54708';
  }

  return (
    <div 
      onClick={isClickable && data.onClick ? () => data.onClick(data.id) : undefined}
      data-testid={`transition-${data.id}`}
      data-status={isRunning ? 'running' : isExecuted ? 'succeeded' : 'idle'}
      data-validation-status={validationStatus ?? 'none'}
      style={{
        padding: '8px', 
        borderRadius: '4px', 
        border: `2px solid ${borderColor}`, 
        backgroundColor: bgColor,
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        position: 'relative',
        minWidth: '60px',
        cursor: isClickable ? 'pointer' : 'default',
        boxShadow: isSelected ? '0 0 0 4px rgba(245, 158, 11, 0.2)' : 'none',
        transition: 'all 0.2s ease',
      }}>
      <Handle type="target" position={Position.Top} style={{ visibility: 'hidden' }} />
      <div style={{ fontSize: '12px', color: textColor, fontWeight: 'bold', textAlign: 'center' }}>
        {data.label}
      </div>
      {validationStatus && validationStatus !== 'none' ? (
        <div
          style={{
            position: 'absolute',
            top: -8,
            right: -8,
            minWidth: 16,
            height: 16,
            padding: '0 4px',
            borderRadius: 999,
            backgroundColor: validationStatus === 'failed' ? '#f04438' : validationStatus === 'passed' ? '#12b76a' : '#f79009',
            color: '#fff',
            fontSize: 9,
            fontWeight: 700,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            boxShadow: '0 1px 4px rgba(15, 23, 42, 0.24)',
          }}
        >
          {validationStatus === 'failed' ? '!' : validationStatus === 'passed' ? 'OK' : '~'}
        </div>
      ) : null}
      <Handle type="source" position={Position.Bottom} style={{ visibility: 'hidden' }} />
    </div>
  );
};

const nodeTypes = {
  place: PlaceNode,
  transition: TransitionNode,
};

export default function PetriNetViewer({ head, childrenFragments = {}, executedTransitions, runningTransitionId, selectedTransitionId, validationStates = {}, onNodeClick }: PetriNetViewerProps) {
  const onNodeClickRef = useRef(onNodeClick);
  useEffect(() => {
    onNodeClickRef.current = onNodeClick;
  }, [onNodeClick]);

  const executedTransitionsKey = useMemo(() => {
    if (!executedTransitions) return '';
    return Array.from(executedTransitions).sort().join(',');
  }, [executedTransitions]);

  const { nodes: initialNodes, edges: initialEdges } = useMemo(() => {
    const richPetriSnapshot = head?.data?.rich_petri_snapshot;
    if (richPetriSnapshot && typeof richPetriSnapshot === 'object') {
      const places = Array.isArray(richPetriSnapshot.places) ? richPetriSnapshot.places : [];
      const transitions = Array.isArray(richPetriSnapshot.transitions) ? richPetriSnapshot.transitions : [];
      const arcs = Array.isArray(richPetriSnapshot.arcs) ? richPetriSnapshot.arcs : [];

      const nodes: Node[] = [];
      const placeValidationStates: Record<string, 'passed' | 'failed' | 'mixed'> = {};
      const selectedValidationStatus = selectedTransitionId ? validationStates[selectedTransitionId] : undefined;
      if (selectedTransitionId && selectedValidationStatus) {
        arcs.forEach((arc: any) => {
          if (arc?.source_id === selectedTransitionId && typeof arc.target_id === 'string') {
            placeValidationStates[arc.target_id] = selectedValidationStatus;
          }
          if (arc?.target_id === selectedTransitionId && typeof arc.source_id === 'string') {
            placeValidationStates[arc.source_id] = selectedValidationStatus;
          }
        });
      }

      places.forEach((p: any) => {
        if (p?.place_id) {
          nodes.push({
            id: p.place_id,
            type: 'place',
            position: { x: 0, y: 0 },
            data: { id: p.place_id, label: p.label || p.place_id, validationStatus: placeValidationStates[p.place_id] },
          });
        }
      });

      transitions.forEach((t: any) => {
        const tId = t?.transition_id;
        if (tId) {
          const isExecuted = executedTransitions ? executedTransitions.has(tId) : undefined;
          const isRunning = runningTransitionId === tId;
          const isClickable = executedTransitions ? (isExecuted || isRunning) : undefined;
          nodes.push({
            id: tId,
            type: 'transition',
            position: { x: 0, y: 0 },
            data: {
              id: tId,
              label: t.label || tId,
              isExecuted,
              isRunning,
              isClickable,
              isSelected: selectedTransitionId === tId,
              validationStatus: validationStates[tId],
              onClick: (id: string) => onNodeClickRef.current?.(id),
            },
          });
        }
      });

      const edges: Edge[] = arcs.map((a: any, i: number) => {
        const isConnectedToSelected = selectedTransitionId && (a.source_id === selectedTransitionId || a.target_id === selectedTransitionId);
        const edgeColor = isConnectedToSelected && selectedValidationStatus
          ? selectedValidationStatus === 'failed'
            ? '#f04438'
            : selectedValidationStatus === 'passed'
              ? '#12b76a'
              : '#f79009'
          : '#6b7280';
        return {
          id: `e-${a.source_id}-${a.target_id}-${i}`,
          source: a.source_id,
          target: a.target_id,
          markerEnd: {
            type: MarkerType.ArrowClosed,
            color: edgeColor,
          },
          style: { stroke: edgeColor, strokeWidth: isConnectedToSelected ? 3 : 2 },
        };
      });

      return layoutPetriElements(nodes, edges);
    }

    const safeChildrenFragments = childrenFragments || {};
    const places = Object.values(safeChildrenFragments).filter(f => f.profile === 'petri_place' || f.data?.schema_id === 'modelado/petri-net-place@1' || f.schema_id === 'modelado/petri-net-place@1');
    const transitions = Object.values(safeChildrenFragments).filter(f => 
      f.profile === 'petri_transition' || 
      f.data?.schema_id === 'modelado/petri-net-transition@1' || 
      f.schema_id === 'modelado/petri-net-transition@1' ||
      f.ast?.params?.transition_data?.schema_id === 'modelado/petri-net-transition@1'
    );
    const arcs = Object.values(safeChildrenFragments).filter(f => 
      f.profile === 'petri_arc' || 
      f.data?.schema_id === 'modelado/petri-net-arc@1' || 
      f.schema_id === 'modelado/petri-net-arc@1' ||
      f.statement?.schema_id === 'modelado/petri-net-arc@1'
    );
    const nodes: Node[] = [];
    const placeValidationStates: Record<string, 'passed' | 'failed' | 'mixed'> = {};
    const selectedValidationStatus = selectedTransitionId ? validationStates[selectedTransitionId] : undefined;

    if (selectedTransitionId && selectedValidationStatus) {
      arcs.forEach((a: any) => {
        const data = a.data || a.statement || {};
        if (data.from_id === selectedTransitionId && typeof data.to_id === 'string') {
          placeValidationStates[data.to_id] = selectedValidationStatus;
        }
        if (data.to_id === selectedTransitionId && typeof data.from_id === 'string') {
          placeValidationStates[data.from_id] = selectedValidationStatus;
        }
      });
    }
    
    places.forEach(p => {
      const data = p.data || {};
      if (data.place_id) nodes.push({
        id: data.place_id,
        type: 'place',
        position: { x: 0, y: 0 },
        data: { id: data.place_id, label: data.label || data.place_id, validationStatus: placeValidationStates[data.place_id] }
      });
    });

    transitions.forEach(t => {
      const data = t.data || t.ast?.params?.transition_data || {};
      const tId = data.transition_id;
      if (tId) { const isExecuted = executedTransitions ? executedTransitions.has(tId) : undefined;
      const isRunning = runningTransitionId === tId;
      const isClickable = executedTransitions ? (isExecuted || isRunning) : undefined;
      
      nodes.push({
        id: tId,
        type: 'transition',
        position: { x: 0, y: 0 },
        data: { 
          id: tId,
          label: data.label || tId,
          isExecuted,
          isRunning,
          isClickable,
          isSelected: selectedTransitionId === tId,
          validationStatus: validationStates[tId],
          onClick: (id: string) => onNodeClickRef.current?.(id)
        }
      });
      }
    });

    const edges: Edge[] = arcs.map((a, i) => {
      const data = a.data || a.statement || {};
      if (!data.from_id || !data.to_id) return null as any;
      const isConnectedToSelected = selectedTransitionId && (data.from_id === selectedTransitionId || data.to_id === selectedTransitionId);
      const edgeColor = isConnectedToSelected && selectedValidationStatus
        ? selectedValidationStatus === 'failed'
          ? '#f04438'
          : selectedValidationStatus === 'passed'
            ? '#12b76a'
            : '#f79009'
        : '#6b7280';
      return {
        id: `e-${data.from_id}-${data.to_id}-${i}`,
        source: data.from_id,
        target: data.to_id,
        markerEnd: {
          type: MarkerType.ArrowClosed,
          color: edgeColor,
        },
        style: { stroke: edgeColor, strokeWidth: isConnectedToSelected ? 3 : 2 },
      };
    }).filter(Boolean);

    return layoutPetriElements(nodes, edges);
  }, [childrenFragments, executedTransitionsKey, runningTransitionId, selectedTransitionId, validationStates]);

  const [rfNodes, setNodes, onNodesChange] = useNodesState(initialNodes);
  const [rfEdges, setEdges, onEdgesChange] = useEdgesState(initialEdges);

  useEffect(() => {
    setNodes(initialNodes);
    setEdges(initialEdges);
  }, [initialNodes, initialEdges, setNodes, setEdges]);

  if (initialNodes.length === 0) {
    return <div style={{ padding: '1rem', color: 'gray' }}>No visual nodes found for this Petri Net.</div>;
  }

  return (
    <GraphFlowCanvas
      nodes={rfNodes}
      edges={rfEdges}
      onNodesChange={onNodesChange}
      onEdgesChange={onEdgesChange}
      nodeTypes={nodeTypes}
    />
  );
}
