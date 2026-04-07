import type { GraphData } from '../types';
import { toInspectableEdge, toInspectableNode } from './adapters';
import type { SelectionDetails } from './types';

export const deriveSelectionDetails = (graph: GraphData, selection: {
  selectedNodeId?: string;
  selectedEdgeId?: string;
  selectedNodeIds?: string[];
}): SelectionDetails => {
  const selectedNode = selection.selectedNodeId
    ? graph.nodes.find((node) => node.id === selection.selectedNodeId) ?? null
    : null;
  const selectedEdge = selection.selectedEdgeId
    ? graph.edges.find((edge) => edge.id === selection.selectedEdgeId) ?? null
    : null;
  return {
    selectedNode: selectedNode ? toInspectableNode(selectedNode) : null,
    selectedEdge: selectedEdge ? toInspectableEdge(selectedEdge) : null,
    selectedNodeIds: selection.selectedNodeIds ?? [],
  };
};
