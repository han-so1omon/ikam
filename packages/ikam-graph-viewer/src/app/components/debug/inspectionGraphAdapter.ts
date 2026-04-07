import type { InspectionSubgraphResponse } from '../../api/client';
import { getLayoutedElements } from '../graphFlowLayout';

type SourceNode = Record<string, unknown>;
type SourceEdge = Record<string, unknown>;

export type InspectionGraphNode = {
  id: string;
  type: 'default';
  className?: string;
  position: { x: number; y: number };
  targetPosition?: string;
  sourcePosition?: string;
  data: {
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
    raw: SourceNode;
  };
};

export type InspectionGraphEdge = {
  id: string;
  source: string;
  target: string;
  label?: string;
  data: {
    relation: string | null;
  };
};

const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === 'object' ? (value as Record<string, unknown>) : {};

const asString = (value: unknown): string | null =>
  typeof value === 'string' && value.trim().length > 0 ? value : null;

const asRecordArray = (value: unknown): Record<string, unknown>[] =>
  Array.isArray(value) ? value.filter((item): item is Record<string, unknown> => Boolean(item) && typeof item === 'object') : [];

const titleCase = (value: string): string => value.replace(/_/g, ' ').replace(/\b\w/g, (char) => char.toUpperCase());

const compactFileLabel = (value: string): string => {
  if (value.length <= 10) {
    return value;
  }
  return `${value.slice(0, 7)}...`;
};

const compactChunkLabel = (value: string): string => {
  const hashIndex = value.lastIndexOf('#');
  if (hashIndex >= 0 && hashIndex < value.length - 1) {
    return value.slice(hashIndex + 1);
  }
  return value.length <= 24 ? value : `${value.slice(0, 21)}...`;
};

const extensionFromName = (value: string | null): string | null => {
  if (!value) return null;
  const clean = value.split('#')[0] ?? value;
  const lastDot = clean.lastIndexOf('.');
  if (lastDot < 0 || lastDot === clean.length - 1) return null;
  const ext = clean.slice(lastDot + 1).replace(/[^a-z0-9]+/gi, '').toUpperCase();
  return ext.length >= 2 && ext.length <= 4 ? ext : null;
};

const mimeShorthand = (value: string | null): string | null => {
  if (!value) return null;
  const lower = value.toLowerCase();
  if (lower.includes('markdown')) return 'MD';
  if (lower.includes('json')) return 'JSON';
  if (lower.includes('pdf')) return 'PDF';
  if (lower.includes('png')) return 'PNG';
  if (lower.includes('jpeg') || lower.includes('jpg')) return 'JPG';
  if (lower.includes('gif')) return 'GIF';
  if (lower.includes('svg')) return 'SVG';
  if (lower.includes('spreadsheet') || lower.includes('excel') || lower.includes('sheet')) return 'XLS';
  if (lower.includes('presentation') || lower.includes('powerpoint')) return 'PPT';
  if (lower.includes('wordprocessingml') || lower.includes('docx')) return 'DOCX';
  if (lower.startsWith('text/')) return 'TXT';
  if (lower.startsWith('image/')) return 'IMG';
  return null;
};

const fallbackKindShorthand = (kind: string): string => {
  const letters = kind.replace(/[^a-z0-9]+/gi, ' ').trim().split(/\s+/).filter(Boolean);
  if (letters.length === 0) return 'FR';
  if (letters.length === 1) return letters[0]!.slice(0, 4).toUpperCase();
  return letters.map((part) => part[0]!.toUpperCase()).join('').slice(0, 4);
};

const kindClassName = (value: string): string => `inspection-graph-node-kind-${value.replace(/[^a-z0-9]+/gi, '-').replace(/^-+|-+$/g, '').toLowerCase() || 'node'}`;

const toneForKind = (value: string): { backgroundColor: string; borderColor: string; color: string } => {
  if (value === 'chunk_extraction_set') {
    return { backgroundColor: 'rgba(201, 143, 59, 0.14)', borderColor: 'rgba(201, 143, 59, 0.34)', color: '#6f4d1e' };
  }
  if (value === 'document_set') {
    return { backgroundColor: 'rgba(52, 121, 214, 0.12)', borderColor: 'rgba(52, 121, 214, 0.34)', color: '#174f97' };
  }
  if (value === 'document_chunk_set') {
    return { backgroundColor: 'rgba(47, 143, 111, 0.12)', borderColor: 'rgba(47, 143, 111, 0.34)', color: '#245f4b' };
  }
  return { backgroundColor: 'rgba(255, 255, 255, 0.78)', borderColor: 'rgba(110, 142, 176, 0.22)', color: '#173048' };
};

const subgraphRefFromNode = (node: SourceNode): string | null => {
  const refs = asRecord(node.refs);
  const selfRef = asRecord(refs.self);
  const locator = asRecord(selfRef.locator);
  return asString(locator.subgraph_ref) ?? asString(node.label);
};

const inferNodeKind = (node: SourceNode): string => {
  const payload = asRecord(node.payload);
  const value = asRecord(payload.value);
  const directKind = asString(value.kind) ?? asString(payload.kind) ?? asString(node.ir_kind);
  if (directKind) return directKind;

  const subgraphRef = subgraphRefFromNode(node);
  if (subgraphRef) {
    if (subgraphRef.includes('/document_set/')) return 'document_set';
    if (subgraphRef.includes('/chunk_extraction_set/')) return 'chunk_extraction_set';
    if (subgraphRef.includes('/document_chunk_set/')) return 'document_chunk_set';
  }

  return asString(node.kind) ?? 'node';
};

const readableNodeLabel = (node: SourceNode): string => {
  const payload = asRecord(node.payload);
  const value = asRecord(payload.value);
  const kind = inferNodeKind(node);

  if (kind === 'document_set') return 'Document set';
  if (kind === 'chunk_extraction_set') return 'Chunk extraction set';
  if (kind === 'document_chunk_set') return 'Document chunk set';
  if (kind === 'document') {
    const fileLabel = asString(value.filename)
      ?? asString(value.file_name)
      ?? asString(value.document_id)
      ?? asString(node.label)
      ?? 'Document';
    return compactFileLabel(fileLabel);
  }
  if (kind === 'chunk' || kind === 'chunk_extraction') {
    const chunkLabel = asString(value.chunk_id)
      ?? asString(value.document_id)
      ?? asString(value.filename)
      ?? asString(value.file_name)
      ?? asString(node.label)
      ?? 'Chunk';
    return compactChunkLabel(chunkLabel);
  }

  return asString(node.label) ?? (kind ? titleCase(kind) : 'Node');
};

const secondaryNodeLabel = (node: SourceNode, primaryLabel: string): string | null => {
  const payload = asRecord(node.payload);
  const value = asRecord(payload.value);
  const kind = inferNodeKind(node);
  if (kind === 'document') {
    const fullLabel = asString(value.filename) ?? asString(value.file_name) ?? asString(value.document_id) ?? asString(node.label);
    return fullLabel && fullLabel !== primaryLabel ? fullLabel : null;
  }
  if (kind === 'chunk' || kind === 'chunk_extraction') {
    const fullLabel = asString(value.chunk_id)
      ?? asString(value.document_id)
      ?? asString(value.filename)
      ?? asString(value.file_name)
      ?? asString(node.label);
    return fullLabel && fullLabel !== primaryLabel ? fullLabel : null;
  }
  const rawLabel = asString(node.label);
  const subgraphRef = subgraphRefFromNode(node);
  const candidate = rawLabel?.startsWith('hot://') ? rawLabel : subgraphRef?.startsWith('hot://') ? subgraphRef : null;
  return candidate && candidate !== primaryLabel ? candidate : null;
};

const iconForKind = (kind: string): string => {
  return fallbackKindShorthand(kind);
};

const nodeBadge = (node: SourceNode, kind: string): string => {
  const payload = asRecord(node.payload);
  const value = asRecord(payload.value);
  if (kind === 'document_set') return 'DOCS';
  if (kind === 'chunk_extraction_set') return 'CHKS';
  if (kind === 'document_chunk_set') return 'DCHS';
  if (kind === 'chunk' || kind === 'chunk_extraction') return 'CHK';
  if (kind === 'document') {
    const fileName = asString(value.filename) ?? asString(value.file_name) ?? asString(node.label);
    return extensionFromName(fileName)
      ?? mimeShorthand(asString(payload.mime_type) ?? asString(value.mime_type))
      ?? 'DOC';
  }
  return iconForKind(kind);
};

export function adaptInspectionSubgraph(response: InspectionSubgraphResponse): {
  nodes: InspectionGraphNode[];
  edges: InspectionGraphEdge[];
} {
  const rootNodeId = asString(response.root_node_id);
  const sourceNodes = asRecordArray(response.nodes).sort((left, right) => {
    const leftId = asString(left.id) ?? '';
    const rightId = asString(right.id) ?? '';
    return leftId.localeCompare(rightId);
  });
  const sourceEdges = asRecordArray(response.edges)
    .filter((edge) => {
      const source = asString(edge.from);
      const target = asString(edge.to);
      return Boolean(source && target);
    })
    .sort((left, right) => {
      const leftId = asString(left.id) ?? `${asString(left.from) ?? ''}:${asString(left.to) ?? ''}`;
      const rightId = asString(right.id) ?? `${asString(right.from) ?? ''}:${asString(right.to) ?? ''}`;
      return leftId.localeCompare(rightId);
    });

  const baseNodes: InspectionGraphNode[] = sourceNodes.map((node) => {
    const id = asString(node.id) ?? 'node';
    const isRoot = rootNodeId === id;
    const label = readableNodeLabel(node);
    const kind = inferNodeKind(node);
    const compact = kind === 'document' || kind === 'chunk' || kind === 'chunk_extraction';
    return {
      id,
      type: 'default',
      className: `${isRoot ? 'inspection-graph-node inspection-graph-node-root' : 'inspection-graph-node'} ${kindClassName(kind)}`,
      position: { x: 0, y: 0 },
      data: {
        label,
        secondaryLabel: secondaryNodeLabel(node, label),
        icon: nodeBadge(node, kind),
        compact,
        kind,
        irKind: asString(node.ir_kind),
        kindClassName: kindClassName(kind),
        tone: toneForKind(kind),
        isRoot,
        raw: node,
      },
    };
  });

  const edges: InspectionGraphEdge[] = sourceEdges.map((edge) => ({
    id: asString(edge.id) ?? `${asString(edge.from) ?? 'source'}:${asString(edge.to) ?? 'target'}:${asString(edge.relation) ?? 'edge'}`,
    source: asString(edge.from) ?? 'source',
    target: asString(edge.to) ?? 'target',
    label: asString(edge.label) ?? asString(edge.relation) ?? undefined,
    data: {
      relation: asString(edge.relation),
    },
  }));

  return getLayoutedElements(baseNodes, edges, {
    direction: 'LR',
    getNodeSize: () => ({ width: 180, height: 56 }),
    nodesep: 30,
    ranksep: 60,
  });
}
