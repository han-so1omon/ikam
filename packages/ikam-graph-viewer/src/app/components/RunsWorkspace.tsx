import { Fragment, useEffect, useMemo, useRef, useState } from 'react';

import {
  controlRun,
  getArtifactPreview,
  getDebugStepDetail,
  getDebugStream,
  getEnvironmentSummary,
  getInspectionSubgraph,
  getRegistry,
  getRegistryNamespaces,
  getScopedFragments,
  getSubgraph,
  type ArtifactPreviewResponse,
  type CaseMeta,
  type DebugStepEvent,
  type InspectionSubgraphResponse,
  type RunEntry,
  type RunEvaluation,
} from '../api/client';
import CaseSelector from './CaseSelector';
import EmbeddingShapePanel from './debug/EmbeddingShapePanel';
import InspectionGraphPanel, { type InspectionGraphNodeSelection } from './debug/InspectionGraphPanel';
import RunFragmentGraphPanel, { type RunFragmentGraphNodeSelection } from './debug/RunFragmentGraphPanel';
import TransformationFlowGraph, { type TransformationFlowEdge, type TransformationFlowNode } from './debug/TransformationFlowGraph';
import ContentViewer from './content-viewer/ContentViewer';
import ContentDiffViewer from './content-viewer/ContentDiffViewer';
import PetriNetViewer from './PetriNetViewer';
import { describeMime } from './content-viewer/mimeDescription';
import RunTable from './RunTable';

type RunsWorkspaceProps = {
  cases: CaseMeta[];
  loadingCases: boolean;
  caseError: string | null;
  selectedCaseIds: string[];
  onToggleCase: (caseId: string) => void;
  reset: boolean;
  onResetChange: (value: boolean) => void;
  running: boolean;
  runError: string | null;
  onRunCases: (pipelineId?: string) => void;
  runs: RunEntry[];
  activeRunId: string | null;
  onSelectRun: (runId: string) => void;
};

const EvaluationPanel = ({ evaluation }: { evaluation: RunEvaluation }) => (
  <div className="evaluation-grid">
    <div className="evaluation-metrics">
      <div className="metric">
        <span className="metric-label">Entities</span>
        <span className="metric-value">{(evaluation.report.entities.coverage * 100).toFixed(1)}%</span>
      </div>
      <div className="metric">
        <span className="metric-label">Predicates</span>
        <span className="metric-value">{(evaluation.report.predicates.predicate_coverage * 100).toFixed(1)}%</span>
      </div>
      <div className="metric">
        <span className="metric-label">Exploration</span>
        <span className="metric-value">{(evaluation.report.exploration.mean_recall * 100).toFixed(1)}%</span>
      </div>
      <div className="metric">
        <span className="metric-label">Query Quality</span>
        <span className="metric-value">{evaluation.report.query.mean_quality_score.toFixed(1)}/10</span>
      </div>
      <div className="metric">
        <span className="metric-label">Dedup Ratio</span>
        <span className="metric-value">{(evaluation.report.compression.dedup_ratio * 100).toFixed(1)}%</span>
      </div>
      <div className="metric">
        <span className="metric-label">Overall</span>
        <span className={`metric-value ${evaluation.report.passed ? 'metric-value-accent' : ''}`}>
          {evaluation.report.passed ? 'Pass' : 'Review'}
        </span>
      </div>
    </div>
    <pre className="evaluation-report">{evaluation.rendered}</pre>
    <div className="evaluation-debug" data-testid="evaluation-debug">
      <h4>Evaluation Debug</h4>
      <p className="panel-subtitle">
        Steps: {(evaluation.details?.pipeline_steps ?? []).join(' -> ') || 'compression -> entities -> predicates -> exploration -> query'}
      </p>
      <div className="evaluation-debug-grid">
        <section>
          <h5>Entity Matches</h5>
          <ul>
            {(evaluation.details?.entities ?? []).map((item) => (
              <li key={item.expected_name}>
                {item.expected_name}: {item.found ? 'found' : 'missing'}
                {item.matched_label ? ` (${item.matched_label})` : ''}
              </li>
            ))}
          </ul>
        </section>
        <section>
          <h5>Predicate Coverage</h5>
          <ul>
            {(evaluation.details?.predicates ?? []).map((item) => (
              <li key={item.label}>
                {item.label}: {(item.chain_coverage * 100).toFixed(0)}%
              </li>
            ))}
          </ul>
        </section>
        <section>
          <h5>Exploration Queries</h5>
          <ul>
            {(evaluation.details?.exploration_queries ?? []).map((item) => (
              <li key={item.query}>
                {item.query} - recall {(item.relevance_score * 100).toFixed(0)}%, fragments {item.fragments_retrieved}
              </li>
            ))}
          </ul>
        </section>
        <section>
          <h5>Query Outcomes</h5>
          <ul>
            {(evaluation.details?.query_results ?? []).map((item) => (
              <li key={item.query}>
                {item.query} - facts {(item.fact_coverage * 100).toFixed(0)}%, quality {item.quality_score.toFixed(1)}/10; evidence {(item.evidence_fragment_ids ?? []).length}; answer {item.answer_text ? item.answer_text : 'n/a'}
              </li>
            ))}
          </ul>
        </section>
      </div>
    </div>
  </div>
);

const asNumber = (value: unknown): number | null => (typeof value === 'number' ? value : null);
const asString = (value: unknown): string | null => (typeof value === 'string' ? value : null);
const asStringArray = (value: unknown): string[] =>
  Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : [];

type DetailNode = {
  node_id: string;
  kind: string;
  fragment_id?: string | null;
  cas_id?: string | null;
  mime_type?: string | null;
  label?: string;
  meta?: Record<string, unknown>;
};

type LiftTransformation = {
  surfaceFragmentId: string;
  sourceArtifactId: string | null;
  irFragmentIds: string[];
  liftStatus: 'lifted' | 'surface_only';
  liftReason: string | null;
};

type DetailEdge = {
  from: string;
  to: string;
  relation: string;
};

type StepTrace = {
  workflowId: string | null;
  requestId: string | null;
  executorId: string | null;
  executorKind: string | null;
  transitionId: string | null;
  markingBeforeRef: string | null;
  markingAfterRef: string | null;
  enabledTransitionIds: string[];
  topicSequence: Array<{ topic: string; eventType: string; status: string }>;
  traceId: string | null;
  traceFragmentId: string | null;
  timeline: Array<Record<string, unknown>>;
  rawEvents: Array<Record<string, unknown>>;
};

type TransitionValidationResult = {
  name: string;
  direction: string | null;
  kind: string | null;
  status: 'passed' | 'failed' | 'not_ready';
  matchedFragmentIds: string[];
};

type InspectionPayload = {
  valueKind: string | null;
  summary: string | null;
  refs: string[];
  content: Record<string, unknown>;
  resolvedRefs: Array<Record<string, unknown>>;
  subgraph: InspectionSubgraphResponse | null;
};

type FragmentListItem = {
  id: string;
  display: string;
  search: string;
  inspection: InspectionPayload | null;
  inspectionRef: string | null;
};

type ConnectedFragmentItem = {
  id: string;
  label: string;
  secondaryLabel?: string;
  ref: string;
  itemId?: string;
  inspection?: InspectionPayload | null;
  inspectionRef?: string | null;
  section?: DrillSection;
  isPrevious?: boolean;
};

type DrillSection = 'input' | 'output';

type ExplorerSelection = {
  section: DrillSection;
  itemId: string;
  label: string;
  inspection: InspectionPayload | null;
  inspectionRef: string | null;
  artifactPreview?: ArtifactPreviewResponse | null;
  loading: boolean;
  showRaw?: boolean;
  ancestry?: ConnectedFragmentItem[];
};

type EnvironmentSummaryState = {
  fragment_count?: number;
  verification_count?: number;
  reconstruction_program_count?: number;
  ref?: string;
  env_type?: string;
  env_id?: string;
  executors_seen?: string[];
};

const getScopeRef = (payload: { ref?: string | null; env_type?: string | null; env_id?: string | null }): string | null => {
  if (payload.ref) {
    return payload.ref;
  }
  if (payload.env_type && payload.env_id) {
    return `${payload.env_type}:${payload.env_id}`;
  }
  return null;
};

const toPetriTransitionId = (stepName: string | null | undefined): string | null => {
  if (!stepName) {
    return null;
  }
  return `transition:${stepName.replace(/\./g, '-')}`;
};

type ScopedFragmentSummary = {
  id: string;
  mime_type?: string;
  meta?: Record<string, unknown>;
};

type TransformationFlowRef = {
  id: string;
  label: string;
  linkedNodeId: string | null;
};

type TransformationFlowStepDetail = {
  id: string;
  title: string;
  subtitle: string;
  summary: string;
  inputs: TransformationFlowRef[];
  outputs: TransformationFlowRef[];
};

type LogBundle = {
  stdout: string[];
  stderr: string[];
};

type OptimisticStep = {
  debugKey: string;
  stepId: string;
  stepName: string;
  attemptIndex: number;
  envType: DebugStepEvent['env_type'];
  envId: string;
};

const buildPreviewFromNode = (node: DetailNode | null): ArtifactPreviewResponse | null => {
  if (!node) return null;
  const meta = node.meta as Record<string, unknown> | undefined;
  const rawPreview = meta ? (meta.value_preview as unknown) : null;
  if (rawPreview == null) return null;

  const fileName =
    (meta && (asString(meta.file_name) ?? asString(meta.filename)))
    ?? node.label
    ?? node.fragment_id
    ?? node.node_id;
  const mimeType = node.mime_type ?? 'text/plain';

  if (typeof rawPreview === 'string') {
    return {
      kind: 'text',
      mime_type: mimeType,
      file_name: fileName,
      metadata: {},
      preview: { text: rawPreview },
    };
  }

  if (rawPreview && typeof rawPreview === 'object') {
    const text = asString((rawPreview as Record<string, unknown>).text);
    if (text) {
      return {
        kind: 'text',
        mime_type: mimeType,
        file_name: fileName,
        metadata: {},
        preview: { text },
      };
    }
    return {
      kind: 'json',
      mime_type: mimeType,
      file_name: fileName,
      metadata: {},
      preview: { parsed: rawPreview },
    };
  }

  return {
    kind: 'text',
    mime_type: mimeType,
    file_name: fileName,
    metadata: {},
    preview: { text: String(rawPreview) },
  };
};

const buildFallbackPreview = (fragmentId: string): ArtifactPreviewResponse => ({
  kind: 'text',
  mime_type: 'text/plain',
  file_name: fragmentId,
  metadata: {},
  preview: {
    text: `Preview unavailable for fragment ${fragmentId}.`,
  },
});

const clampPreviewText = (text: string, maxChars = 400): string => {
  if (text.length <= maxChars) return text;
  return `${text.slice(0, maxChars)}...`;
};

const joinOrFallback = (values: string[], fallback = 'n/a'): string => (values.length > 0 ? values.join(', ') : fallback);
const FRAGMENT_PREVIEW_COUNT = 6;

const basenameFromArtifactId = (value: string): string => {
  const tail = value.includes(':') ? value.slice(value.lastIndexOf(':') + 1) : value;
  const parts = tail.split('/').filter(Boolean);
  return parts[parts.length - 1] ?? tail;
};

const compactId = (value: string): string => (value.length <= 16 ? value : `${value.slice(0, 12)}...${value.slice(-4)}`);
const shortId = (value: string): string => (value.length <= 4 ? value : `${value.slice(0, 4)}...`);
const asFlowRef = (id: string, label: string, linkedNodeId: string | null): TransformationFlowRef => ({ id, label, linkedNodeId });
const emptyLogs = (): LogBundle => ({ stdout: [], stderr: [] });
const asRecordArray = (value: unknown): Array<Record<string, unknown>> =>
  Array.isArray(value) ? value.filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object')) : [];
const normalizeLogs = (value: unknown): LogBundle | null => {
  if (!value || typeof value !== 'object') return null;
  return {
    stdout: asStringArray((value as Record<string, unknown>).stdout_lines),
    stderr: asStringArray((value as Record<string, unknown>).stderr_lines),
  };
};
const hasLogs = (value: LogBundle | null): value is LogBundle => Boolean(value && (value.stdout.length > 0 || value.stderr.length > 0));
const parseInspection = (value: unknown): InspectionPayload | null => {
  if (!value || typeof value !== 'object') return null;
  const record = value as Record<string, unknown>;
  return {
    valueKind: asString(record.value_kind),
    summary: asString(record.summary),
    refs: asStringArray(record.refs),
    content: record.content && typeof record.content === 'object' ? record.content as Record<string, unknown> : {},
    resolvedRefs: asRecordArray(record.resolved_refs),
    subgraph: record.subgraph && typeof record.subgraph === 'object' ? record.subgraph as InspectionSubgraphResponse : null,
  };
};

const summarizeInspectionContent = (valueKind: string | null, content: Record<string, unknown>, refs: string[]): string | null => {
  if (valueKind === 'url') {
    const location = asString(content.location);
    return location ? `url ${location}` : 'url';
  }
  if (refs.length > 0 && valueKind) {
    return `${valueKind} ${refs.length} ${refs.length === 1 ? 'ref' : 'refs'}`;
  }
  if (valueKind === 'loaded_document') {
    const identifier = asString(content.document_id) ?? asString(content.filename) ?? asString(content.name);
    return identifier ? `loaded_document ${identifier}` : 'loaded_document';
  }
  return valueKind;
};

const inspectionArtifactId = (inspection: InspectionPayload | null): string | null => {
  if (!inspection) return null;
  const content = inspection.content;
  return asString(content.artifact_id)
    ?? asString(content.artifact_head_ref)
    ?? null;
};

const inspectionContentRefs = (content: Record<string, unknown>): string[] => {
  for (const key of ['document_refs', 'extraction_refs', 'entity_relationship_refs', 'claim_refs', 'refs']) {
    const values = asStringArray(content[key]);
    if (values.length > 0) return values;
  }
  return [];
};

const inspectionRefFromNode = (value: unknown): string | null => {
  if (!value || typeof value !== 'object') return null;
  const record = value as Record<string, unknown>;
  const backend = asString(record.backend);
  const locator = record.locator && typeof record.locator === 'object' ? record.locator as Record<string, unknown> : null;
  if (!locator) return null;
  const casId = asString(locator.cas_id);
  if (casId) return `inspect://fragment/${casId}`;
  const subgraphRef = asString(locator.subgraph_ref);
  if (subgraphRef) return `inspect://subgraph/${subgraphRef}`;
  const artifactId = asString(locator.artifact_id);
  if (artifactId) return `inspect://artifact/${artifactId}`;
  return backend ? null : null;
};

const parseInspectionResponse = (value: unknown): InspectionPayload | null => {
  const direct = parseInspection(value);
  if (!value || typeof value !== 'object') return direct;
  const record = value as Record<string, unknown>;
  const nodes = asRecordArray(record.nodes);
  const rootNodeId = asString(record.root_node_id);
  if (!rootNodeId || nodes.length === 0) return direct;
  const root = nodes.find((node) => asString(node.id) === rootNodeId);
  if (!root) return direct;
  const rootPayload = root.payload && typeof root.payload === 'object' ? root.payload as Record<string, unknown> : {};
  const rootKind = asString(root.ir_kind) ?? asString(rootPayload.kind) ?? asString(root.kind);
  const rootNodeKind = asString(root.kind);
  const content = rootNodeKind === 'fragment'
    ? (rootPayload.value && typeof rootPayload.value === 'object' ? rootPayload.value as Record<string, unknown> : rootPayload)
    : rootPayload;
  const edges = asRecordArray(record.edges);
  const connectedNodeIds = new Set(
    edges.flatMap((edge) => {
      const relation = asString(edge.relation);
      const from = asString(edge.from);
      const to = asString(edge.to);
      if (!relation || !from || !to) {
        return [];
      }
      if (from === rootNodeId && (relation === 'contains' || relation === 'derives' || relation === 'references')) {
        return [to];
      }
      if (to === rootNodeId && (relation === 'contains' || relation === 'derives' || relation === 'references')) {
        return [from];
      }
      return [];
    })
    .filter((edge): edge is string => Boolean(edge))
  );
  const connectedNodes = nodes.filter((node) => connectedNodeIds.has(asString(node.id) ?? ''));
  const resolvedRefs = connectedNodes.map((node) => {
    const payload = node.payload && typeof node.payload === 'object' ? node.payload as Record<string, unknown> : {};
      const fragmentId = asString(payload.cas_id) ?? asString(payload.id) ?? asString(node.id)?.replace(/^fragment:/, '') ?? asString(node.label) ?? 'fragment';
      const nodeKind = asString(node.kind);
      const nodeIrKind = asString(node.ir_kind);
      return {
        fragment_id: fragmentId,
        cas_id: fragmentId,
        mime_type: asString(payload.mime_type),
        name: nodeKind === 'subgraph' ? (nodeIrKind ?? asString(node.label) ?? connectedItemLabel(payload)) : (asString(node.label) ?? connectedItemLabel(payload)),
        inspection_ref: inspectionRefFromNode((node.refs as Record<string, unknown> | undefined)?.self) ?? `inspect://fragment/${fragmentId}`,
        value: payload.value,
      };
  });
  const refs = inspectionContentRefs(content);
  const fallbackRefs = resolvedRefs.map((item) => asString(item.fragment_id)).filter((item): item is string => Boolean(item));
  const mergedResolvedRefs = (() => {
    if (!direct) return resolvedRefs;
    if (resolvedRefs.length === 0) return direct.resolvedRefs;
    const merged = [...direct.resolvedRefs];
    const known = new Set(merged.map((item) => asString(item.fragment_id) ?? asString(item.cas_id) ?? asString(item.inspection_ref) ?? ''));
    for (const item of resolvedRefs) {
      const key = asString(item.fragment_id) ?? asString(item.cas_id) ?? asString(item.inspection_ref) ?? '';
      if (!known.has(key)) {
        merged.push(item);
        known.add(key);
      }
    }
    return merged;
  })();
  const mergedRefs = (() => {
    const combined = [...(direct?.refs ?? []), ...(refs.length > 0 ? refs : fallbackRefs)];
    return Array.from(new Set(combined.filter((item): item is string => Boolean(item))));
  })();
  const subgraph: InspectionSubgraphResponse = {
    schema_version: 'v1',
    root_node_id: rootNodeId,
    navigation: record.navigation,
    nodes,
    edges,
  };
  return {
    valueKind: direct?.valueKind ?? rootKind,
    summary: direct?.summary ?? asString(root.summary) ?? summarizeInspectionContent(rootKind, content, refs.length > 0 ? refs : fallbackRefs),
    refs: mergedRefs,
    content: Object.keys(direct?.content ?? {}).length > 0 ? (direct?.content ?? {}) : content,
    resolvedRefs: mergedResolvedRefs,
    subgraph,
  };
};
const parseInspectionRef = (value: unknown): string | null => {
  if (!value || typeof value !== 'object') return null;
  return asString((value as Record<string, unknown>).inspection_ref);
};

const connectedItemLabel = (value: Record<string, unknown>): string => {
  const chunkValue = value.value && typeof value.value === 'object' ? value.value as Record<string, unknown> : null;
  const candidateValue = chunkValue ?? value;
  if (asString(candidateValue.kind) === 'document_chunk_set') {
    return 'document_chunk_set';
  }
  const isChunk = Boolean(
    asString(value.mime_type)?.toLowerCase().includes('chunk')
    || asString(candidateValue.span)
    || (candidateValue && typeof candidateValue === 'object' && 'span' in candidateValue && 'text' in candidateValue)
  );
  if (isChunk) {
    return asString(candidateValue.chunk_id)
      ?? asString(candidateValue.document_id)
      ?? asString(candidateValue.filename)
      ?? asString(candidateValue.file_name)
      ?? asString(value.fragment_id)
      ?? asString(value.cas_id)
      ?? 'fragment';
  }
  return asString(value.name)
    ?? asString(value.filename)
    ?? asString(value.file_name)
    ?? asString(value.document_id)
    ?? asString(value.fragment_id)
    ?? asString(value.cas_id)
    ?? 'fragment';
};

const inspectionFieldsForEntry = (entry: ExplorerSelection): Array<{ label: string; value: unknown }> => {
  const content = entry.inspection?.content ?? {};
  const fields: Array<{ label: string; value: unknown }> = [];
  const sourceArtifact = asString(content.artifact_id) ?? asString(content.artifact_head_ref);
  const originalDocument = asString(content.filename) ?? asString(content.file_name) ?? asString(content.document_id);
  if (sourceArtifact) fields.push({ label: 'Source artifact', value: sourceArtifact });
  if (originalDocument) fields.push({ label: 'Original document', value: originalDocument });
  return fields;
};

const connectedItemRef = (value: Record<string, unknown>): string | null => {
  return asString(value.inspection_ref)
    ?? asString(value.fragment_id)
    ?? asString(value.cas_id);
};

const selectionInspectionRef = (selection: ExplorerSelection): string => {
  if (selection.inspectionRef) {
    return selection.inspectionRef;
  }
  const subgraphRef = asString(selection.inspection?.content.subgraph_ref);
  if (subgraphRef) {
    return `inspect://subgraph/${subgraphRef}`;
  }
  return selection.itemId;
};
type TerminalLogLine = {
  id: string;
  text: string;
  stream: 'stdout' | 'stderr';
};

type StructuredLogEvent = {
  seq: number;
  at: string;
  source: 'executor' | 'system';
  stream: 'stdout' | 'stderr';
  message: string;
};

const normalizeLogEvents = (value: unknown): StructuredLogEvent[] => {
  if (!Array.isArray(value)) return [];
  return value
    .filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
    .map((item) => ({
      seq: typeof item.seq === 'number' ? item.seq : Number.NaN,
      at: asString(item.at) ?? '',
      source: item.source === 'system' ? 'system' : item.source === 'executor' ? 'executor' : 'executor',
      stream: item.stream === 'stderr' ? 'stderr' : 'stdout',
      message: asString(item.message) ?? '',
    }))
    .filter((item) => Number.isFinite(item.seq) && item.at && item.message);
};

const stripLeadingTimestamp = (value: string): string => value.replace(/^\[[^\]]+\]\s*/, '');

const buildTerminalLines = (
  bundle: { executor: LogBundle | null; system: LogBundle | null } | null,
  legacy: LogBundle,
  logEvents: StructuredLogEvent[]
): TerminalLogLine[] => {
  if (logEvents.length > 0) {
    return logEvents
      .slice()
      .sort((left, right) => left.seq - right.seq)
      .map((event) => ({
        id: `event-${event.seq}`,
        text: `[${event.at}] [${event.source === 'executor' ? 'exec' : 'sys'}] ${stripLeadingTimestamp(event.message)}`,
        stream: event.stream,
      }));
  }

  const lines: TerminalLogLine[] = [];
  const pushLines = (prefix: string | null, logBundle: LogBundle | null) => {
    if (!logBundle) return;
    logBundle.stdout.forEach((text, index) => {
      lines.push({
        id: `${prefix ?? 'legacy'}-stdout-${index}`,
        text: prefix ? `[${prefix}] ${text}` : text,
        stream: 'stdout',
      });
    });
    logBundle.stderr.forEach((text, index) => {
      lines.push({
        id: `${prefix ?? 'legacy'}-stderr-${index}`,
        text: prefix ? `[${prefix}] ${text}` : text,
        stream: 'stderr',
      });
    });
  };

  if (bundle && (hasLogs(bundle.executor) || hasLogs(bundle.system))) {
    pushLines('exec', bundle.executor);
    pushLines('sys', bundle.system);
    return lines;
  }

  pushLines(null, legacy);
  return lines;
};

const formatInspectionText = (inspection: InspectionPayload | null): string | null => {
  if (!inspection) return null;
  return JSON.stringify(
    inspection.resolvedRefs.length > 0
      ? { ...inspection.content, resolved_refs: inspection.resolvedRefs }
      : inspection.content,
    null,
    2
  );
};

const formatInspectionValue = (value: unknown): string => {
  if (value == null) return 'n/a';
  if (Array.isArray(value)) {
    return value.every((item) => item == null || ['string', 'number', 'boolean'].includes(typeof item))
      ? value.map((item) => String(item)).join(', ')
      : `[${value.length} items]`;
  }
  if (typeof value === 'object') return '[object]';
  return String(value);
};

const renderInspectionFields = (fields: Array<{ label: string; value: unknown }>) => (
  <dl className="runs-fragment-drill-fields">
    {fields.map((field) => (
      <Fragment key={field.label}>
        <dt>{field.label}</dt>
        <dd>{formatInspectionValue(field.value)}</dd>
      </Fragment>
    ))}
  </dl>
);

const asInspectionSubgraph = (inspection: InspectionPayload | null) => inspection?.subgraph ?? null;

const buildInspectionPreview = (inspection: InspectionPayload | null): ArtifactPreviewResponse | null => {
  if (!inspection) return null;
  const mimeTypeByKind: Record<string, string> = {
    claim: 'application/vnd.ikam.claim-ir+json',
    proposition: 'application/vnd.ikam.proposition-ir+json',
    structured_data: 'application/json',
    edge: 'application/json',
  };
  const kind = inspection.valueKind ?? 'value';
  return {
    kind: 'json',
    mime_type: mimeTypeByKind[kind] ?? 'application/json',
    file_name: `${kind}.json`,
    metadata: {},
    preview: {
      parsed: inspection.content,
    },
  };
};

const sparkline = (values: number[]): string => {
  if (values.length === 0) return '';
  const blocks = '▁▂▃▄▅▆▇█';
  const max = Math.max(...values, 1);
  return values
    .slice(0, 24)
    .map((value) => blocks[Math.min(blocks.length - 1, Math.floor((value / max) * (blocks.length - 1)))])
    .join('');
};

const TRANSITIONS = {
  INIT: 'init.initialize',
  MAP: 'map.conceptual.lift.surface_fragments',
  EMBED_MAPPED: 'map.conceptual.embed.discovery_index',
  LIFT: 'map.conceptual.normalize.discovery',
  EMBED_LIFTED: 'map.reconstructable.embed',
  CANDIDATE_SEARCH: 'map.reconstructable.search.dependency_resolution',
  NORMALIZE: 'map.reconstructable.normalize',
  COMPOSE: 'map.reconstructable.compose.reconstruction_programs',
  VERIFY: 'map.conceptual.verify.discovery_gate',
  PROMOTE: 'map.conceptual.commit.semantic_only',
  PROJECT: 'map.reconstructable.build_subgraph.reconstruction',
} as const;

const summarizeStepReason = (stepName: string, payload: Record<string, unknown>): string | null => {
  switch (stepName) {
    case TRANSITIONS.MAP: {
      const fragments = Array.isArray(payload.fragments) ? payload.fragments.length : null;
      const canonical = asString(payload.mime_type);
      if (fragments != null) {
        return `Mapped source content into ${fragments} fragment(s)${canonical ? ` using ${canonical}` : ''}.`;
      }
      return null;
    }
    case TRANSITIONS.EMBED_MAPPED: {
      const count = asNumber(payload.embedding_count);
      const dims = asNumber(payload.dimensions);
      const clusters = asNumber(payload.cluster_count);
      if (count != null) {
        return `Embedded ${count} mapped fragment(s)${dims != null ? ` at ${dims} dimensions` : ''}${clusters != null ? ` and formed ${clusters} cluster(s)` : ''}.`;
      }
      return null;
    }
    case TRANSITIONS.LIFT: {
      const fragments = Array.isArray(payload.fragments) ? payload.fragments.length : null;
      if (fragments != null) {
        return `Lifted canonical fragments into ${fragments} IR fragment(s) for structural reasoning.`;
      }
      return null;
    }
    case TRANSITIONS.EMBED_LIFTED: {
      const count = asNumber(payload.embedding_count);
      const dims = asNumber(payload.dimensions);
      if (count != null) {
        return `Embedded lifted IR fragments (${count})${dims != null ? ` at ${dims} dimensions` : ''} for candidate matching.`;
      }
      return null;
    }
    case TRANSITIONS.CANDIDATE_SEARCH: {
      const pairs = Array.isArray(payload.candidates) ? payload.candidates.length : null;
      if (pairs != null) {
        return `Scored pairwise candidate matches across embedded fragments and produced ${pairs} candidate pair(s).`;
      }
      return null;
    }
    case TRANSITIONS.NORMALIZE: {
      const norm = asNumber(payload.normalized_fragment_count);
      const recon = asNumber(payload.reconstruction_program_count);
      if (norm != null || recon != null) {
        return `Normalized ${norm ?? 0} fragment(s) and generated ${recon ?? 0} reconstruction program(s).`;
      }
      return null;
    }
    case TRANSITIONS.COMPOSE: {
      const proposal = payload.proposal;
      if (proposal && typeof proposal === 'object') {
        const mode = asString((proposal as Record<string, unknown>).commit_mode);
        const count = asNumber((proposal as Record<string, unknown>).fragment_count);
        return `Composed commit proposal${mode ? ` in ${mode} mode` : ''}${count != null ? ` for ${count} fragment(s)` : ''}.`;
      }
      return null;
    }
    case TRANSITIONS.VERIFY: {
      const verification = payload.verification;
      if (verification && typeof verification === 'object') {
        const passed = (verification as Record<string, unknown>).passed;
        if (typeof passed === 'boolean') {
          return passed
            ? 'Verification checks passed; proposal is internally consistent for this attempt.'
            : 'Verification checks failed; pipeline paused for retry or intervention.';
        }
      }
      return null;
    }
    case TRANSITIONS.PROMOTE: {
      const commit = payload.commit;
      if (commit && typeof commit === 'object') {
        const mode = asString((commit as Record<string, unknown>).mode);
        return `Promoted staged changes into ${mode ?? 'commit'} scope for graph projection.`;
      }
      return null;
    }
    case TRANSITIONS.PROJECT: {
      const graph = payload.graph;
      if (graph && typeof graph === 'object') {
        const nodeCount = asNumber((graph as Record<string, unknown>).node_count);
        const edgeCount = asNumber((graph as Record<string, unknown>).edge_count);
        if (nodeCount != null || edgeCount != null) {
          return `Projected promoted artifacts into graph view (${nodeCount ?? 0} node(s), ${edgeCount ?? 0} edge(s)).`;
        }
      }
      return null;
    }
    default:
      return null;
  }
};

const RunsWorkspace = ({
  cases,
  loadingCases,
  caseError,
  selectedCaseIds,
  onToggleCase,
  reset,
  onResetChange,
  running,
  runError,
  onRunCases,
  runs,
  activeRunId,
  onSelectRun,
}: RunsWorkspaceProps) => {
  const LOG_DOCK_MIN_HEIGHT = 180;
  const LOG_DOCK_DEFAULT_HEIGHT = 280;
  const LOG_DOCK_MAX_HEIGHT = 560;
  const activeRun = runs.find((r) => r.run_id === activeRunId) ?? null;
  const [debugEvents, setDebugEvents] = useState<DebugStepEvent[]>([]);
  const [loadedDebugKey, setLoadedDebugKey] = useState<string | null>(null);
  const [pipelineSteps, setPipelineSteps] = useState<string[]>([]);
  const [debugStatus, setDebugStatus] = useState<'idle' | 'loading' | 'ready' | 'missing' | 'error'>('idle');
  const [selectedStepId, setSelectedStepId] = useState<string | null>(null);
  const [stepDetailPayload, setStepDetailPayload] = useState<Record<string, unknown> | null>(null);
  const [executionMode, setExecutionMode] = useState<'autonomous' | 'manual'>('autonomous');
  const [executionState, setExecutionState] = useState<string>('idle');
  const [controlInFlight, setControlInFlight] = useState(false);
  const [expandedNodeIds, setExpandedNodeIds] = useState<Record<string, boolean>>({});
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [fragmentSearch, setFragmentSearch] = useState('');
  const [isDrillThroughCollapsed, setIsDrillThroughCollapsed] = useState(false);
  const [isLogDockCollapsed, setIsLogDockCollapsed] = useState(false);
  const [logDockHeight, setLogDockHeight] = useState(LOG_DOCK_DEFAULT_HEIGHT);
  const [stepDetailError, setStepDetailError] = useState<string | null>(null);
  const [artifactPreview, setArtifactPreview] = useState<ArtifactPreviewResponse | null>(null);
  const [environmentSummary, setEnvironmentSummary] = useState<EnvironmentSummaryState | null>(null);
  const [scopedFragments, setScopedFragments] = useState<ScopedFragmentSummary[]>([]);
  const [showFullMapSummary, setShowFullMapSummary] = useState(false);
  const [inputFragmentSearch, setInputFragmentSearch] = useState('');
  const [outputFragmentSearch, setOutputFragmentSearch] = useState('');
  const [inputFragmentsExpanded, setInputFragmentsExpanded] = useState(false);
  const [outputFragmentsExpanded, setOutputFragmentsExpanded] = useState(false);
  const [selectedExplorerItem, setSelectedExplorerItem] = useState<ExplorerSelection | null>(null);
  const [controlAvailability, setControlAvailability] = useState<{ can_resume: boolean; can_next_step: boolean }>({
    can_resume: false,
    can_next_step: false,
  });
  const [optimisticStep, setOptimisticStep] = useState<OptimisticStep | null>(null);
  const [availablePipelines, setAvailablePipelines] = useState<{id: string, title: string}[]>([]);
  const [selectedPipelineId, setSelectedPipelineId] = useState<string>('');
  const [debugViewMode, setDebugViewMode] = useState<'list' | 'graph'>('list');
  const [petriGraphData, setPetriGraphData] = useState<{head: any, childrenFragments: Record<string, any>} | null>(null);
  const debugReloadInFlightKeyRef = useRef<string | null>(null);
  const logDockResizeRef = useRef<{ startY: number; startHeight: number } | null>(null);
  const explorerInspectionRequestRef = useRef(0);
  const explorerPreviewRequestRef = useRef(0);
  const selectedStepIdRef = useRef<string | null>(null);

  const clampLogDockHeight = (height: number) => Math.min(LOG_DOCK_MAX_HEIGHT, Math.max(LOG_DOCK_MIN_HEIGHT, height));

  useEffect(() => {
    selectedStepIdRef.current = selectedStepId;
  }, [selectedStepId]);

  useEffect(() => {
    const onPointerMove = (event: MouseEvent) => {
      if (!logDockResizeRef.current) {
        return;
      }
      const deltaY = logDockResizeRef.current.startY - event.clientY;
      setLogDockHeight(clampLogDockHeight(logDockResizeRef.current.startHeight + deltaY));
    };

    const stopResize = () => {
      logDockResizeRef.current = null;
    };

    window.addEventListener('mousemove', onPointerMove);
    window.addEventListener('mouseup', stopResize);

    return () => {
      window.removeEventListener('mousemove', onPointerMove);
      window.removeEventListener('mouseup', stopResize);
    };
  }, []);

  const onLogDockResizeStart = (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    logDockResizeRef.current = {
      startY: event.clientY,
      startHeight: logDockHeight,
    };
  };

  useEffect(() => {
    const loadPipelines = async () => {
      try {
        const registry = await getRegistry('petri_net_runnables');
        const pipelines = registry.entries.map((e: any) => ({
          id: e.key,
          title: e.title || e.key.slice(0, 12) + '...',
        }));
        setAvailablePipelines(pipelines);
        if (pipelines.length > 0) {
          setSelectedPipelineId(pipelines[0].id);
        }
      } catch (err) {
        console.error('Failed to load pipelines:', err);
      }
    };
    void loadPipelines();
  }, []);

  const pipelineContext = useMemo(() => {
    if (!activeRun) {
      return null;
    }
    const debugPipeline = activeRun.evaluation?.details?.debug_pipeline;
    const pipelineId = debugPipeline?.pipeline_id ?? activeRun.pipeline_id ?? null;
    if (!pipelineId) {
      return null;
    }
    return {
      pipelineId,
      pipelineRunId: debugPipeline?.pipeline_run_id ?? activeRun.pipeline_run_id ?? activeRun.run_id,
    };
  }, [activeRun]);

  const activeDebugKey = useMemo(() => {
    if (!activeRun || !pipelineContext) {
      return null;
    }
    return `${activeRun.run_id}:${pipelineContext.pipelineId}:${pipelineContext.pipelineRunId}`;
  }, [activeRun, pipelineContext]);

  const activeOptimisticStep = optimisticStep?.debugKey === activeDebugKey ? optimisticStep : null;
  const activeRunDebugEvents = loadedDebugKey === activeDebugKey ? debugEvents : [];

  const displayedDebugEvents = useMemo(() => {
    if (!activeOptimisticStep) {
      return activeRunDebugEvents;
    }
    const hasRealReplacement = activeRunDebugEvents.some(
      (event) => event.step_name === activeOptimisticStep.stepName && event.attempt_index >= activeOptimisticStep.attemptIndex
    );
    if (hasRealReplacement) {
      return activeRunDebugEvents;
    }
    return [
      ...activeRunDebugEvents,
      {
        event_id: activeOptimisticStep.stepId,
        step_id: activeOptimisticStep.stepId,
        step_name: activeOptimisticStep.stepName,
        status: 'running',
        attempt_index: activeOptimisticStep.attemptIndex,
        env_type: activeOptimisticStep.envType,
        env_id: activeOptimisticStep.envId,
      } satisfies DebugStepEvent,
    ];
  }, [activeOptimisticStep, activeRunDebugEvents]);

  const selectedEvent = useMemo(
    () => displayedDebugEvents.find((event) => event.step_id === selectedStepId) ?? null,
    [displayedDebugEvents, selectedStepId]
  );

  useEffect(() => {
    setInputFragmentSearch('');
    setOutputFragmentSearch('');
    setInputFragmentsExpanded(false);
    setOutputFragmentsExpanded(false);
    setSelectedExplorerItem(null);
  }, [selectedStepId]);

  const currentOutputs = useMemo(() => {
    if (!stepDetailPayload || typeof stepDetailPayload.outputs !== 'object' || !stepDetailPayload.outputs) {
      return {} as Record<string, unknown>;
    }
    return stepDetailPayload.outputs as Record<string, unknown>;
  }, [stepDetailPayload]);

  const currentInputs = useMemo(() => {
    if (!stepDetailPayload || typeof stepDetailPayload.inputs !== 'object' || !stepDetailPayload.inputs) {
      return {} as Record<string, unknown>;
    }
    return stepDetailPayload.inputs as Record<string, unknown>;
  }, [stepDetailPayload]);

  const outputDocuments = useMemo(
    () => (Array.isArray(currentOutputs.documents)
      ? currentOutputs.documents.filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
      : []),
    [currentOutputs.documents]
  );

  const selectedEnvironmentContext = useMemo(() => {
    if (!selectedEvent) {
      return null;
    }

    const outcome = stepDetailPayload?.outcome && typeof stepDetailPayload.outcome === 'object'
      ? stepDetailPayload.outcome as Record<string, unknown>
      : null;
    const envType = selectedEvent.env_type ?? asString(outcome?.env_type) ?? null;
    const envId = selectedEvent.env_id ?? asString(outcome?.env_id) ?? null;

    if (!envType || !envId) {
      return null;
    }

    return {
      envType,
      envId,
      stepId: selectedEvent.step_id,
      attemptIndex: selectedEvent.attempt_index,
    };
  }, [selectedEvent, stepDetailPayload]);

  const selectedScopeRef = useMemo(() => {
    if (!selectedEvent) {
      return null;
    }

    const outcome = stepDetailPayload?.outcome && typeof stepDetailPayload.outcome === 'object'
      ? stepDetailPayload.outcome as Record<string, unknown>
      : null;

    return getScopeRef({
      ref: asString(outcome?.ref) ?? selectedEvent.ref,
      env_type: selectedEvent.env_type ?? asString(outcome?.env_type),
      env_id: selectedEvent.env_id ?? asString(outcome?.env_id),
    });
  }, [selectedEvent, stepDetailPayload]);

  const documentLoads = useMemo(
    () => (Array.isArray(currentOutputs.document_loads)
      ? currentOutputs.document_loads.filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
      : []),
    [currentOutputs.document_loads]
  );

  const documentLoadByArtifactId = useMemo(() => {
    const map = new Map<string, Record<string, unknown>>();
    for (const item of documentLoads) {
      const artifactId = asString(item.artifact_id);
      if (!artifactId || map.has(artifactId)) continue;
      map.set(artifactId, item);
    }
    return map;
  }, [documentLoads]);

  const documentCountsByArtifactId = useMemo(() => {
    const counts = new Map<string, number>();
    for (const item of outputDocuments) {
      const artifactId = asString(item.artifact_id);
      if (!artifactId) continue;
      counts.set(artifactId, (counts.get(artifactId) ?? 0) + 1);
    }
    for (const item of documentLoads) {
      const artifactId = asString(item.artifact_id);
      if (!artifactId) continue;
      const reportedCount = asNumber(item.document_count);
      if (reportedCount != null) {
        counts.set(artifactId, reportedCount);
      }
    }
    return counts;
  }, [documentLoads, outputDocuments]);

  const artifactFallback = useMemo(() => {
    const artifactIds = Array.from(
      new Set([
        ...asStringArray(currentInputs.artifact_ids),
        ...asStringArray(currentOutputs.artifact_ids),
        ...documentLoads.map((item) => asString(item.artifact_id)).filter((item): item is string => Boolean(item)),
        ...outputDocuments.map((item) => asString(item.artifact_id)).filter((item): item is string => Boolean(item)),
      ])
    );

    const roots: string[] = [];
    const nodes = new Map<string, DetailNode>();
    for (const artifactId of artifactIds) {
      const load = documentLoadByArtifactId.get(artifactId);
      const document = outputDocuments.find((item) => asString(item.artifact_id) === artifactId) ?? null;
      const fileName = asString(load?.filename)
        ?? asString(document?.filename)
        ?? asString((document?.metadata as Record<string, unknown> | undefined)?.file_name)
        ?? basenameFromArtifactId(artifactId);
      const mimeType = asString(load?.mime_type) ?? asString(document?.mime_type);
      const nodeId = `artifact:${artifactId}`;
      roots.push(nodeId);
      nodes.set(nodeId, {
        node_id: nodeId,
        kind: 'artifact',
        fragment_id: artifactId,
        mime_type: mimeType,
        label: fileName,
        meta: {
          artifact_id: artifactId,
          file_name: fileName,
          filename: fileName,
          reader_key: asString(load?.reader_key),
          reader_library: asString(load?.reader_library),
          reader_method: asString(load?.reader_method),
          load_status: asString(load?.status),
          document_count: documentCountsByArtifactId.get(artifactId) ?? 0,
        },
      });
    }
    return { roots, nodes };
  }, [currentInputs.artifact_ids, currentOutputs.artifact_ids, documentCountsByArtifactId, documentLoadByArtifactId, documentLoads, outputDocuments]);

  const producedFragmentIds = useMemo(() => {
    if (!stepDetailPayload) {
      return [] as string[];
    }
    const topLevelProduced = asStringArray((stepDetailPayload as Record<string, unknown>).produced_fragment_ids);
    if (topLevelProduced.length > 0) {
      return topLevelProduced;
    }
    const outputProduced = asStringArray(currentOutputs.produced_fragment_ids);
    if (outputProduced.length > 0) {
      return outputProduced;
    }
    return [] as string[];
  }, [currentOutputs.produced_fragment_ids, stepDetailPayload]);

  const stepBoundaries = useMemo(() => {
    const raw = stepDetailPayload?.step_boundaries;
    return raw && typeof raw === 'object' ? raw as Record<string, unknown> : null;
  }, [stepDetailPayload]);

  const boundaryInput = useMemo(() => {
    const raw = stepBoundaries?.input_boundary;
    return raw && typeof raw === 'object' ? raw as Record<string, unknown> : null;
  }, [stepBoundaries]);

  const boundaryTransition = useMemo(() => {
    const raw = stepBoundaries?.transition;
    return raw && typeof raw === 'object' ? raw as Record<string, unknown> : null;
  }, [stepBoundaries]);

  const boundaryOutput = useMemo(() => {
    const raw = stepBoundaries?.output_boundary;
    return raw && typeof raw === 'object' ? raw as Record<string, unknown> : null;
  }, [stepBoundaries]);

  const boundaryEnvironmentBefore = useMemo(() => {
    const raw = stepBoundaries?.ikam_environment_before;
    return raw && typeof raw === 'object' ? raw as Record<string, unknown> : null;
  }, [stepBoundaries]);

  const boundaryEnvironmentAfter = useMemo(() => {
    const raw = stepBoundaries?.ikam_environment_after;
    return raw && typeof raw === 'object' ? raw as Record<string, unknown> : null;
  }, [stepBoundaries]);

  const boundaryHandoff = useMemo(() => {
    const raw = stepBoundaries?.handoff_to_next;
    return raw && typeof raw === 'object' ? raw as Record<string, unknown> : null;
  }, [stepBoundaries]);

  const contextInputFragments = useMemo(() => {
    const fragments = asStringArray(currentInputs.fragment_ids);
    if (fragments.length > 0) return fragments;
    return scopedFragments.map((fragment) => fragment.id);
  }, [currentInputs.fragment_ids, scopedFragments]);

  const contextOutputFragments = useMemo(() => {
    const direct = asStringArray(currentOutputs.fragment_ids);
    if (direct.length > 0) return direct;
    if (producedFragmentIds.length > 0) return producedFragmentIds;
    return scopedFragments.map((fragment) => fragment.id);
  }, [currentOutputs.fragment_ids, producedFragmentIds, scopedFragments]);

  const fallbackBoundaryOutput = useMemo(() => {
    if (boundaryOutput) return boundaryOutput;
    if (!selectedEvent) return null;
    const fragmentIds = contextOutputFragments;
    const programIds = asStringArray(currentOutputs.program_ids);
    if (fragmentIds.length === 0 && programIds.length === 0 && producedFragmentIds.length === 0) {
      return null;
    }
    return {
      fragment_ids: fragmentIds,
      program_ids: programIds,
      produced_fragment_ids: producedFragmentIds,
    } as Record<string, unknown>;
  }, [boundaryOutput, contextOutputFragments, currentOutputs.program_ids, producedFragmentIds, selectedEvent]);

  const fallbackBoundaryEnvironmentAfter = useMemo(() => {
    if (boundaryEnvironmentAfter) return boundaryEnvironmentAfter;
    if (!selectedEvent && !selectedScopeRef) return null;
    return {
      active_ref: selectedScopeRef,
      fragment_count: contextOutputFragments.length,
      reconstruction_program_count: asStringArray(currentOutputs.program_ids).length,
      visible_fragment_ids: contextOutputFragments,
    } as Record<string, unknown>;
  }, [boundaryEnvironmentAfter, contextOutputFragments, currentOutputs.program_ids, selectedEvent, selectedScopeRef]);

  const fallbackBoundaryHandoff = useMemo(() => {
    if (boundaryHandoff) return boundaryHandoff;
    if (contextOutputFragments.length === 0) return null;
    return {
      next_step_name: pipelineSteps[pipelineSteps.indexOf(selectedEvent?.step_name ?? '') + 1] ?? 'n/a',
      source: 'current_step_outputs',
      forwarded_fragment_ids: contextOutputFragments,
      forwarded_program_ids: asStringArray(currentOutputs.program_ids),
    } as Record<string, unknown>;
  }, [boundaryHandoff, contextOutputFragments, currentOutputs.program_ids, pipelineSteps, selectedEvent?.step_name]);

  const outcomePayload = stepDetailPayload?.outcome && typeof stepDetailPayload.outcome === 'object'
    ? stepDetailPayload.outcome as Record<string, unknown>
    : null;
  const selectedStepStatus = selectedEvent?.status ?? asString(outcomePayload?.status) ?? 'idle';
  const selectedStepDuration = asNumber(outcomePayload?.duration_ms) ?? asNumber(boundaryTransition?.duration_ms);

  const detailRows = useMemo(() => {
    if (!selectedEvent || !stepDetailPayload) {
      return [] as Array<{ label: string; value: string }>;
    }

    const rows: Array<{ label: string; value: string }> = [];
    const status = selectedEvent.status;
    rows.push({ label: 'Status', value: status.toUpperCase() });
    rows.push({ label: 'Attempt', value: String(selectedEvent.attempt_index) });
    rows.push({ label: 'Duration', value: selectedEvent.duration_ms != null ? `${selectedEvent.duration_ms} ms` : 'n/a' });
    const scopeRef = getScopeRef({
      ref: typeof stepDetailPayload.outcome === 'object' && stepDetailPayload.outcome
        ? asString((stepDetailPayload.outcome as Record<string, unknown>).ref)
        : undefined,
      env_type: selectedEvent.env_type,
      env_id: selectedEvent.env_id,
    });
    if (scopeRef) {
      rows.push({ label: 'Scope Ref', value: scopeRef });
    }

    const outputs = currentOutputs;

    const embeddingCount = asNumber(outputs.embedding_count);
    const dimensions = asNumber(outputs.embedding_dimensions);
    const clusterCount = Array.isArray(outputs.clusters) ? outputs.clusters.length : asNumber(outputs.cluster_count);
    const normalizedCount = Array.isArray(outputs.fragment_ids) ? outputs.fragment_ids.length : asNumber(outputs.normalized_fragment_count);
    const reconstructionCount = Array.isArray(outputs.program_ids) ? outputs.program_ids.length : asNumber(outputs.reconstruction_program_count);
    const candidateCount = Array.isArray(outputs.candidates) ? outputs.candidates.length : null;
    const fragments = Array.isArray(outputs.fragment_ids) ? outputs.fragment_ids.length : null;
    const artifactCount = asNumber(outputs.artifact_count);
    const totalSize = asNumber(outputs.total_size_bytes);
    const artifactTypes = outputs.artifact_type_counts && typeof outputs.artifact_type_counts === 'object'
      ? Object.keys(outputs.artifact_type_counts as Record<string, unknown>).length
      : null;

    if (embeddingCount != null) rows.push({ label: 'Embeddings', value: String(embeddingCount) });
    if (dimensions != null) rows.push({ label: 'Vector Dimensions', value: String(dimensions) });
    if (clusterCount != null) rows.push({ label: 'Clusters', value: String(clusterCount) });
    if (candidateCount != null) rows.push({ label: 'Candidate Pairs', value: String(candidateCount) });
    if (normalizedCount != null) rows.push({ label: 'Normalized Fragments', value: String(normalizedCount) });
    if (reconstructionCount != null) rows.push({ label: 'Reconstruction Programs', value: String(reconstructionCount) });
    if (fragments != null) rows.push({ label: 'Fragments in Payload', value: String(fragments) });
    if (artifactCount != null) rows.push({ label: 'Artifacts', value: String(artifactCount) });
    if (totalSize != null) rows.push({ label: 'Artifact Bytes', value: String(totalSize) });
    if (artifactTypes != null) rows.push({ label: 'Artifact Types', value: String(artifactTypes) });

    return rows;
  }, [currentOutputs, selectedEvent, stepDetailPayload]);

  const canonicalDetail = useMemo(() => {
    if (!stepDetailPayload) {
      return null;
    }
    const required = ['schema_version', 'outcome', 'why', 'inputs', 'outputs', 'checks', 'lineage'];
    const missing = required.filter((key) => !(key in stepDetailPayload));
    if (missing.length > 0) {
      return { schemaValid: false as const, missing };
    }
    return { schemaValid: true as const, payload: stepDetailPayload };
  }, [stepDetailPayload]);

  const lineageRoots = useMemo(() => {
    if (!canonicalDetail || !canonicalDetail.schemaValid) return [] as string[];
    const roots = (canonicalDetail.payload.lineage as Record<string, unknown>).roots;
    const rootIds = Array.isArray(roots) ? roots.filter((item): item is string => typeof item === 'string') : [];
    if (rootIds.length === 0) {
      return artifactFallback.roots;
    }
    if (canonicalDetail.payload.step_name !== TRANSITIONS.MAP) {
      return rootIds;
    }
    const outputs = canonicalDetail.payload.outputs as Record<string, unknown> | undefined;
    const mapPayload = outputs?.map as Record<string, unknown> | undefined;
    const mapRootNodeId = asString(mapPayload?.root_node_id);
    if (!mapRootNodeId) {
      return rootIds;
    }
    if (!rootIds.includes(mapRootNodeId)) {
      return rootIds;
    }
    return [mapRootNodeId, ...rootIds.filter((nodeId) => nodeId !== mapRootNodeId)];
  }, [artifactFallback.roots, canonicalDetail]);

  const reconstructionCompleteness = useMemo(() => {
    if (!canonicalDetail || !canonicalDetail.schemaValid) {
      return null;
    }
    if (canonicalDetail.payload.step_name !== TRANSITIONS.MAP) {
      return null;
    }
    const outputs = canonicalDetail.payload.outputs as Record<string, unknown> | undefined;
    if (!outputs || typeof outputs !== 'object') {
      return null;
    }
    const decomposition = outputs.decomposition as Record<string, unknown> | undefined;
    if (!decomposition || typeof decomposition !== 'object') {
      return null;
    }
    const structural = Array.isArray(decomposition.structural_fragment_ids)
      ? decomposition.structural_fragment_ids.filter((item): item is string => typeof item === 'string')
      : [];
    const roots = Array.isArray(decomposition.root_fragment_ids)
      ? decomposition.root_fragment_ids.filter((item): item is string => typeof item === 'string')
      : [];
    const rootSet = new Set(roots);
    const missing = structural.filter((item) => !rootSet.has(item));
    const pass = roots.length > structural.length && missing.length === 0;
    return {
      pass,
      structuralCount: structural.length,
      rootCount: roots.length,
      missing,
    };
  }, [canonicalDetail]);

  const boundaryDiagnostics = useMemo(() => {
    if (!canonicalDetail || !canonicalDetail.schemaValid) return [] as Array<Record<string, unknown>>;
    if (canonicalDetail.payload.step_name !== TRANSITIONS.MAP) return [] as Array<Record<string, unknown>>;
    const outputs = canonicalDetail.payload.outputs as Record<string, unknown> | undefined;
    const decomposition = outputs?.decomposition as Record<string, unknown> | undefined;
    const diagnostics = decomposition?.boundary_diagnostics;
    if (!Array.isArray(diagnostics)) return [] as Array<Record<string, unknown>>;
    const priority = { failed: 0, coarse: 1, good: 2 } as Record<string, number>;
    const rows = diagnostics.filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'));
    rows.sort((a, b) => {
      const pa = priority[String(a.status ?? '')] ?? 99;
      const pb = priority[String(b.status ?? '')] ?? 99;
      if (pa !== pb) return pa - pb;
      return String(a.file_name ?? '').localeCompare(String(b.file_name ?? ''));
    });
    return rows;
  }, [canonicalDetail]);

  const boundarySummary = useMemo(() => {
    if (boundaryDiagnostics.length === 0) return null;
    const summary = { failed: 0, coarse: 0, good: 0 };
    for (const row of boundaryDiagnostics) {
      const status = String(row.status ?? '');
      if (status === 'failed' || status === 'coarse' || status === 'good') {
        summary[status] += 1;
      }
    }
    return summary;
  }, [boundaryDiagnostics]);

  const stepTrace = useMemo<StepTrace | null>(() => {
    if (!stepDetailPayload) {
      return null;
    }
    const raw = stepDetailPayload.trace;
    if (!raw || typeof raw !== 'object') {
      return null;
    }
    const trace = raw as Record<string, unknown>;
    const topicSequence = Array.isArray(trace.topic_sequence)
      ? trace.topic_sequence
          .filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
          .map((item) => ({
            topic: asString(item.topic) ?? 'unknown-topic',
            eventType: asString(item.event_type) ?? 'unknown-event',
            status: asString(item.status) ?? 'unknown-status',
          }))
      : [];
    const normalized = {
      workflowId: asString(trace.workflow_id),
      requestId: asString(trace.request_id),
      executorId: asString(trace.executor_id),
      executorKind: asString(trace.executor_kind),
      transitionId: asString(trace.transition_id),
      markingBeforeRef: asString(trace.marking_before_ref),
      markingAfterRef: asString(trace.marking_after_ref),
      enabledTransitionIds: asStringArray(trace.enabled_transition_ids),
      topicSequence,
      traceId: asString(trace.trace_id),
      traceFragmentId: asString(trace.trace_fragment_id) ?? asString(trace.committed_trace_fragment_id),
      timeline: Array.isArray(trace.timeline)
        ? trace.timeline.filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
        : [],
      rawEvents: Array.isArray(trace.raw_events)
        ? trace.raw_events.filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
        : [],
    };
    const hasValue = Boolean(
      normalized.workflowId
      || normalized.requestId
      || normalized.executorId
      || normalized.executorKind
      || normalized.transitionId
      || normalized.markingBeforeRef
      || normalized.markingAfterRef
      || normalized.enabledTransitionIds.length > 0
      || normalized.topicSequence.length > 0
      || normalized.timeline.length > 0
      || normalized.rawEvents.length > 0
      || normalized.traceId
      || normalized.traceFragmentId
    );
    return hasValue ? normalized : null;
  }, [stepDetailPayload]);

  const lineageNodes = useMemo(() => {
    const map = new Map<string, DetailNode>();
    if (canonicalDetail && canonicalDetail.schemaValid) {
      const nodes = (canonicalDetail.payload.lineage as Record<string, unknown>).nodes;
      if (Array.isArray(nodes)) {
        for (const node of nodes) {
          if (!node || typeof node !== 'object') continue;
          const payload = node as Record<string, unknown>;
          const nodeId = asString(payload.node_id);
          if (!nodeId) continue;
          map.set(nodeId, {
            node_id: nodeId,
            kind: asString(payload.kind) ?? 'surface',
            fragment_id: asString(payload.fragment_id),
            cas_id: asString(payload.cas_id),
            mime_type: asString(payload.mime_type),
            label: asString(payload.label) ?? asString(payload.fragment_id) ?? nodeId,
            meta: payload.meta && typeof payload.meta === 'object' ? (payload.meta as Record<string, unknown>) : undefined,
          });
        }
      }
    }
    artifactFallback.nodes.forEach((node, nodeId) => {
      if (!map.has(nodeId)) {
        map.set(nodeId, node);
      }
    });
    return map;
  }, [artifactFallback.nodes, canonicalDetail]);

  const childrenByNode = useMemo(() => {
    const map = new Map<string, string[]>();
    if (canonicalDetail && canonicalDetail.schemaValid) {
      const edges = (canonicalDetail.payload.lineage as Record<string, unknown>).edges;
      if (Array.isArray(edges)) {
        for (const edge of edges) {
          if (!edge || typeof edge !== 'object') continue;
          const payload = edge as Record<string, unknown>;
          const from = asString(payload.from);
          const to = asString(payload.to);
          if (!from || !to) continue;
          const existing = map.get(from) ?? [];
          if (!existing.includes(to)) existing.push(to);
          map.set(from, existing);
        }
      }
    }
    return map;
  }, [canonicalDetail]);

  const lineageEdges = useMemo(() => {
    const edges: DetailEdge[] = [];
    if (!canonicalDetail || !canonicalDetail.schemaValid) return edges;
    const rawEdges = (canonicalDetail.payload.lineage as Record<string, unknown>).edges;
    if (!Array.isArray(rawEdges)) return edges;
    for (const edge of rawEdges) {
      if (!edge || typeof edge !== 'object') continue;
      const payload = edge as Record<string, unknown>;
      const from = asString(payload.from);
      const to = asString(payload.to);
      if (!from || !to) continue;
      edges.push({ from, to, relation: asString(payload.relation) ?? 'contains' });
    }
    return edges;
  }, [canonicalDetail]);

  const parentByChild = useMemo(() => {
    const map = new Map<string, string>();
    childrenByNode.forEach((children, parent) => {
      for (const child of children) {
        map.set(child, parent);
      }
    });
    return map;
  }, [childrenByNode]);

  const filteredNodeIds = useMemo(() => {
    const query = fragmentSearch.trim().toLowerCase();
    if (!query) {
      return null;
    }
    const keep = new Set<string>();
    lineageNodes.forEach((node, nodeId) => {
      const fragmentId = (node.fragment_id ?? '').toLowerCase();
      const casId = (node.cas_id ?? '').toLowerCase();
      const label = (node.label ?? '').toLowerCase();
      if (!fragmentId.includes(query) && !casId.includes(query) && !label.includes(query)) {
        return;
      }
      keep.add(nodeId);
      let current = nodeId;
      while (parentByChild.has(current)) {
        const parent = parentByChild.get(current);
        if (!parent) break;
        keep.add(parent);
        current = parent;
      }
    });
    return keep;
  }, [fragmentSearch, lineageNodes, parentByChild]);

  const selectedNode = useMemo(() => {
    if (selectedNodeId) {
      return lineageNodes.get(selectedNodeId) ?? null;
    }
    const fallbackRootId = lineageRoots[0] ?? null;
    return fallbackRootId ? (lineageNodes.get(fallbackRootId) ?? null) : null;
  }, [lineageRoots, selectedNodeId, lineageNodes]);

  const embedSimilarityViz = useMemo(() => {
    if (!selectedEvent || !stepDetailPayload) {
      return null;
    }
    if (selectedEvent.step_name !== TRANSITIONS.EMBED_MAPPED && selectedEvent.step_name !== TRANSITIONS.EMBED_LIFTED) {
      return null;
    }
    const outputs = stepDetailPayload.outputs && typeof stepDetailPayload.outputs === 'object'
      ? (stepDetailPayload.outputs as Record<string, unknown>)
      : null;
    if (!outputs) return null;
    const pairwise = outputs.pairwise_similarity && typeof outputs.pairwise_similarity === 'object'
      ? (outputs.pairwise_similarity as Record<string, unknown>)
      : null;
    const debug = outputs.embedding_debug && typeof outputs.embedding_debug === 'object'
      ? (outputs.embedding_debug as Record<string, unknown>)
      : {};
    if (!pairwise) return null;

    const fragmentIds = Array.isArray(pairwise.fragment_ids)
      ? pairwise.fragment_ids.filter((item): item is string => typeof item === 'string')
      : [];
    const matrix = Array.isArray(pairwise.matrix)
      ? pairwise.matrix.map((row) => (Array.isArray(row) ? row.map((cell) => (typeof cell === 'number' ? cell : 0)) : []))
      : [];

    const fragmentItems = fragmentIds.map((fragmentId) => {
      const node = Array.from(lineageNodes.values()).find((item) => item.fragment_id === fragmentId);
      const displayId = fragmentId;
      const fileName = asString(node?.meta?.file_name) || asString(node?.meta?.filename) || node?.label || displayId;
      const mimeDescription = describeMime(node?.mime_type);
      return {
        fragment_id: fragmentId,
        label: `${shortId(displayId)} / ${fileName} / ${mimeDescription}`,
      };
    });

    return {
      fragmentItems,
      pairwise: {
        fragment_ids: fragmentIds,
        matrix,
        threshold: typeof pairwise.threshold === 'number' ? pairwise.threshold : null,
      },
      debug: {
        expected_count: typeof debug.expected_count === 'number' ? debug.expected_count : undefined,
        embedded_count: typeof debug.embedded_count === 'number' ? debug.embedded_count : undefined,
        coverage_ratio: typeof debug.coverage_ratio === 'number' ? debug.coverage_ratio : undefined,
        singleton_clusters: typeof debug.singleton_clusters === 'number' ? debug.singleton_clusters : undefined,
        missing_fragment_ids: Array.isArray(debug.missing_fragment_ids)
          ? debug.missing_fragment_ids.filter((item): item is string => typeof item === 'string')
          : undefined,
        threshold: typeof debug.threshold === 'number' ? debug.threshold : undefined,
        embedding_mode: typeof debug.embedding_mode === 'string' ? debug.embedding_mode : null,
      },
    };
  }, [lineageNodes, selectedEvent, stepDetailPayload]);

  const liftTransformations = useMemo(() => {
    if (!selectedEvent || !stepDetailPayload) {
      return [] as LiftTransformation[];
    }
    if (selectedEvent.step_name !== TRANSITIONS.LIFT && selectedEvent.step_name !== TRANSITIONS.EMBED_LIFTED) {
      return [] as LiftTransformation[];
    }
    const outputs = stepDetailPayload.outputs && typeof stepDetailPayload.outputs === 'object'
      ? (stepDetailPayload.outputs as Record<string, unknown>)
      : null;
    if (outputs && Array.isArray(outputs.lift_transformations)) {
      return outputs.lift_transformations
        .filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
        .map((item) => {
          const liftStatus = asString(item.lift_status) === 'surface_only' ? 'surface_only' : 'lifted';
          return {
            surfaceFragmentId: asString(item.surface_fragment_id) ?? '',
            sourceArtifactId: asString(item.source_artifact_id),
            irFragmentIds: asStringArray(item.ir_fragment_ids),
            liftStatus,
            liftReason: asString(item.lift_reason),
          };
        })
        .filter((item) => item.surfaceFragmentId.length > 0);
    }

    if (selectedEvent.step_name !== TRANSITIONS.EMBED_LIFTED) {
      return [] as LiftTransformation[];
    }

    const irBySurface = new Map<string, string[]>();
    for (const edge of lineageEdges) {
      const fromNode = lineageNodes.get(edge.from);
      const toNode = lineageNodes.get(edge.to);
      if (!fromNode || !toNode) continue;
      if (fromNode.kind !== 'surface' || toNode.kind !== 'ir') continue;
      const surfaceId = fromNode.fragment_id ?? fromNode.node_id;
      const irId = toNode.fragment_id ?? toNode.node_id;
      const existing = irBySurface.get(surfaceId) ?? [];
      if (!existing.includes(irId)) {
        irBySurface.set(surfaceId, [...existing, irId]);
      }
    }

    const surfaces = Array.from(lineageNodes.values()).filter((node) => node.kind === 'surface');
    return surfaces.map((node) => {
      const surfaceId = node.fragment_id ?? node.node_id;
      const irFragmentIds = irBySurface.get(surfaceId) ?? [];
      return {
        surfaceFragmentId: surfaceId,
        sourceArtifactId: asString(node.meta?.artifact_id),
        irFragmentIds,
        liftStatus: irFragmentIds.length > 0 ? 'lifted' : 'surface_only',
        liftReason: irFragmentIds.length > 0 ? null : 'no_ir_generated',
      };
    });
  }, [lineageEdges, lineageNodes, selectedEvent, stepDetailPayload]);

  const liftTransformationBySurface = useMemo(
    () => new Map(liftTransformations.map((item) => [item.surfaceFragmentId, item])),
    [liftTransformations]
  );

  const liftSourceByIr = useMemo(() => {
    const map = new Map<string, string>();
    for (const transformation of liftTransformations) {
      for (const irId of transformation.irFragmentIds) {
        map.set(irId, transformation.surfaceFragmentId);
      }
    }
    return map;
  }, [liftTransformations]);

  const selectedLiftTransformation = useMemo(() => {
    if (!selectedNode || selectedEvent?.step_name !== TRANSITIONS.LIFT) return null;
    const selectedFragmentId = selectedNode.fragment_id ?? selectedNode.node_id;
    if (selectedNode.kind === 'surface') {
      return liftTransformationBySurface.get(selectedFragmentId) ?? null;
    }
    const sourceSurface = liftSourceByIr.get(selectedFragmentId);
    if (!sourceSurface) return null;
    return liftTransformationBySurface.get(sourceSurface) ?? null;
  }, [liftSourceByIr, liftTransformationBySurface, selectedEvent?.step_name, selectedNode]);

  const selectedLiftIrPreviews = useMemo(() => {
    if (!selectedLiftTransformation) return [] as ArtifactPreviewResponse[];
    return selectedLiftTransformation.irFragmentIds
      .map((fragmentId) => {
        const node = Array.from(lineageNodes.values()).find((item) => item.fragment_id === fragmentId) ?? null;
        return buildPreviewFromNode(node) ?? buildFallbackPreview(fragmentId);
      });
  }, [lineageNodes, selectedLiftTransformation]);

  const selectedLiftSurfacePreview = useMemo(() => {
    if (!selectedLiftTransformation) return null;
    const node = Array.from(lineageNodes.values()).find((item) => item.fragment_id === selectedLiftTransformation.surfaceFragmentId) ?? null;
    return buildPreviewFromNode(node) ?? buildFallbackPreview(selectedLiftTransformation.surfaceFragmentId);
  }, [lineageNodes, selectedLiftTransformation]);

  const selectedTransformationFlow = useMemo(() => {
    if (!selectedEvent || !selectedNode) {
      return null;
    }

    const selectedFragmentId = selectedNode.fragment_id ?? selectedNode.node_id;
    const nodeLabel = `${shortId(selectedFragmentId)} / ${selectedNode.kind}`;
    const indexOfStep = (name: string): number => pipelineSteps.indexOf(name);
    const currentStepIndex = indexOfStep(selectedEvent.step_name);
    const hasReached = (name: string): boolean => {
      const targetIndex = indexOfStep(name);
      if (currentStepIndex < 0) {
        return false;
      }
      if (targetIndex < 0) {
        return false;
      }
      return currentStepIndex >= targetIndex;
    };
    const hasSurfaceNodes = Array.from(lineageNodes.values()).some((node) => node.kind === 'surface');
    const hasIrNodes = Array.from(lineageNodes.values()).some((node) => node.kind === 'ir');
    const hasNormalizedNodes = Array.from(lineageNodes.values()).some((node) => {
      const recordType = asString(node.meta?.record_type) ?? '';
      return node.kind === 'normalized' || recordType.includes('normalized');
    });
    const isPreSemanticStep = selectedEvent.step_name === TRANSITIONS.INIT || selectedEvent.step_name === 'load.documents';
    const canShowMap = hasReached(TRANSITIONS.MAP) || (!isPreSemanticStep && hasSurfaceNodes);
    const canShowLift = hasReached(TRANSITIONS.LIFT) || (!isPreSemanticStep && hasIrNodes);
    const canShowNormalize = hasReached(TRANSITIONS.NORMALIZE) || (!isPreSemanticStep && hasNormalizedNodes);

    const findNodeIdByFragment = (fragmentId: string): string | null => {
      for (const [nodeId, node] of lineageNodes.entries()) {
        if (node.fragment_id === fragmentId) {
          return nodeId;
        }
      }
      return null;
    };

    const resolveArtifactNodeId = (): string | null => {
      if (selectedNode.kind === 'artifact') {
        return selectedNode.node_id;
      }
      const fromMeta = asString(selectedNode.meta?.artifact_id);
      if (fromMeta) {
        const metaNode = findNodeIdByFragment(fromMeta);
        if (metaNode) {
          return metaNode;
        }
      }
      let current = selectedNode.node_id;
      while (parentByChild.has(current)) {
        const parentId = parentByChild.get(current);
        if (!parentId) break;
        const parentNode = lineageNodes.get(parentId);
        if (parentNode?.kind === 'artifact') {
          return parentId;
        }
        current = parentId;
      }
      return null;
    };

    const artifactNodeId = resolveArtifactNodeId();
    const artifactNode = artifactNodeId ? lineageNodes.get(artifactNodeId) ?? null : null;
    const artifactId = artifactNode?.fragment_id ?? asString(selectedNode.meta?.artifact_id) ?? null;
    const artifactLabel = artifactNode?.label ?? artifactId ?? 'n/a';
    const artifactFileName = asString(artifactNode?.meta?.filename)
      ?? asString(artifactNode?.meta?.file_name)
      ?? artifactNode?.label
      ?? artifactId
      ?? selectedNode.label
      ?? selectedFragmentId;

    const descendants = new Set<string>();
    if (artifactNodeId) {
      const queue = [artifactNodeId];
      while (queue.length > 0) {
        const nodeId = queue.shift();
        if (!nodeId) continue;
        if (descendants.has(nodeId)) continue;
        descendants.add(nodeId);
        const children = childrenByNode.get(nodeId) ?? [];
        for (const child of children) {
          queue.push(child);
        }
      }
    }

    const artifactSurfaceIds = Array.from(lineageNodes.entries())
      .filter(([nodeId, node]) => node.kind === 'surface' && (descendants.has(nodeId) || asString(node.meta?.artifact_id) === artifactId))
      .map(([, node]) => node.fragment_id ?? node.node_id);

    const scopedSurfaceIds = selectedNode.kind === 'surface'
      ? [selectedFragmentId]
      : (selectedNode.kind === 'ir' && selectedLiftTransformation
        ? [selectedLiftTransformation.surfaceFragmentId]
        : artifactSurfaceIds);

    const flowNodeMap = new Map<string, TransformationFlowNode>();
    const flowEdges: TransformationFlowEdge[] = [];
    const stepDetails = new Map<string, TransformationFlowStepDetail>();
    const findNodeByFragment = (fragmentId: string): DetailNode | null => {
      for (const node of lineageNodes.values()) {
        if ((node.fragment_id ?? node.node_id) === fragmentId) {
          return node;
        }
      }
      return null;
    };

    const upsertFlowNode = (node: TransformationFlowNode) => {
      const existing = flowNodeMap.get(node.id);
      if (!existing) {
        flowNodeMap.set(node.id, node);
        return;
      }
      flowNodeMap.set(node.id, {
        ...existing,
        ...node,
        summary: node.summary || existing.summary,
      });
    };

    const addFlowEdge = (from: string, to: string) => {
      const edgeId = `edge:${from}->${to}`;
      if (flowEdges.some((edge) => edge.id === edgeId)) return;
      flowEdges.push({ id: edgeId, from, to });
    };

    const addStepDetail = (detail: TransformationFlowStepDetail) => {
      stepDetails.set(detail.id, detail);
    };

    const artifactFlowId = `flow:artifact:${artifactNodeId ?? artifactId ?? selectedFragmentId}`;
    upsertFlowNode({
      id: artifactFlowId,
      label: 'Source Artifact',
      caption: artifactFileName,
      nodeType: 'data',
      kind: 'artifact',
      stage: 'artifact',
      summary: `Artifact in scope: ${artifactLabel}`,
      state: 'complete',
      linkedNodeId: artifactNodeId ?? selectedNode.node_id,
    });

    if (selectedEvent.step_name === 'load.documents') {
      const documents = Array.isArray(currentOutputs.documents)
        ? currentOutputs.documents.filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
        : [];
      const documentLoads = Array.isArray(currentOutputs.document_loads)
        ? currentOutputs.document_loads.filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
        : [];
      const selectedArtifactId = artifactId ?? selectedFragmentId;
      const selectedLoad = documentLoads.find((item) => asString(item.artifact_id) === selectedArtifactId) ?? null;
      const selectedDocumentCount = asNumber(selectedLoad?.document_count) ?? documents.filter((item) => asString(item.artifact_id) === selectedArtifactId).length;
      const loadStepId = `flow:step:load:${selectedArtifactId}`;

      upsertFlowNode({
        id: loadStepId,
        label: 'Load Documents',
        caption: `${selectedDocumentCount} doc${selectedDocumentCount === 1 ? '' : 's'}`,
        nodeType: 'step',
        kind: 'operation',
        stage: 'surface',
        summary: 'Dispatch the selected artifact through its reader and emit reader documents.',
        state: selectedDocumentCount > 0 ? 'complete' : 'attention',
      });
      addFlowEdge(artifactFlowId, loadStepId);
      addStepDetail({
        id: loadStepId,
        title: 'Load Documents',
        subtitle: selectedEvent.step_name === 'load.documents' ? 'Current step' : 'Earlier transformation stage',
        summary: `${selectedDocumentCount} reader document${selectedDocumentCount === 1 ? '' : 's'}`,
        inputs: [asFlowRef(artifactFlowId, artifactFileName, artifactNodeId ?? selectedNode.node_id)],
        outputs: documents
          .filter((item) => asString(item.artifact_id) === selectedArtifactId)
          .map((item, index) => asFlowRef(
            `document:${selectedArtifactId}:${index}`,
            asString(item.filename) ?? asString(item.id) ?? `document-${index + 1}`,
            artifactNodeId ?? selectedNode.node_id,
          )),
      });

      const flowNodes = Array.from(flowNodeMap.values());
      return {
        artifactLabel: artifactFileName,
        selectedLabel: artifactFileName,
        nodes: flowNodes,
        edges: flowEdges,
        stepDetails,
      };
    }

    const scopedSurfaceNodes = scopedSurfaceIds
      .map((surfaceId) => findNodeByFragment(surfaceId))
      .filter((node): node is DetailNode => Boolean(node));
    const scopedSurfaceRefs = scopedSurfaceNodes.map((node) => asFlowRef(`surface:${node.node_id}`, node.fragment_id ?? node.node_id, node.node_id));
    const surfaceOnlyRefs = scopedSurfaceIds
      .filter((surfaceId) => (liftTransformationBySurface.get(surfaceId)?.liftStatus ?? null) === 'surface_only')
      .map((surfaceId) => {
        const surfaceNode = findNodeByFragment(surfaceId);
        return asFlowRef(`surface-only:${surfaceId}`, surfaceId, surfaceNode?.node_id ?? null);
      });

    const scopedIrFragments = new Set<string>();
    const scopedIrRefs: TransformationFlowRef[] = [];
    for (const surfaceId of scopedSurfaceIds) {
      const transformation = liftTransformationBySurface.get(surfaceId) ?? null;
      if (!transformation) continue;
      for (const irId of transformation.irFragmentIds) {
        if (scopedIrFragments.has(irId)) continue;
        scopedIrFragments.add(irId);
        const irNode = findNodeByFragment(irId);
        scopedIrRefs.push(asFlowRef(`ir:${irId}`, irId, irNode?.node_id ?? null));
      }
    }

    const outputNormalized = new Set(asStringArray(currentOutputs.fragment_ids));
    const normalizedRefs: TransformationFlowRef[] = [];
    if (canShowNormalize) {
      const normalizedEdgeCandidates = lineageEdges.filter((edge) => {
        const toNode = lineageNodes.get(edge.to);
        if (!toNode) return false;
        const toFragment = toNode.fragment_id ?? toNode.node_id;
        const recordType = asString(toNode.meta?.record_type) ?? '';
        const isNormalized = toNode.kind === 'normalized' || recordType.includes('normalized') || outputNormalized.has(toFragment);
        if (!isNormalized) return false;
        if (scopedIrFragments.size === 0) return true;
        const fromNode = lineageNodes.get(edge.from);
        const fromFragment = fromNode?.fragment_id ?? fromNode?.node_id ?? '';
        return scopedIrFragments.has(fromFragment);
      });
      const seenNormalized = new Set<string>();
      for (const edge of normalizedEdgeCandidates) {
        const toNode = lineageNodes.get(edge.to);
        if (!toNode) continue;
        const toFragment = toNode.fragment_id ?? toNode.node_id;
        if (seenNormalized.has(toFragment)) continue;
        seenNormalized.add(toFragment);
        normalizedRefs.push(asFlowRef(`normalized:${toFragment}`, toFragment, toNode.node_id));
      }
    }

    const mapStepId = 'flow:step:map';
    const liftStepId = 'flow:step:lift';
    const normalizeStepId = 'flow:step:normalize';
    let previousNodeId = artifactFlowId;

    if (canShowMap) {
      upsertFlowNode({
        id: mapStepId,
        label: 'Map Surface Fragments',
        caption: `${scopedSurfaceRefs.length} surface fragment${scopedSurfaceRefs.length === 1 ? '' : 's'}`,
        nodeType: 'step',
        kind: 'operation',
        stage: 'surface',
        summary: 'Map source content into surface fragments.',
        state: scopedSurfaceRefs.length > 0 ? 'complete' : 'neutral',
      });
      addFlowEdge(previousNodeId, mapStepId);
      addStepDetail({
        id: mapStepId,
        title: 'Map Surface Fragments',
        subtitle: selectedEvent.step_name === TRANSITIONS.MAP ? 'Current step' : 'Earlier transformation stage',
        summary: scopedSurfaceRefs.length > 0
          ? `Mapped the source artifact into ${scopedSurfaceRefs.length} surface fragment${scopedSurfaceRefs.length === 1 ? '' : 's'}.`
          : 'No mapped surface fragments are currently in scope.',
        inputs: [asFlowRef(artifactFlowId, artifactFileName, artifactNodeId ?? selectedNode.node_id)],
        outputs: scopedSurfaceRefs,
      });
      previousNodeId = mapStepId;
    }

    if (canShowLift) {
      upsertFlowNode({
        id: liftStepId,
        label: 'Lift Claims',
        caption: `${scopedIrRefs.length} IR fragment${scopedIrRefs.length === 1 ? '' : 's'}`,
        nodeType: 'step',
        kind: 'operation',
        stage: 'ir',
        summary: 'Lift structured IR from mapped surface fragments.',
        state: scopedIrRefs.length > 0 ? 'complete' : surfaceOnlyRefs.length > 0 ? 'attention' : 'neutral',
      });
      addFlowEdge(previousNodeId, liftStepId);
      addStepDetail({
        id: liftStepId,
        title: 'Lift Claims',
        subtitle: selectedEvent.step_name === TRANSITIONS.LIFT ? 'Current step' : 'Earlier transformation stage',
        summary: scopedIrRefs.length > 0
          ? `Lifted ${scopedIrRefs.length} IR fragment${scopedIrRefs.length === 1 ? '' : 's'} from mapped surface content.`
          : surfaceOnlyRefs.length > 0
            ? 'Some mapped surfaces were kept as surface-only and did not produce IR fragments.'
            : 'No IR fragments are currently in scope.',
        inputs: scopedSurfaceRefs,
        outputs: [...scopedIrRefs, ...surfaceOnlyRefs],
      });
      previousNodeId = liftStepId;
    }

    if (canShowNormalize) {
      upsertFlowNode({
        id: normalizeStepId,
        label: 'Normalize Claims',
        caption: `${normalizedRefs.length} normalized fragment${normalizedRefs.length === 1 ? '' : 's'}`,
        nodeType: 'step',
        kind: 'operation',
        stage: 'normalized',
        summary: 'Normalize IR into final canonical fragments.',
        state: normalizedRefs.length > 0 ? 'complete' : 'neutral',
      });
      addFlowEdge(previousNodeId, normalizeStepId);
      addStepDetail({
        id: normalizeStepId,
        title: 'Normalize Claims',
        subtitle: selectedEvent.step_name === TRANSITIONS.NORMALIZE ? 'Current step' : 'Earlier transformation stage',
        summary: normalizedRefs.length > 0
          ? `Normalized ${normalizedRefs.length} fragment${normalizedRefs.length === 1 ? '' : 's'} from IR.`
          : 'No normalized fragments are currently in scope.',
        inputs: scopedIrRefs,
        outputs: normalizedRefs,
      });
    }

    if (canShowNormalize) {
      const outputNormalized = new Set(asStringArray(currentOutputs.fragment_ids));
      const normalizedEdgeCandidates = lineageEdges.filter((edge) => {
        const toNode = lineageNodes.get(edge.to);
        if (!toNode) return false;
        const toFragment = toNode.fragment_id ?? toNode.node_id;
        const recordType = asString(toNode.meta?.record_type) ?? '';
        const isNormalized = toNode.kind === 'normalized' || recordType.includes('normalized') || outputNormalized.has(toFragment);
        if (!isNormalized) return false;
        if (scopedIrFragments.size === 0) return true;
        const fromNode = lineageNodes.get(edge.from);
        const fromFragment = fromNode?.fragment_id ?? fromNode?.node_id ?? '';
        return scopedIrFragments.has(fromFragment);
      });

      for (const edge of normalizedEdgeCandidates) {
        const fromNode = lineageNodes.get(edge.from);
        const toNode = lineageNodes.get(edge.to);
        if (!toNode) continue;
        const fromFragment = fromNode?.fragment_id ?? fromNode?.node_id ?? edge.from;
        const toFragment = toNode.fragment_id ?? toNode.node_id;
        const fromFlowId = fromNode?.kind === 'surface'
          ? `flow:surface:${fromNode.node_id}`
          : `flow:ir:${fromNode?.node_id ?? edge.from}`;
        if (!flowNodeMap.has(fromFlowId)) {
          upsertFlowNode({
            id: fromFlowId,
            label: fromFragment,
            kind: fromNode?.kind === 'surface' ? 'surface' : 'ir',
            stage: fromNode?.kind === 'surface' ? 'surface' : 'ir',
            summary: fromNode?.kind === 'surface' ? 'Surface fragment in flow scope.' : 'IR fragment in flow scope.',
            state: 'neutral',
            linkedNodeId: fromNode?.node_id,
          });
        }
        const toFlowId = `flow:normalized:${toNode.node_id}`;
        upsertFlowNode({
          id: toFlowId,
          label: toFragment,
          kind: 'normalized',
          stage: 'normalized',
          summary: 'Normalized fragment emitted from IR.',
          state: 'complete',
          linkedNodeId: toNode.node_id,
        });
        addFlowEdge(fromFlowId, toFlowId);
      }
    }

    const flowNodes = Array.from(flowNodeMap.values());
    if (flowNodes.length === 0) {
      return null;
    }

    
    return {
      artifactLabel,
      selectedLabel: nodeLabel,
      nodes: flowNodes,
      edges: flowEdges,
      stepDetails,
    };
  }, [
    childrenByNode,
    currentOutputs,
    lineageEdges,
    lineageNodes,
    liftTransformationBySurface,
    parentByChild,
    selectedEvent,
    selectedLiftTransformation,
    pipelineSteps,
    selectedNode,
  ]);

  const selectedNodePreviewState = useMemo<{ preview: ArtifactPreviewResponse | null; source: string }>(() => {
    if (!selectedNode) return { preview: null, source: 'none' };
    const mapNodeKind = asString(selectedNode.meta?.kind);
    const isMapNode = selectedNode.kind === 'map_root' || selectedNode.kind === 'map_node';

    if (selectedEvent?.step_name === TRANSITIONS.MAP && isMapNode && mapNodeKind !== 'surface_fragment') {
      const mapPayload = currentOutputs.map as Record<string, unknown> | undefined;
      const nodeSummaries = mapPayload?.node_summaries;
      if (nodeSummaries && typeof nodeSummaries === 'object') {
        const summary = asString((nodeSummaries as Record<string, unknown>)[selectedNode.node_id]);
        if (summary) {
          return {
            preview: {
              kind: 'text',
              mime_type: 'text/plain',
              file_name: selectedNode.label ?? selectedNode.node_id,
              metadata: { preview_source: 'semantic_map' },
              preview: { text: summary },
            },
            source: 'semantic_map',
          };
        }
      }
    }

    const directPreview = buildPreviewFromNode(selectedNode);
    if (directPreview) return { preview: directPreview, source: 'node_meta' };

    if (!isMapNode) return { preview: null, source: 'none' };

    const mapToSurfaceEdge = lineageEdges.find(
      (edge) => edge.from === selectedNode.node_id && edge.relation === 'map_to_surface' && edge.to.startsWith('fragment:')
    );
    if (mapToSurfaceEdge) {
      const linkedSurfaceNode = lineageNodes.get(mapToSurfaceEdge.to) ?? null;
      const linkedPreview = buildPreviewFromNode(linkedSurfaceNode);
      if (linkedPreview) return { preview: linkedPreview, source: 'surface_fragment' };
      const linkedSurfaceId = linkedSurfaceNode?.fragment_id ?? mapToSurfaceEdge.to.replace(/^fragment:/, '');
      return { preview: buildFallbackPreview(linkedSurfaceId), source: 'surface_fragment' };
    }

    const mapToArtifactEdge = lineageEdges.find(
      (edge) => edge.from === selectedNode.node_id && edge.relation === 'map_to_artifact' && edge.to.startsWith('artifact:')
    );
    if (mapToArtifactEdge) {
      const linkedArtifactNode = lineageNodes.get(mapToArtifactEdge.to) ?? null;
      const linkedPreview = buildPreviewFromNode(linkedArtifactNode);
      if (linkedPreview) return { preview: linkedPreview, source: 'artifact' };
    }

    if (selectedEvent?.step_name === TRANSITIONS.MAP) {
      const mapPayload = currentOutputs.map as Record<string, unknown> | undefined;
      const structuralMap = mapPayload?.structural_map;
      if (structuralMap && typeof structuralMap === 'object') {
        return {
          preview: {
            kind: 'json',
            mime_type: 'application/vnd.ikam.structural-map+json',
            file_name: selectedNode.label ?? selectedNode.node_id,
            metadata: {},
            preview: { parsed: structuralMap as Record<string, unknown> },
          },
          source: 'structural_map',
        };
      }
    }

    return { preview: null, source: 'none' };
  }, [currentOutputs, lineageEdges, lineageNodes, selectedEvent?.step_name, selectedNode]);

  const selectedNodePreview = selectedNodePreviewState?.preview ?? null;
  const selectedPreviewSource = selectedNodePreviewState?.source ?? 'none';
  const selectedMapContext = useMemo(() => {
    if (!selectedNode || selectedEvent?.step_name !== TRANSITIONS.MAP) {
      return { constituents: [] as string[], relationships: [] as Array<{ type: string; source: string; target: string }> };
    }
    const mapPayload = currentOutputs.map as Record<string, unknown> | undefined;
    const nodeConstituents = (mapPayload?.node_constituents as Record<string, unknown> | undefined) ?? {};
    const relationshipsRaw = (mapPayload?.relationships as unknown[]) ?? [];

    const constituents = Array.isArray(nodeConstituents[selectedNode.node_id])
      ? (nodeConstituents[selectedNode.node_id] as unknown[]).filter((item): item is string => typeof item === 'string')
      : [];
    const relationships = relationshipsRaw
      .filter((item): item is Record<string, unknown> => typeof item === 'object' && item !== null)
      .map((item) => ({
        type: asString(item.type) ?? '',
        source: asString(item.source) ?? '',
        target: asString(item.target) ?? '',
      }))
      .filter((item) => item.source === selectedNode.node_id || item.target === selectedNode.node_id);

    return { constituents, relationships };
  }, [currentOutputs, selectedEvent?.step_name, selectedNode]);

  const displayNodePreview = useMemo<ArtifactPreviewResponse | null>(() => {
    if (!selectedNodePreview) return null;
    if (selectedPreviewSource !== 'semantic_map') return selectedNodePreview;
    const text = asString((selectedNodePreview.preview as Record<string, unknown> | undefined)?.text);
    if (!text) return selectedNodePreview;
    const rendered = showFullMapSummary ? text : clampPreviewText(text);
    return {
      ...selectedNodePreview,
      preview: {
        ...(selectedNodePreview.preview as Record<string, unknown>),
        text: rendered,
      },
    };
  }, [selectedNodePreview, selectedPreviewSource, showFullMapSummary]);

  useEffect(() => {
    setShowFullMapSummary(false);
  }, [selectedNodeId]);

  useEffect(() => {
    let cancelled = false;
    const loadPreview = async () => {
      if (!selectedNode || selectedNode.kind !== 'artifact' || !activeRun) {
        setArtifactPreview(null);
        return;
      }
      const artifactId = selectedNode.fragment_id ?? selectedNode.node_id.replace(/^artifact:/, '');
      try {
        const payload = await getArtifactPreview({ runId: activeRun.run_id, artifactId });
        if (cancelled) return;
        setArtifactPreview(payload);
      } catch {
        if (cancelled) return;
        setArtifactPreview(null);
      }
    };
    void loadPreview();
    return () => {
      cancelled = true;
    };
  }, [activeRun, selectedNode]);

  useEffect(() => {
    let cancelled = false;
    const loadExplorerPreview = async () => {
      if (!activeRun || !selectedExplorerItem || selectedExplorerItem.loading) {
        return;
      }
      const requestId = explorerPreviewRequestRef.current + 1;
      explorerPreviewRequestRef.current = requestId;
      const stepIdAtRequest = selectedStepIdRef.current;
      const artifactId = inspectionArtifactId(selectedExplorerItem.inspection);
      if (!artifactId) {
        setSelectedExplorerItem((current) => (current ? { ...current, artifactPreview: null } : current));
        return;
      }
      try {
        const payload = await getArtifactPreview({ runId: activeRun.run_id, artifactId });
        if (cancelled || explorerPreviewRequestRef.current !== requestId || selectedStepIdRef.current !== stepIdAtRequest) return;
        setSelectedExplorerItem((current) => (
          current
            && current.itemId === selectedExplorerItem.itemId
            && current.section === selectedExplorerItem.section
            && explorerPreviewRequestRef.current === requestId
            ? { ...current, artifactPreview: payload }
            : current
        ));
      } catch {
        if (cancelled || explorerPreviewRequestRef.current !== requestId || selectedStepIdRef.current !== stepIdAtRequest) return;
        setSelectedExplorerItem((current) => (
          current
            && current.itemId === selectedExplorerItem.itemId
            && current.section === selectedExplorerItem.section
            && explorerPreviewRequestRef.current === requestId
            ? { ...current, artifactPreview: null }
            : current
        ));
      }
    };
    void loadExplorerPreview();
    return () => {
      cancelled = true;
    };
  }, [activeRun, selectedExplorerItem?.inspection, selectedExplorerItem?.itemId, selectedExplorerItem?.loading, selectedExplorerItem?.section]);

  useEffect(() => {
    let cancelled = false;

    const loadExecutionContext = async () => {
      if (!activeRun || !pipelineContext || !selectedEnvironmentContext) {
        setEnvironmentSummary(null);
        setScopedFragments([]);
        return;
      }

      try {
        const [environmentPayload, scopedPayload] = await Promise.all([
          getEnvironmentSummary({
            runId: activeRun.run_id,
            pipelineId: pipelineContext.pipelineId,
            pipelineRunId: pipelineContext.pipelineRunId,
            envType: selectedEnvironmentContext.envType,
            envId: selectedEnvironmentContext.envId,
          }),
          getScopedFragments({
            runId: activeRun.run_id,
            pipelineId: pipelineContext.pipelineId,
            pipelineRunId: pipelineContext.pipelineRunId,
            envType: selectedEnvironmentContext.envType,
            envId: selectedEnvironmentContext.envId,
            stepId: selectedEnvironmentContext.stepId,
            attemptIndex: selectedEnvironmentContext.attemptIndex,
          }),
        ]);

        if (cancelled) return;
        setEnvironmentSummary(environmentPayload?.status === 'ok'
          ? ((environmentPayload.summary as EnvironmentSummaryState | undefined) ?? null)
          : null);
        setScopedFragments(scopedPayload?.status === 'ok'
          ? ((scopedPayload.fragments as ScopedFragmentSummary[] | undefined) ?? [])
          : []);
      } catch {
        if (cancelled) return;
        setEnvironmentSummary(null);
        setScopedFragments([]);
      }
    };

    void loadExecutionContext();
    return () => {
      cancelled = true;
    };
  }, [activeRun, pipelineContext, selectedEnvironmentContext]);

  const streamMetricLogs = useMemo(() => {
    const metrics = selectedEvent?.metrics;
    if (!metrics || typeof metrics !== 'object') {
      return { legacy: emptyLogs(), split: null as { executor: LogBundle | null; system: LogBundle | null } | null };
    }
    const executor = normalizeLogs((metrics as Record<string, unknown>).executor_logs);
    const system = normalizeLogs((metrics as Record<string, unknown>).system_logs);
    const legacy = normalizeLogs((metrics as Record<string, unknown>).logs) ?? emptyLogs();
    return {
      legacy,
      split: hasLogs(executor) || hasLogs(system) ? { executor, system } : null,
    };
  }, [selectedEvent]);

  const traceLogs = useMemo(() => {
    const detailLogs = normalizeLogs(stepDetailPayload?.logs);
    if (hasLogs(detailLogs)) {
      return detailLogs;
    }
    return streamMetricLogs.legacy;
  }, [stepDetailPayload, streamMetricLogs]);

  const splitStepLogs = useMemo(() => {
    const detailExecutor = normalizeLogs(stepDetailPayload?.executor_logs);
    const detailSystem = normalizeLogs(stepDetailPayload?.system_logs);
    if (hasLogs(detailExecutor) || hasLogs(detailSystem)) {
      return { executor: detailExecutor, system: detailSystem };
    }
    return streamMetricLogs.split;
  }, [stepDetailPayload, streamMetricLogs]);

  const terminalLines = useMemo(
    () => buildTerminalLines(splitStepLogs, traceLogs, normalizeLogEvents(stepDetailPayload?.log_events)),
    [splitStepLogs, stepDetailPayload, traceLogs]
  );

  const terminalState = selectedEvent?.status === 'running' || executionState === 'running' ? 'streaming' : 'idle';

  useEffect(() => {
    const terminal = document.querySelector('[data-testid="executor-log-terminal"]');
    if (!terminal) return;
    if (typeof (terminal as HTMLElement).scrollIntoView === 'function') {
      (terminal as HTMLElement).scrollIntoView({ block: 'end' });
    }
  }, [splitStepLogs, traceLogs]);

  const agentReview = useMemo(() => {
    const mapPayload = currentOutputs.map;
    if (!mapPayload || typeof mapPayload !== 'object') return null;
    const review = (mapPayload as Record<string, unknown>).agent_review;
    return review && typeof review === 'object' ? (review as Record<string, unknown>) : null;
  }, [currentOutputs]);

  const elicitation = useMemo(() => {
    const mapPayload = currentOutputs.map;
    if (!mapPayload || typeof mapPayload !== 'object') return null;
    const value = (mapPayload as Record<string, unknown>).elicitation;
    return value && typeof value === 'object' ? (value as Record<string, unknown>) : null;
  }, [currentOutputs]);

  const agentSpec = useMemo(() => {
    const mapPayload = currentOutputs.map;
    if (!mapPayload || typeof mapPayload !== 'object') return null;
    const value = (mapPayload as Record<string, unknown>).agent_spec;
    return value && typeof value === 'object' ? (value as Record<string, unknown>) : null;
  }, [currentOutputs]);

  const operationTelemetry = useMemo(() => {
    const value = currentOutputs.operation_telemetry;
    return value && typeof value === 'object' ? (value as Record<string, unknown>) : null;
  }, [currentOutputs]);

  const contextSummary = useMemo(() => {
    if (!selectedEvent) return null;
    const environmentName = selectedEnvironmentContext?.envId ?? environmentSummary?.env_id ?? selectedScopeRef ?? 'n/a';
    const processName = pipelineContext?.pipelineId ?? activeRun?.pipeline_id ?? 'n/a';
    const stepTime = selectedStepDuration != null ? `${selectedStepDuration} ms` : 'n/a';
    const executorKind = selectedEvent.executor_kind ?? stepTrace?.executorKind ?? environmentSummary?.executors_seen?.[0] ?? 'n/a';
    const executorId = selectedEvent.executor_id ?? stepTrace?.executorId ?? asString(operationTelemetry?.executor_id) ?? 'n/a';
    return {
      environmentName,
      processName,
      stepName: selectedEvent.step_name,
      stepStatus: selectedStepStatus,
      stepTime,
      executorInfo: `${executorKind} · ${executorId}`,
      inputFragments: joinOrFallback(contextInputFragments),
      outputFragments: joinOrFallback(contextOutputFragments),
    };
  }, [activeRun?.pipeline_id, contextInputFragments, contextOutputFragments, environmentSummary?.env_id, environmentSummary?.executors_seen, operationTelemetry, pipelineContext?.pipelineId, selectedEnvironmentContext?.envId, selectedEvent, selectedScopeRef, selectedStepDuration, selectedStepStatus, stepTrace?.executorId, stepTrace?.executorKind]);

  const whyText = useMemo(() => {
    if (!selectedEvent) return 'Select a debug step to view details.';
    if (selectedEvent.error?.reason) return `Step failed: ${selectedEvent.error.reason}`;
    if (canonicalDetail && canonicalDetail.schemaValid) {
      const why = canonicalDetail.payload.why as Record<string, unknown> | undefined;
      const summary = why && typeof why.summary === 'string' ? why.summary : null;
      if (summary) {
        return summary;
      }
    }
    if (stepDetailPayload) {
      const reason = summarizeStepReason(selectedEvent.step_name, stepDetailPayload);
      if (reason) {
        return reason;
      }
    }
    const details = selectedEvent.metrics?.details;
    if (details && typeof details === 'object') {
      const keys = Object.keys(details as Record<string, unknown>);
      if (keys.length > 0) {
        return `Step completed using ${keys.slice(0, 3).join(', ')} from execution metrics.`;
      }
    }
    return 'Step completed without additional diagnostic details.';
  }, [selectedEvent, stepDetailPayload, canonicalDetail]);

  const transitionValidation = useMemo(() => {
    const raw = stepDetailPayload?.transition_validation;
    if (!raw || typeof raw !== 'object') {
      return null;
    }

    const results = Array.isArray((raw as Record<string, unknown>).results)
      ? ((raw as Record<string, unknown>).results as unknown[])
          .filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
          .map((item) => ({
            name: asString(item.name) ?? 'unnamed-check',
            direction: asString(item.direction),
            kind: asString(item.kind),
            status: asString(item.status) === 'passed' ? 'passed' : 'failed',
            matchedFragmentIds: asStringArray(item.matched_fragment_ids),
          }))
      : [];
    const specs = Array.isArray((raw as Record<string, unknown>).specs)
      ? ((raw as Record<string, unknown>).specs as unknown[])
          .filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
          .map((item) => {
            const config = item.config && typeof item.config === 'object' ? item.config as Record<string, unknown> : {};
            const schema = config.schema && typeof config.schema === 'object' ? config.schema as Record<string, unknown> : {};
            return {
              name: asString(item.name) ?? 'unnamed-check',
              direction: asString(item.direction),
              kind: asString(item.kind),
              selector: asString(item.selector),
              typeLabel: asString(schema.title) ?? asString(item.name)?.replace(/^(input|output)-/, '').replace(/-/g, '_') ?? 'unknown',
            };
          })
      : [];

    if (results.length === 0) {
      return null;
    }

    const contractsByDirection = new Map<'input' | 'output', Array<{ label: string; status: 'passed' | 'failed' }>>();
    for (const spec of specs) {
      const direction = spec.direction === 'input' ? 'input' : spec.direction === 'output' ? 'output' : null;
      if (!direction) continue;
      const result = results.find((item) => item.name === spec.name);
      const items = contractsByDirection.get(direction) ?? [];
      items.push({ label: `${direction}: ${spec.kind ?? 'type'}<${spec.typeLabel}>`, status: result?.status ?? 'failed' });
      contractsByDirection.set(direction, items);
    }

    const passed = results.filter((item) => item.status === 'passed').length;
    return {
      passed,
      total: results.length,
      results,
      contractSummary: `${selectedEvent?.step_name ?? 'operation'} >> ${
        (contractsByDirection.get('input') ?? []).map((item) => item.label).join(' ; ') || 'input: none'
      } :: ${
        (contractsByDirection.get('output') ?? []).map((item) => item.label).join(' ; ') || 'output: none'
      }`,
      groupedContracts: {
        input: contractsByDirection.get('input') ?? [],
        output: contractsByDirection.get('output') ?? [],
      },
      resolvedInputs: Object.fromEntries(
        Object.entries(((raw as Record<string, unknown>).resolved_inputs as Record<string, unknown> | undefined) ?? {}).map(([key, value]) => [key, asRecordArray(value)])
      ),
      resolvedOutputs: Object.fromEntries(
        Object.entries(((raw as Record<string, unknown>).resolved_outputs as Record<string, unknown> | undefined) ?? {}).map(([key, value]) => [key, asRecordArray(value)])
      ),
    };
  }, [selectedEvent?.step_name, stepDetailPayload]);

  const phasedValidation = useMemo(() => {
    const rawInput = stepDetailPayload?.input_validation;
    const rawOutput = stepDetailPayload?.output_validation;
    const parseResolved = (raw: unknown, key: 'resolved_inputs' | 'resolved_outputs') =>
      Object.fromEntries(
        Object.entries(((raw as Record<string, unknown> | undefined)?.[key] as Record<string, unknown> | undefined) ?? {}).map(([entryKey, value]) => [entryKey, asRecordArray(value)])
      );
    const parseSpecs = (raw: unknown) =>
      Array.isArray((raw as Record<string, unknown> | undefined)?.specs)
        ? ((raw as Record<string, unknown>).specs as unknown[]).filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
        : [];
    const parseResults = (raw: unknown) =>
      Array.isArray((raw as Record<string, unknown> | undefined)?.results)
        ? ((raw as Record<string, unknown>).results as unknown[]).filter((item): item is Record<string, unknown> => Boolean(item && typeof item === 'object'))
        : [];
    return {
      resolvedInputs: rawInput && typeof rawInput === 'object' ? parseResolved(rawInput, 'resolved_inputs') : {},
      resolvedOutputs: rawOutput && typeof rawOutput === 'object' ? parseResolved(rawOutput, 'resolved_outputs') : {},
      inputSpecs: parseSpecs(rawInput),
      outputSpecs: parseSpecs(rawOutput),
      inputResults: parseResults(rawInput),
      outputResults: parseResults(rawOutput),
      hasInputPhase: Boolean(rawInput && typeof rawInput === 'object'),
      hasOutputPhase: Boolean(rawOutput && typeof rawOutput === 'object'),
    };
  }, [stepDetailPayload]);

  const displayTransitionValidation = useMemo(() => {
    if (!transitionValidation) return null;
    const hasTypedPhases = phasedValidation.hasInputPhase || phasedValidation.hasOutputPhase;
    const outputNotReady = phasedValidation.hasOutputPhase && phasedValidation.outputSpecs.length > 0 && phasedValidation.outputResults.length === 0;
    if (!hasTypedPhases || (!outputNotReady && phasedValidation.hasOutputPhase)) return transitionValidation;
    const outputContracts = phasedValidation.outputSpecs.length > 0
      ? phasedValidation.outputSpecs.map((spec) => ({ label: `output: not_ready<${asString(((spec.config as Record<string, unknown> | undefined)?.schema as Record<string, unknown> | undefined)?.title) ?? asString(spec.name)?.replace(/^output-/, '').replace(/-/g, '_') ?? 'unknown'}>`, status: 'not_ready' as const }))
      : transitionValidation.groupedContracts.output.length > 0
        ? transitionValidation.groupedContracts.output
        : [{ label: 'output: not_ready', status: 'not_ready' as const }];
    const contractSummary = `${selectedEvent?.step_name ?? 'operation'} >> ${
      (transitionValidation.groupedContracts.input ?? []).map((item) => item.label).join(' ; ') || 'input: none'
    } :: ${
      outputContracts.map((item) => item.label).join(' ; ') || 'output: not_ready'
    }`;
    return {
      ...transitionValidation,
      total: transitionValidation.groupedContracts.input.length + outputContracts.length,
      groupedContracts: {
        input: transitionValidation.groupedContracts.input,
        output: outputContracts,
      },
      contractSummary,
    };
  }, [phasedValidation, selectedEvent?.step_name, transitionValidation]);

  const contextInputItems = useMemo<FragmentListItem[]>(() => {
    const resolvedInputs = phasedValidation.hasInputPhase
      ? phasedValidation.resolvedInputs
      : (transitionValidation?.resolvedInputs ?? {});
    const items = Object.entries(resolvedInputs).flatMap(([sourceKey, entries]) =>
      entries.map((entry) => {
        const value = entry.value && typeof entry.value === 'object' ? entry.value as Record<string, unknown> : null;
        const typedKind = asString(value?.kind);
        const primary = typedKind
          ?? asString(value?.document_id)
          ?? asString(entry.fragment_id)
          ?? asString(entry.cas_id)
          ?? sourceKey;
        const detail = asString(value?.subgraph_ref)
          ?? asString(value?.location)
          ?? asString(value?.filename)
          ?? asString(entry.mime_type)
          ?? sourceKey;
        return {
          id: `${sourceKey}:${primary}`,
          display: detail && detail !== primary ? `${primary} · ${detail}` : primary,
          search: `${sourceKey} ${primary} ${detail}`.toLowerCase(),
          inspection: parseInspection(entry.inspection),
          inspectionRef: parseInspectionRef(entry.inspection_stub),
        };
      })
    );
    if (items.length > 0) return items;
    return contextInputFragments.map((fragmentId) => ({
        id: fragmentId,
        display: fragmentId,
        search: fragmentId.toLowerCase(),
        inspection: null,
        inspectionRef: null,
      }));
  }, [contextInputFragments, phasedValidation, transitionValidation?.resolvedInputs]);

  const contextOutputItems = useMemo<FragmentListItem[]>(() => {
    const resolvedOutputs = phasedValidation.hasOutputPhase
      ? phasedValidation.resolvedOutputs
      : (transitionValidation?.resolvedOutputs ?? {});
    const items = Object.entries(resolvedOutputs).flatMap(([sourceKey, entries]) =>
      entries.map((entry) => {
        const value = entry.value && typeof entry.value === 'object' ? entry.value as Record<string, unknown> : null;
        const typedKind = asString(value?.kind);
        const primary = typedKind
          ?? asString(value?.document_id)
          ?? asString(value?.segment_id)
          ?? asString(entry.fragment_id)
          ?? asString(entry.cas_id)
          ?? sourceKey;
        const detail = asString(value?.subgraph_ref)
          ?? asString(value?.filename)
          ?? asString(entry.mime_type)
          ?? sourceKey;
        return {
          id: `${sourceKey}:${primary}`,
          display: detail && detail !== primary ? `${primary} · ${detail}` : primary,
          search: `${sourceKey} ${primary} ${detail}`.toLowerCase(),
          inspection: parseInspection(entry.inspection),
          inspectionRef: parseInspectionRef(entry.inspection_stub),
        };
      })
    );
    if (items.length > 0) return items;
    return contextOutputFragments.map((fragmentId) => ({
        id: fragmentId,
        display: fragmentId,
        search: fragmentId.toLowerCase(),
        inspection: null,
        inspectionRef: null,
      }));
  }, [contextOutputFragments, phasedValidation, transitionValidation?.resolvedOutputs]);

  const explorerAncestryForSelection = (selection: ExplorerSelection): ConnectedFragmentItem[] => [
    {
      id: `previous:${selection.section}:${selection.itemId}`,
      label: selection.label,
      ref: selectionInspectionRef(selection),
      itemId: selection.itemId,
      inspection: selection.inspection,
      inspectionRef: selection.inspectionRef,
      section: selection.section,
      isPrevious: true,
    },
    ...(selection.ancestry ?? []),
  ];

  const mergeConnectedFragments = (
    connected: ConnectedFragmentItem[],
    ancestry: ConnectedFragmentItem[]
  ): ConnectedFragmentItem[] => {
    const merged = [...connected];
    const seen = new Set(connected.map((item) => item.ref));
    for (const item of ancestry) {
      const key = item.ref;
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      merged.push(item);
    }
    return merged;
  };

  const openInspection = async (section: DrillSection, item: FragmentListItem) => {
    const requestId = explorerInspectionRequestRef.current + 1;
    explorerInspectionRequestRef.current = requestId;
    const stepIdAtRequest = selectedStepIdRef.current;
    const previousSelection = selectedExplorerItem;
    const hydratedInspectionRef = item.inspectionRef
      ?? (() => {
        const subgraphRef = asString(item.inspection?.content.subgraph_ref);
        return subgraphRef ? `inspect://subgraph/${subgraphRef}` : null;
      })();
    const needsHydration = Boolean(item.inspection && !item.inspection.subgraph && hydratedInspectionRef);
    const baseEntry: ExplorerSelection = {
      section,
      itemId: item.id,
      label: item.display,
      inspection: item.inspection,
      inspectionRef: item.inspectionRef,
      artifactPreview: null,
      loading: !item.inspection && Boolean(hydratedInspectionRef),
      showRaw: false,
      ancestry: previousSelection ? explorerAncestryForSelection(previousSelection) : [],
    };
    setSelectedExplorerItem(baseEntry);
    if ((!needsHydration && item.inspection) || !hydratedInspectionRef || !activeRun) {
      return;
    }
    try {
      const payload = await getInspectionSubgraph({ runId: activeRun.run_id, ref: hydratedInspectionRef, maxDepth: 1 });
      const inspection = parseInspectionResponse(payload);
      if (explorerInspectionRequestRef.current !== requestId || selectedStepIdRef.current !== stepIdAtRequest) {
        return;
      }
      setSelectedExplorerItem((current) => (
        current && current.itemId === item.id && current.section === section && explorerInspectionRequestRef.current === requestId
          ? { ...current, inspection, artifactPreview: null, loading: false }
          : current
      ));
    } catch {
      if (explorerInspectionRequestRef.current !== requestId || selectedStepIdRef.current !== stepIdAtRequest) {
        return;
      }
      setSelectedExplorerItem((current) => (
        current && current.itemId === item.id && current.section === section && explorerInspectionRequestRef.current === requestId
          ? { ...current, artifactPreview: null, loading: false }
          : current
      ));
    }
  };

  const renderFragmentList = (section: DrillSection, items: FragmentListItem[]) => (
    <ul className="runs-fragment-list">
      {items.map((item) => {
        const clickable = Boolean(item.inspection || item.inspectionRef);
        const selected = selectedExplorerItem?.section === section && selectedExplorerItem.itemId === item.id;
        return (
          <li key={item.id} className={selected ? 'runs-fragment-list-item-active' : ''}>
            {clickable ? (
              <button type="button" className="runs-fragment-row-button" onClick={() => void openInspection(section, item)}>
                <span>{item.display}</span>
              </button>
            ) : (
              <div className="runs-fragment-row">
                <span>{item.display}</span>
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );

  const connectedFragments = useMemo<ConnectedFragmentItem[]>(() => {
    if (!selectedExplorerItem) {
      return [];
    }
    const resolvedConnected = (selectedExplorerItem.inspection?.resolvedRefs ?? [])
      .map((value) => {
        const ref = connectedItemRef(value);
        if (!ref) return null;
        const fragmentId = asString(value.fragment_id) ?? asString(value.cas_id) ?? ref;
        return {
          id: `${selectedExplorerItem.itemId}:${fragmentId}`,
          label: connectedItemLabel(value),
          secondaryLabel: connectedItemLabel(value) === fragmentId ? undefined : fragmentId,
          ref,
          itemId: fragmentId,
          inspection: null,
          inspectionRef: asString(value.inspection_ref) ?? ref,
          section: selectedExplorerItem.section,
        } satisfies ConnectedFragmentItem;
      })
      .filter((item): item is ConnectedFragmentItem => Boolean(item));
    const resolvedIds = new Set(resolvedConnected.map((item) => item.itemId ?? item.ref));
    const unresolvedConnected = (selectedExplorerItem.inspection?.refs ?? [])
      .filter((ref) => !resolvedIds.has(ref))
      .map((ref) => ({
          id: `${selectedExplorerItem.itemId}:${ref}`,
          label: ref,
          secondaryLabel: undefined,
          ref,
          itemId: ref,
          inspection: null,
          inspectionRef: ref,
          section: selectedExplorerItem.section,
        }));
    return mergeConnectedFragments([...resolvedConnected, ...unresolvedConnected], selectedExplorerItem.ancestry ?? []);
  }, [selectedExplorerItem]);

  const openConnectedFragment = async (item: ConnectedFragmentItem) => {
    if (!selectedExplorerItem) {
      return;
    }
    await openInspection(item.section ?? selectedExplorerItem.section, {
      id: item.itemId ?? item.ref,
      display: item.label,
      search: item.label.toLowerCase(),
      inspection: item.inspection ?? null,
      inspectionRef: item.inspectionRef ?? item.ref,
    });
  };

  const openRunGraphNode = async (selection: RunFragmentGraphNodeSelection) => {
    const connectedMatch = connectedFragments.find((item) => (
      (selection.inspectionRef && (item.inspectionRef ?? item.ref) === selection.inspectionRef)
      || (selection.fragmentId && ((item.itemId ?? null) === selection.fragmentId || item.ref === selection.fragmentId || (item.inspectionRef ?? item.ref) === `inspect://fragment/${selection.fragmentId}`))
      || item.ref === selection.id
      || (item.inspectionRef ?? item.ref) === selection.id
    ));
    if (connectedMatch) {
      await openConnectedFragment(connectedMatch);
    }
  };

  const openInspectionGraphNode = async (selection: InspectionGraphNodeSelection) => {
    if (!selectedExplorerItem) {
      return;
    }
    const connectedMatch = connectedFragments.find((item) => {
      const inspectionRef = item.inspectionRef ?? item.ref;
      return (
        (selection.inspectionRef && inspectionRef === selection.inspectionRef)
        || (selection.fragmentId && ((item.itemId ?? null) === selection.fragmentId || item.ref === selection.fragmentId || inspectionRef === `inspect://fragment/${selection.fragmentId}`))
        || item.ref === selection.id
        || inspectionRef === selection.id
      );
    });
    if (connectedMatch) {
      await openConnectedFragment(connectedMatch);
    }
  };

  const renderExplorerDetail = (entry: ExplorerSelection | null) => {
    if (!entry) {
      return <p className="runs-fragment-empty">Select an input or output to explore.</p>;
    }
    const inspectionPreview = entry.artifactPreview ?? buildInspectionPreview(entry.inspection);
    const inspectionText = formatInspectionText(entry.inspection);
    const inspectionFields = inspectionFieldsForEntry(entry);
    const inspectionGraph = asInspectionSubgraph(entry.inspection);
    const runGraphInspections = [entry.inspection?.subgraph, ...connectedFragments.map((item) => item.inspection?.subgraph)]
      .filter((value): value is InspectionSubgraphResponse => Boolean(value));
    const runGraphFocusNodeId = entry.inspection?.subgraph?.root_node_id
      ?? inspectionGraph?.root_node_id
      ?? runGraphInspections.find((item) => typeof item.root_node_id === 'string')?.root_node_id
      ?? null;
    return (
      <div className="runs-fragment-drill-card">
        <div className="runs-fragment-explorer-breadcrumbs" aria-label="Fragment explorer breadcrumb">
          <span className="runs-fragment-drill-breadcrumb">{entry.section === 'input' ? 'Input' : 'Output'}</span>
          <span className="runs-fragment-drill-breadcrumb">/</span>
          <span className="runs-fragment-drill-breadcrumb">{entry.label}</span>
        </div>
        <strong className="runs-fragment-drill-summary">{entry.inspection?.summary ?? entry.label}</strong>
        {!entry.loading && inspectionFields.length > 0 ? renderInspectionFields(inspectionFields) : null}
        {entry.loading ? <p className="runs-fragment-empty">Loading inspection...</p> : null}
        {!entry.loading ? (
          <section className="runs-fragment-drill-links">
            <div className="runs-fragment-drill-links-head">
              <h6>Connected Fragments</h6>
            </div>
            {connectedFragments.length > 0 ? (
              <ul className="runs-fragment-drill-links-list runs-fragment-drill-links-list-scroll" data-testid="connected-fragments-list">
                {connectedFragments.map((item) => (
                  <li key={`${item.id}:${item.ref}:${item.isPrevious ? 'previous' : 'linked'}`}>
                    <button
                      type="button"
                      className={`runs-fragment-drill-link${item.isPrevious ? ' runs-fragment-drill-link-previous' : ''}`}
                      onClick={() => void openConnectedFragment(item)}
                    >
                      <span className="runs-fragment-drill-link-primary">{item.label}</span>
                      {item.secondaryLabel ? <span className="runs-fragment-drill-link-secondary">{item.secondaryLabel}</span> : null}
                    </button>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="runs-fragment-empty">No connected fragments</p>
            )}
          </section>
        ) : null}
        {!entry.loading && runGraphFocusNodeId ? (
          <RunFragmentGraphPanel
            inspections={runGraphInspections}
            focusNodeId={runGraphFocusNodeId}
            onSelectNode={(node) => void openRunGraphNode(node)}
          />
        ) : !entry.loading && inspectionGraph ? (
          <InspectionGraphPanel inspection={inspectionGraph} onSelectNode={(node) => void openInspectionGraphNode(node)} />
        ) : null}
        {inspectionPreview ? (
          <section className="runs-fragment-drill-preview">
            <div className="runs-fragment-drill-preview-head">
              <h6>Element Preview</h6>
            </div>
            <div className="runs-fragment-drill-preview-body">
              <ContentViewer preview={inspectionPreview} />
            </div>
          </section>
        ) : null}
        {!entry.loading && inspectionText ? (
          <section className="runs-fragment-drill-raw">
            <button
              type="button"
              className="runs-fragment-drill-raw-toggle"
              onClick={() => setSelectedExplorerItem((current) => (current ? { ...current, showRaw: !current.showRaw } : current))}
            >
              {entry.showRaw ? 'Hide Raw' : 'Show Raw'}
            </button>
            {entry.showRaw ? (
              <div className="runs-fragment-drill-raw-body">
                <h6>Raw Payload</h6>
                <pre className="runs-fragment-drill-payload">{inspectionText}</pre>
              </div>
            ) : null}
          </section>
        ) : null}
        {!entry.loading && !inspectionPreview && !inspectionText ? <p className="runs-fragment-empty">Inspection unavailable</p> : null}
      </div>
    );
  };

  const filteredInputFragments = useMemo(() => {
    const query = inputFragmentSearch.trim().toLowerCase();
    if (!query) return contextInputItems;
    return contextInputItems.filter((item) => item.search.includes(query));
  }, [contextInputItems, inputFragmentSearch]);

  const filteredOutputFragments = useMemo(() => {
    const query = outputFragmentSearch.trim().toLowerCase();
    if (!query) return contextOutputItems;
    return contextOutputItems.filter((item) => item.search.includes(query));
  }, [contextOutputItems, outputFragmentSearch]);

  const visibleInputFragments = useMemo(
    () => (inputFragmentsExpanded ? filteredInputFragments : filteredInputFragments.slice(0, FRAGMENT_PREVIEW_COUNT)),
    [filteredInputFragments, inputFragmentsExpanded]
  );

  const visibleOutputFragments = useMemo(
    () => (outputFragmentsExpanded ? filteredOutputFragments : filteredOutputFragments.slice(0, FRAGMENT_PREVIEW_COUNT)),
    [filteredOutputFragments, outputFragmentsExpanded]
  );

  useEffect(() => {
    if (selectedExplorerItem) {
      return;
    }
    const nextItem = [...contextOutputItems, ...contextInputItems].find((item) => item.inspection || item.inspectionRef);
    if (!nextItem) {
      return;
    }
    const section = contextOutputItems.includes(nextItem) ? 'output' : 'input';
    void openInspection(section, nextItem);
  }, [contextInputItems, contextOutputItems, selectedExplorerItem, selectedStepId]);

  const petriTransitionValidationStates = useMemo<Record<string, 'passed' | 'failed' | 'mixed'>>(() => {
    const transitionId = toPetriTransitionId(selectedEvent?.step_name);
    if (!transitionId || !transitionValidation) {
      return {};
    }
    const passed = transitionValidation.results.filter((item) => item.status === 'passed').length;
    const failed = transitionValidation.results.filter((item) => item.status === 'failed').length;
    const status = failed > 0 && passed > 0
      ? 'mixed'
      : failed > 0
        ? 'failed'
        : 'passed';
    return { [transitionId]: status };
  }, [selectedEvent?.step_name, transitionValidation]);

  const reloadDebug = async (showLoading = true, optimisticOverride: OptimisticStep | null = activeOptimisticStep): Promise<void> => {
    if (!activeRun || !pipelineContext) {
      setDebugEvents([]);
      setLoadedDebugKey(null);
      setPipelineSteps([]);
      setSelectedStepId(null);
      setOptimisticStep(null);
      setDebugStatus('idle');
      setExecutionState('idle');
      setControlAvailability({ can_resume: false, can_next_step: false });
      return;
    }

    if (showLoading) {
      setDebugStatus('loading');
    }
    if (activeDebugKey && debugReloadInFlightKeyRef.current === activeDebugKey) {
      return;
    }
    if (activeDebugKey) {
      debugReloadInFlightKeyRef.current = activeDebugKey;
    }
    try {
      const payload = await getDebugStream({
        runId: activeRun.run_id,
        pipelineId: pipelineContext.pipelineId,
        pipelineRunId: pipelineContext.pipelineRunId,
      });

      if (activeDebugKey && debugReloadInFlightKeyRef.current !== activeDebugKey) {
        return;
      }

      if (payload.status !== 'ok') {
        setDebugStatus('missing');
        setDebugEvents([]);
        setLoadedDebugKey(activeDebugKey);
        setPipelineSteps([]);
        setSelectedStepId(null);
        setOptimisticStep(null);
        setExecutionState('missing');
        setControlAvailability({ can_resume: false, can_next_step: false });
        return;
      }

      const events = payload.events ?? [];
      const derivedSteps = payload.pipeline_steps && payload.pipeline_steps.length > 0
        ? payload.pipeline_steps
        : Array.from(new Set(events.map((event) => event.step_name)));
      const nextExecutionState = payload.execution_state ?? 'ready';
      const runningEvent = events.find((event) => event.status === 'running') ?? null;
      const realOptimisticEvent = optimisticOverride
        ? (events.find(
            (event) => event.step_name === optimisticOverride.stepName && event.attempt_index >= optimisticOverride.attemptIndex
          ) ?? null)
        : null;
      const keepOptimisticRunning = Boolean(optimisticOverride && !realOptimisticEvent && (controlInFlight || executionState === 'running'));
      const effectiveExecutionState = keepOptimisticRunning ? 'running' : nextExecutionState;

      if (optimisticOverride && !keepOptimisticRunning && (realOptimisticEvent || nextExecutionState !== 'running')) {
        setOptimisticStep(null);
      }

      setDebugEvents(events);
      setLoadedDebugKey(activeDebugKey);
      setPipelineSteps(derivedSteps);
      setDebugStatus('ready');
      setExecutionMode((payload.execution_mode as 'autonomous' | 'manual') ?? executionMode);
      setExecutionState(effectiveExecutionState);
      setControlAvailability(payload.control_availability ?? { can_resume: false, can_next_step: false });
      setSelectedStepId((current) => {
        if (events.length === 0) {
          return optimisticOverride && effectiveExecutionState === 'running' ? optimisticOverride.stepId : null;
        }
        if (effectiveExecutionState === 'running' || controlInFlight) {
          if (runningEvent) {
            return runningEvent.step_id;
          }
          if (optimisticOverride && !realOptimisticEvent) {
            return optimisticOverride.stepId;
          }
        }
        if (current && optimisticOverride && current === optimisticOverride.stepId && realOptimisticEvent) {
          return realOptimisticEvent.step_id;
        }
        if (current && events.some((event) => event.step_id === current)) {
          return current;
        }
        return events[0]?.step_id ?? null;
      });
    } catch {
      if (activeDebugKey && debugReloadInFlightKeyRef.current !== activeDebugKey) {
        return;
      }
      setDebugStatus('error');
      setDebugEvents([]);
      setLoadedDebugKey(activeDebugKey);
      setPipelineSteps([]);
      setSelectedStepId(null);
      setOptimisticStep(null);
      setExecutionState('error');
      setControlAvailability({ can_resume: false, can_next_step: false });
    } finally {
      if (activeDebugKey && debugReloadInFlightKeyRef.current === activeDebugKey) {
        debugReloadInFlightKeyRef.current = null;
      }
    }
  };

  useEffect(() => {
    const loadSubgraph = async () => {
      const targetPipelineId = selectedPipelineId;
      if (debugViewMode === 'graph' && targetPipelineId) {
        try {
          const registry = await getRegistry('petri_net_runnables');
          const entry = registry.entries.find((e: any) => e.key === targetPipelineId);
          const headId = entry?.head_fragment_id || targetPipelineId;
          if (headId) {
            const data = await getSubgraph(headId);
            setPetriGraphData({ head: data.head, childrenFragments: data.children || {} });
          }
        } catch (err) {
          console.error('Failed to load subgraph data:', err);
          setPetriGraphData(null);
        }
      }
    };
    void loadSubgraph();
  }, [debugViewMode, selectedPipelineId]);

  useEffect(() => {
    void reloadDebug(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeRun?.run_id, pipelineContext?.pipelineId, pipelineContext?.pipelineRunId]);

  useEffect(() => {
    if (activeRun) {
      return;
    }
    if (running) {
      setDebugStatus('loading');
      setExecutionState('running');
      setControlAvailability({ can_resume: false, can_next_step: false });
      return;
    }
    setDebugStatus('idle');
    setExecutionState('idle');
    setControlAvailability({ can_resume: false, can_next_step: false });
    setOptimisticStep(null);
  }, [activeRun, running]);

  useEffect(() => {
    if (!activeRun || !pipelineContext) {
      return;
    }
    const shouldPoll = executionState === 'running' || controlInFlight;
    if (!shouldPoll) {
      return;
    }
    const timer = window.setInterval(() => {
      void reloadDebug(false);
    }, 600);
    return () => window.clearInterval(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeRun?.run_id, pipelineContext?.pipelineId, pipelineContext?.pipelineRunId, executionState, controlInFlight]);

  const onControl = async (
    action: 'set_mode' | 'pause' | 'resume' | 'next_step',
    mode?: 'autonomous' | 'manual'
  ) => {
    if (!activeRun || !pipelineContext) {
      return;
    }

    setControlInFlight(true);
    try {
      const response = await controlRun({
        runId: activeRun.run_id,
        commandId: `cmd-${Date.now()}-${Math.random().toString(16).slice(2)}`,
        action,
        pipelineId: pipelineContext.pipelineId,
        pipelineRunId: pipelineContext.pipelineRunId,
        mode,
      });
      if (mode) {
        setExecutionMode(mode);
      }
      const responseMode = response.state?.execution_mode;
      if (responseMode === 'manual' || responseMode === 'autonomous') {
        setExecutionMode(responseMode);
      }
      if (response.state?.execution_state) {
        setExecutionState(response.state.execution_state);
      }
      if (response.control_availability) {
        setControlAvailability(response.control_availability);
      }
      if (action === 'next_step' && response.state?.execution_state === 'running') {
        const currentStepName = response.state.current_step_name;
        const currentSelectedEvent = selectedEvent
          ?? activeRunDebugEvents.find((event) => event.step_id === selectedStepId)
          ?? activeRunDebugEvents[0]
          ?? null;
        const currentStepIndex = pipelineSteps.indexOf(currentStepName);
        const inferredNextStepName = currentStepName === currentSelectedEvent?.step_name
          ? (currentStepIndex >= 0 && currentStepIndex < pipelineSteps.length - 1
              ? pipelineSteps[currentStepIndex + 1]
              : currentStepName)
          : null;
        const optimisticStepName = inferredNextStepName
          ?? (currentStepName && currentStepName !== currentSelectedEvent?.step_name ? currentStepName : null);
        const fallbackEvent = activeRunDebugEvents[activeRunDebugEvents.length - 1] ?? activeRunDebugEvents[0] ?? selectedEvent ?? null;
        const nextEnvType = fallbackEvent?.env_type ?? 'dev';
        const nextEnvId = fallbackEvent?.env_id ?? 'optimistic';
        const nextAttemptIndex = response.state.current_attempt_index;
        const realRunningEvent = activeRunDebugEvents.find((event) => event.status === 'running');
        if (!realRunningEvent && optimisticStepName) {
          const nextOptimisticStep = {
            debugKey: activeDebugKey ?? activeRun.run_id,
            stepId: `optimistic:${optimisticStepName}:${nextAttemptIndex}`,
            stepName: optimisticStepName,
            attemptIndex: nextAttemptIndex,
            envType: nextEnvType,
            envId: nextEnvId,
          } satisfies OptimisticStep;
          setOptimisticStep(nextOptimisticStep);
          setSelectedStepId(nextOptimisticStep.stepId);
          setExecutionState('running');
          return;
        }
      }
      await reloadDebug(false);
    } finally {
      setControlInFlight(false);
    }
  };

  useEffect(() => {
    setStepDetailPayload(null);
    setStepDetailError(null);
  }, [selectedEvent?.step_id]);

  useEffect(() => {
    let cancelled = false;
    const loadStepDetail = async () => {
      if (!activeRun || !pipelineContext || !selectedEvent || selectedEvent.step_id.startsWith('optimistic:')) {
        setStepDetailPayload(null);
        setStepDetailError(null);
        return;
      }

      try {
        setStepDetailError(null);
        const stepDetail = await getDebugStepDetail({ runId: activeRun.run_id, stepId: selectedEvent.step_id });
        if (cancelled) return;
        setStepDetailPayload(stepDetail as Record<string, unknown>);
      } catch {
        if (cancelled) return;
        setStepDetailPayload(null);
        setStepDetailError('Failed to load step detail. Check backend logs for /debug-step/{step_id}/detail.');
      }
    };

    void loadStepDetail();
    if (selectedEvent?.status !== 'running' && executionState !== 'running') {
      return () => {
        cancelled = true;
      };
    }
    const timer = window.setInterval(() => {
      void loadStepDetail();
    }, 600);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeRun, pipelineContext, selectedEvent, executionState]);

  useEffect(() => {
    setExpandedNodeIds({});
    setSelectedNodeId(null);
    setFragmentSearch('');
  }, [selectedEvent?.step_id]);

  useEffect(() => {
    setIsDrillThroughCollapsed(false);
  }, [activeRunId]);

  useEffect(() => {
    if (selectedNodeId) {
      return;
    }
    if (lineageRoots.length > 0) {
      setSelectedNodeId(lineageRoots[0]);
    }
  }, [lineageRoots, selectedNodeId]);

  const copyText = async (value: string) => {
    try {
      await navigator.clipboard.writeText(value);
    } catch {
      // no-op
    }
  };
  const selectedStepExecutor = selectedEvent?.executor_id
    ?? stepTrace?.executorId
    ?? asString(boundaryTransition?.executor_id)
    ?? asString(operationTelemetry?.executor_id);
  const hasStepLogs = Boolean(splitStepLogs || traceLogs.stdout.length > 0 || traceLogs.stderr.length > 0);
  return (
    <section className="panel panel-accent runs-workspace-panel">
      {caseError ? <div className="panel-placeholder">Error: {caseError}</div> : null}
      {runError ? <div className="panel-placeholder">Error: {runError}</div> : null}

      <div className="runs-dashboard-layout runs-workbench-frame runs-workbench-frame-scroll-safe" data-testid="run-debug-layout">
        <div className="runs-dashboard-rail runs-left-rail" data-testid="runs-left-rail">
          <div className="runs-left-overlay-shell" data-testid="runs-left-overlay-shell">
            <aside className="runs-latest-navigator" data-testid="runs-latest-navigator">
            <div className="runs-dashboard-card runs-dashboard-runlist-card runs-left-overlay-pane">
              <div className="runs-dashboard-section-heading">
                <h3>Latest Runs</h3>
                <p className="panel-subtitle">Choose an execution to inspect.</p>
              </div>
              <RunTable runs={runs} activeRunId={activeRunId} onSelectRun={onSelectRun} />
            </div>

            <div className="runs-dashboard-card runs-dashboard-case-card runs-left-overlay-pane">
              <div className="run-debug-header-copy">
                <h3>Case Filter</h3>
                <p className="panel-subtitle">
                  {activeRun?.evaluation
                    ? `Oracle report for ${activeRun.case_id}`
                    : 'Run a case with an oracle fixture to see evaluation results'}
                </p>
              </div>
              <div className="run-debug-case-list" data-testid="run-debug-case-list">
                {loadingCases ? (
                  <div className="panel-placeholder">Loading cases...</div>
                ) : (
                  <CaseSelector
                    cases={cases}
                    selectedCaseIds={selectedCaseIds}
                    onToggleCase={onToggleCase}
                    disabled={running}
                  />
                )}
              </div>
            </div>
            </aside>
          </div>
        </div>

        <div className="runs-dashboard-shell run-debug-layout" data-testid="runs-dashboard-shell">
          <div className="runs-command-bar runs-dashboard-card" data-testid="runs-command-bar">
          <div className="runs-command-bar-identity">
            <span className="runs-command-bar-kicker">Active Run</span>
            <strong>{activeRun?.run_id ?? 'No active run'}</strong>
            <span>{activeRun?.case_id ?? 'Select a run to inspect runtime state'}</span>
          </div>
          <div className="runs-command-bar-config">
            <div className="runs-dashboard-pipeline-select-wrap">
              <label htmlFor="pipeline-select" className="runs-dashboard-field-label">Pipeline</label>
              <select
                id="pipeline-select"
                className="runs-dashboard-pipeline-select"
                value={selectedPipelineId}
                onChange={(e) => setSelectedPipelineId(e.target.value)}
              >
                {availablePipelines.length === 0 && <option value="">Loading pipelines...</option>}
                {availablePipelines.map((p) => (
                  <option key={p.id} value={p.id}>{p.title}</option>
                ))}
              </select>
            </div>
            <div className="runs-command-bar-mode">
              <span className="runs-dashboard-field-label">Mode</span>
              <strong>{executionMode}</strong>
            </div>
          </div>
          <div className="panel-actions run-debug-controls-stack runs-command-bar-actions" data-testid="run-debug-controls-stack">
            <label className="model-select runs-dashboard-reset-toggle">
              <input type="checkbox" checked={reset} onChange={(event) => onResetChange(event.target.checked)} />
              <span>Reset before run</span>
            </label>
            <div className="panel-actions run-debug-actions" data-testid="run-debug-actions">
              <button type="button" disabled={running || selectedCaseIds.length === 0 || controlInFlight} onClick={() => onRunCases(selectedPipelineId)}>
                Run Pipeline
              </button>
              <button type="button" onClick={() => void onControl('set_mode', 'autonomous')} disabled={!activeRun || executionMode === 'autonomous' || controlInFlight}>
                Autonomous
              </button>
              <button type="button" onClick={() => void onControl('set_mode', 'manual')} disabled={!activeRun || executionMode === 'manual' || controlInFlight}>
                Manual
              </button>
              <button type="button" onClick={() => void onControl('pause')} disabled={!activeRun || controlInFlight}>
                Pause
              </button>
              <button type="button" onClick={() => void onControl('resume')} disabled={!activeRun || !controlAvailability.can_resume || controlInFlight}>
                Resume
              </button>
              <button
                type="button"
                onClick={() => void onControl('next_step')}
                disabled={!activeRun || executionMode !== 'manual' || !controlAvailability.can_next_step || controlInFlight}
              >
                Next Step
              </button>
            </div>
          </div>
        </div>

          <div className="runs-dashboard-right runs-right-stage" data-testid="runs-right-stage">
          <div className="debug-stream-panel runs-dashboard-card" data-testid="debug-step-stream">
            <div className="runs-dashboard-stream-header">
              <div>
                <h4>Timeline</h4>
                <p className="panel-subtitle">Timeline owns selection; Petri mirrors the same step state.</p>
              </div>
              <div className="runs-dashboard-view-toggle">
                <button
                  type="button"
                  aria-pressed={debugViewMode === 'list'}
                  className={debugViewMode === 'list' ? 'runs-dashboard-view-toggle-active' : ''}
                  onClick={() => setDebugViewMode('list')}
                >
                  Timeline
                </button>
                <button
                  type="button"
                  aria-pressed={debugViewMode === 'graph'}
                  className={debugViewMode === 'graph' ? 'runs-dashboard-view-toggle-active' : ''}
                  onClick={() => setDebugViewMode('graph')}
                >
                  Petri
                </button>
              </div>
            </div>
            {debugStatus === 'loading' ? <p className="panel-subtitle">Loading debug stream...</p> : null}
            {debugStatus === 'missing' ? <p className="panel-subtitle">No debug stream available for this run.</p> : null}
            {debugStatus === 'error' ? <p className="panel-subtitle">Failed to load debug stream.</p> : null}
            {debugStatus === 'ready' && displayedDebugEvents.length === 0 ? (
              <p className="panel-subtitle">Debug stream returned no events.</p>
            ) : null}
            {displayedDebugEvents.length > 0 ? (
              debugViewMode === 'list' ? (
                <div className="debug-step-list" data-testid="debug-step-list">
                  {displayedDebugEvents.map((event) => (
                    <button
                      key={`${event.step_id}:${event.attempt_index}`}
                      type="button"
                      className={`debug-step-item ${selectedStepId === event.step_id ? 'debug-step-item-active' : ''}`}
                      onClick={() => setSelectedStepId(event.step_id)}
                    >
                      <span className="debug-step-title">{event.step_name}</span>
                      <span className="debug-step-badges">
                        <span className="debug-step-badge">A{event.attempt_index}</span>
                        {event.retry_parent_step_id ? <span className="debug-step-badge">Retry</span> : null}
                        <span className="debug-step-badge">{event.status}</span>
                      </span>
                    </button>
                  ))}
                </div>
              ) : (
                <div className="runs-dashboard-graph-stage">
                  {petriGraphData ? (
                      <PetriNetViewer
                        head={petriGraphData.head}
                        childrenFragments={petriGraphData.childrenFragments}
                        executedTransitions={new Set(displayedDebugEvents.map((e) => toPetriTransitionId(e.step_name)).filter((id): id is string => Boolean(id)))}
                        runningTransitionId={toPetriTransitionId(displayedDebugEvents.find((event) => event.status === 'running')?.step_name)}
                        selectedTransitionId={toPetriTransitionId(selectedEvent?.step_name)}
                        validationStates={petriTransitionValidationStates}
                        onNodeClick={(tId) => {
                          const event = [...displayedDebugEvents].reverse().find((e) => toPetriTransitionId(e.step_name) === tId);
                          if (event) setSelectedStepId(event.step_id);
                        }}
                      />
                  ) : (
                    <div className="runs-dashboard-graph-empty">
                      Loading graph data...
                    </div>
                  )}
                </div>
              )
            ) : null}
          </div>
        </div>

          <div className="runs-dashboard-main runs-center-stage" data-testid="runs-center-stage">
            <div className="runs-dashboard-inspector" data-testid="runs-dashboard-inspector">
          <div className="debug-step-detail runs-dashboard-card runs-dashboard-inspector-top" data-testid="debug-step-detail">
          <h4>Selected Step Detail</h4>
          {selectedEvent ? (
            <>
              <p className="panel-subtitle">Selected step: {selectedEvent.step_name}</p>
              {stepDetailError ? <p>{stepDetailError}</p> : null}
              {contextSummary ? (
                <section className="runs-context-panel" data-testid="execution-environment-panel">
                  <div className="runs-context-panel-heading">
                    <div>
                      <h5>{contextSummary.stepName}</h5>
                      <p>{whyText}</p>
                    </div>
                    <span className={`runs-dashboard-status-badge runs-dashboard-status-${selectedStepStatus}`}>
                      {contextSummary.stepStatus}
                    </span>
                  </div>
                  <div className="runs-context-summary-grid">
                    <div>
                      <span>Environment Name</span>
                      <strong>{contextSummary.environmentName}</strong>
                    </div>
                    <div>
                      <span>Process Name</span>
                      <strong>{contextSummary.processName}</strong>
                    </div>
                    <div>
                      <span>Step Name</span>
                      <strong>{contextSummary.stepName}</strong>
                    </div>
                    <div>
                      <span>Step Status</span>
                      <strong>{contextSummary.stepStatus}</strong>
                    </div>
                    <div>
                      <span>Step Time</span>
                      <strong>{contextSummary.stepTime}</strong>
                    </div>
                    <div>
                      <span>Executor Info</span>
                      <strong>{contextSummary.executorInfo}</strong>
                    </div>
                  </div>
                  {displayTransitionValidation ? (
                    <section className="runs-transition-validation" data-testid="transition-validation-panel">
                      <div className="runs-transition-validation-head">
                        <div>
                          <h6>Transition Validation</h6>
                          <p>{displayTransitionValidation.passed}/{displayTransitionValidation.total} passed</p>
                          <strong>{displayTransitionValidation.contractSummary}</strong>
                        </div>
                      </div>
                      <ul className="runs-transition-validation-list">
                        {[...displayTransitionValidation.groupedContracts.input, ...displayTransitionValidation.groupedContracts.output].map((contract) => {
                          return (
                            <li key={contract.label} className="runs-transition-validation-item">
                              <div>
                                <strong>{contract.label}</strong>
                              </div>
                              <span className={`runs-transition-validation-status runs-transition-validation-status-${contract.status}`}>
                                {contract.status}
                              </span>
                            </li>
                          );
                        })}
                      </ul>
                    </section>
                  ) : null}
                  <section className="runs-fragment-explorer" data-testid="fragment-explorer">
                    <div className="runs-fragment-explorer-head">
                      <div>
                        <h6>Fragment Explorer</h6>
                        <p>Navigate fragments like a filesystem-backed graph.</p>
                      </div>
                    </div>
                    <div className="runs-fragment-sections">
                      <section className="runs-fragment-section" data-testid="fragment-section-input">
                        <div className="runs-fragment-section-head">
                          <div>
                            <h6>Inputs</h6>
                            <p>{contextInputItems.length} total</p>
                          </div>
                          {filteredInputFragments.length > FRAGMENT_PREVIEW_COUNT ? (
                            <button type="button" className="runs-fragment-toggle" onClick={() => setInputFragmentsExpanded((current) => !current)}>
                              {inputFragmentsExpanded ? 'Show less' : 'Show all'}
                            </button>
                          ) : null}
                        </div>
                        <label className="runs-fragment-search">
                          <span>Search inputs</span>
                          <input
                            type="search"
                            aria-label="Search inputs"
                            placeholder="Filter inputs"
                            value={inputFragmentSearch}
                            onChange={(event) => setInputFragmentSearch(event.target.value)}
                          />
                        </label>
                        {visibleInputFragments.length > 0 ? (
                          renderFragmentList('input', visibleInputFragments)
                        ) : (
                          <p className="runs-fragment-empty">No inputs</p>
                        )}
                      </section>
                      <section className="runs-fragment-section" data-testid="fragment-section-output">
                        <div className="runs-fragment-section-head">
                          <div>
                            <h6>Outputs</h6>
                            <p>{contextOutputItems.length} total</p>
                          </div>
                          {filteredOutputFragments.length > FRAGMENT_PREVIEW_COUNT ? (
                            <button type="button" className="runs-fragment-toggle" onClick={() => setOutputFragmentsExpanded((current) => !current)}>
                              {outputFragmentsExpanded ? 'Show less' : 'Show all'}
                            </button>
                          ) : null}
                        </div>
                        <label className="runs-fragment-search">
                          <span>Search outputs</span>
                          <input
                            type="search"
                            aria-label="Search outputs"
                            placeholder="Filter outputs"
                            value={outputFragmentSearch}
                            onChange={(event) => setOutputFragmentSearch(event.target.value)}
                          />
                        </label>
                        {visibleOutputFragments.length > 0 ? (
                          renderFragmentList('output', visibleOutputFragments)
                        ) : (
                          <p className="runs-fragment-empty">No outputs</p>
                        )}
                      </section>
                    </div>
                    <div className="runs-fragment-explorer-detail">
                      {renderExplorerDetail(selectedExplorerItem)}
                    </div>
                  </section>
                </section>
              ) : null}

            </>
          ) : (
            <p className="panel-subtitle">Select a debug step to view details.</p>
          )}
        </div>

            </div>
          </div>

          {activeRun?.evaluation ? (
            <div className="runs-workspace-evaluation runs-dashboard-evaluation">
              <EvaluationPanel evaluation={activeRun.evaluation} />
            </div>
          ) : (
            <div className="panel-placeholder runs-workspace-evaluation-placeholder runs-dashboard-evaluation">
              {activeRun ? 'No oracle fixture for this case - evaluation skipped' : 'Select a run to view evaluation'}
            </div>
          )}

          <div
            className="runs-log-dock runs-dashboard-card"
            data-testid="runs-log-dock"
            data-collapsed={isLogDockCollapsed ? 'true' : 'false'}
          >
            <button
              type="button"
              className="runs-log-dock-resize-handle"
              data-testid="runs-log-dock-resize-handle"
              aria-label="Resize step logs"
              aria-orientation="vertical"
              onMouseDown={onLogDockResizeStart}
            />
            <div className="runs-log-dock-header">
              <h4>Step Logs</h4>
              <span>{selectedEvent?.step_name ?? 'No step selected'}</span>
              <button
                type="button"
                onClick={() => setIsLogDockCollapsed((value) => !value)}
                aria-label={isLogDockCollapsed ? 'Expand step logs' : 'Collapse step logs'}
              >
                {isLogDockCollapsed ? 'Expand' : 'Collapse'}
              </button>
            </div>
            {isLogDockCollapsed ? null : selectedEvent ? (
              <div className="executor-log-terminal" data-testid="executor-log-terminal" data-terminal-state={terminalState}>
                <div className="executor-log-terminal-body" data-testid="executor-log-terminal-body" style={{ height: `${logDockHeight}px` }}>
                  {terminalLines.length > 0 ? terminalLines.map((line) => (
                    <div
                      key={line.id}
                      className={`executor-log-line ${line.stream === 'stderr' ? 'executor-log-line-stderr' : ''}`.trim()}
                      data-testid={line.stream === 'stderr' ? 'executor-log-line-stderr' : 'executor-log-line'}
                    >
                      <span>{line.text}</span>
                    </div>
                  )) : (
                    <span className="executor-log-line-empty">Waiting for log output...</span>
                  )}
                </div>
              </div>
            ) : (
              <p className="panel-subtitle">{selectedEvent ? 'No logs emitted for this step.' : 'Select a debug step to inspect its live terminal output.'}</p>
            )}
          </div>
        </div>
      </div>
    </section>
  );
};

export default RunsWorkspace;
