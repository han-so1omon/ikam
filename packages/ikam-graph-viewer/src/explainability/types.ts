export type ProvenanceRef = {
  origin: 'map' | 'semantic' | 'merge' | 'unknown';
  runId?: string;
  decisionRef?: string;
  caseId?: string;
};

export type SemanticLinkRef = {
  entityIds: string[];
  relationIds: string[];
};

export type InspectableNode = {
  id: string;
  type: string;
  label: string;
  level?: number;
  salience?: number;
  artifactIds: string[];
  fragmentIds: string[];
  provenance: ProvenanceRef;
  semanticLinks: SemanticLinkRef;
  meta: Record<string, unknown>;
};

export type InspectableEdge = {
  id?: string;
  source: string;
  target: string;
  kind: string;
  provenance: ProvenanceRef;
  semanticLinks: SemanticLinkRef;
  meta: Record<string, unknown>;
};

export type SelectionDetails = {
  selectedNode?: InspectableNode | null;
  selectedEdge?: InspectableEdge | null;
  selectedNodeIds: string[];
};
