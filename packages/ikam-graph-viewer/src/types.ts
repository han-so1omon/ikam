export type GraphNode = {
  id: string;
  type: string;
  label?: string;
  level?: number;
  salience?: number;
  meta?: Record<string, unknown>;
};

export type GraphEdge = {
  id?: string;
  source: string;
  target: string;
  kind?: string;
  meta?: Record<string, unknown>;
};

export type GraphData = {
  nodes: GraphNode[];
  edges: GraphEdge[];
};

export type GraphOptions = {
  background?: string;
  nodeSize?: number;
  edgeOpacity?: number;
  orbitControls?: boolean;
  showGroups?: boolean;
  activeGroupId?: string | null;
  onNodeClick?: (node: GraphNode) => void;
  onNodeHover?: (node: GraphNode | null) => void;
  onEdgeHover?: (edge: GraphEdge | null) => void;
  onEdgeClick?: (edge: GraphEdge) => void;
  onSelectionChange?: (selection: {
    selectedNodeId?: string;
    selectedEdgeId?: string;
    selectedNodeIds: string[];
  }) => void;
  onPointerMove?: (coords: { x: number; y: number }) => void;
  onViewportChange?: (viewport: {
    width: number;
    height: number;
    cameraDistance?: number;
  }) => void;
  /**
   * Fired when the user completes a box (drag) selection.
   * Typically initiated via Shift + drag.
   */
  onNodesSelected?: (nodeIds: string[]) => void;
};

export type GraphHandle = {
  update: (data: GraphData) => void;
  setOptions: (options: Partial<GraphOptions>) => void;
  focusNode: (nodeId: string) => void;
  focusGroup: (groupId: string) => void;
  fitToNodes: (nodeIds: string[]) => void;
  fitToGroup: (groupId: string) => void;
  fitToData: () => void;
  destroy: () => void;
};
