export type CaseMeta = {
  case_id: string;
  domain: string;
  size_tier: string;
  chaos_level?: number;
  deliberate_contradictions?: boolean;
  idea_file?: string;
  image_targets?: Record<string, number>;
};

export type Stage = {
  stage_name: string;
  duration_ms: number;
};

export type Decision = {
  step_index: number;
  decision_type: string;
  created_at: string;
};

export type SemanticEntity = {
  id: string;
  label?: string;
  kind?: string;
  confidence?: number;
  payload?: Record<string, unknown>;
};

export type SemanticRelation = {
  id: string;
  kind?: string;
  source?: string;
  target?: string;
  confidence?: number;
  payload?: Record<string, unknown>;
};

export type RunEvaluation = {
  report: {
    compression: {
      total_fragments: number;
      unique_fragments: number;
      total_bytes: number;
      unique_bytes: number;
      dedup_ratio: number;
    };
    entities: { coverage: number; passed: boolean };
    predicates: { predicate_coverage: number; contradiction_coverage: number; passed: boolean };
    exploration: { mean_recall: number; passed: boolean };
    query: { mean_fact_coverage: number; mean_quality_score: number; passed: boolean };
    passed: boolean;
  };
  details?: {
    pipeline_steps?: string[];
    debug_pipeline?: {
      pipeline_id?: string;
      pipeline_run_id?: string;
      env_handles?: { dev_env_id?: string; staging_env_id?: string; committed_env_id?: string };
    };
    entities?: Array<{ expected_name: string; found: boolean; matched_label?: string | null }>;
    predicates?: Array<{ label: string; chain_coverage: number; matched_hops: string[] }>;
    contradictions?: Array<{ field_name: string; detected: boolean; score: number }>;
    exploration_queries?: Array<{ query: string; fragments_retrieved: number; relevance_score: number }>;
    query_results?: Array<{ query: string; facts_found: string[]; fact_coverage: number; quality_score: number; answer_text?: string; evidence_fragment_ids?: string[] }>;
  };
  rendered: string;
};

export type DebugStepEvent = {
  event_id: string;
  step_id: string;
  step_name: string;
  status: 'pending' | 'running' | 'succeeded' | 'failed' | 'not_executed';
  attempt_index: number;
  retry_parent_step_id?: string | null;
   ref?: string;
  env_type: 'dev' | 'staging' | 'committed';
  env_id: string;
  executor_id?: string;
  executor_kind?: string;
  started_at?: string;
  ended_at?: string | null;
  duration_ms?: number | null;
  metrics?: Record<string, unknown>;
  error?: Record<string, unknown> | null;
};

export type DebugStreamResponse = {
  run_id: string;
  pipeline_id: string;
  pipeline_run_id: string;
  status: 'ok' | 'missing';
  execution_mode?: string;
  execution_state?: string;
  control_availability?: {
    can_resume: boolean;
    can_next_step: boolean;
  };
  pipeline_steps?: string[];
  events: DebugStepEvent[];
};

export type ScopedVerificationResponse = {
  run_id: string;
  pipeline_id: string;
  pipeline_run_id: string;
  status: 'ok' | 'missing';
  verification_records: Array<Record<string, unknown>>;
};

export type ScopedFragmentsResponse = {
  run_id: string;
  pipeline_id: string;
  pipeline_run_id: string;
  status: 'ok' | 'missing';
  fragments: Array<Record<string, unknown>>;
};

export type ScopedReconstructionResponse = {
  run_id: string;
  pipeline_id: string;
  pipeline_run_id: string;
  status: 'ok' | 'missing';
  reconstruction_programs: Array<Record<string, unknown>>;
};

export type DebugStepDetailResponse = {
  schema_version: 'v1';
  pipeline_id: string;
  pipeline_run_id: string;
  run_id: string;
  step_id: string;
  step_name: string;
  attempt_index: number;
  outcome: {
    status: string;
    duration_ms: number;
    ref?: string;
    env_type: string;
    env_id: string;
  };
  why: {
    summary: string;
    policy: {
      name: string;
      params: Record<string, unknown>;
    };
  };
  inputs: Record<string, unknown>;
  outputs: Record<string, unknown>;
  checks: Array<Record<string, unknown>>;
  lineage: {
    roots: string[];
    nodes: Array<Record<string, unknown>>;
    edges: Array<Record<string, unknown>>;
  };
  logs?: {
    stdout_lines: string[];
    stderr_lines: string[];
  };
  step_boundaries?: {
    input_boundary?: Record<string, unknown>;
    transition?: Record<string, unknown>;
    output_boundary?: Record<string, unknown>;
    ikam_environment_before?: Record<string, unknown>;
    ikam_environment_after?: Record<string, unknown>;
    handoff_to_next?: Record<string, unknown>;
  };
  executor_logs?: {
    stdout_lines: string[];
    stderr_lines: string[];
  };
  system_logs?: {
    stdout_lines: string[];
    stderr_lines: string[];
  };
  transition_validation?: {
    specs: Array<Record<string, unknown>>;
    resolved_inputs: Record<string, unknown>;
    resolved_outputs: Record<string, unknown>;
    results: Array<{
      name?: string;
      direction?: string;
      kind?: string;
      status?: string;
      matched_fragment_ids?: string[];
      evidence?: Record<string, unknown>;
    }>;
  };
  log_events?: Array<{
    seq: number;
    at: string;
    source: 'executor' | 'system';
    stream: 'stdout' | 'stderr';
    message: string;
  }>;
  [key: string]: unknown;
};

export type ArtifactPreviewResponse = {
  kind: 'text' | 'json' | 'table' | 'slides' | 'doc' | 'pdf' | 'image' | 'binary';
  mime_type: string;
  file_name: string;
  metadata: Record<string, unknown>;
  preview: Record<string, unknown>;
};

export type InspectionSubgraphResponse = {
  schema_version?: 'v1';
  root_node_id?: string;
  navigation?: Record<string, unknown>;
  nodes?: Array<Record<string, unknown>>;
  edges?: Array<Record<string, unknown>>;
  [key: string]: unknown;
};

export type EnvironmentSummaryResponse = {
  run_id: string;
  pipeline_id: string;
  pipeline_run_id: string;
  status: 'ok' | 'missing';
  summary: {
    fragment_count: number;
    verification_count: number;
    reconstruction_program_count: number;
    ref?: string;
    env_type?: string;
    env_id?: string;
    executors_seen?: string[];
  };
};

export type RunControlResponse = {
  run_id: string;
  pipeline_id: string;
  pipeline_run_id: string;
  status: 'ok' | 'missing' | 'duplicate' | 'accepted' | 'busy';
  control_availability?: {
    can_resume: boolean;
    can_next_step: boolean;
  };
  state?: {
    execution_mode: string;
    execution_state: string;
    current_step_name: string;
    current_attempt_index: number;
  };
};

export type HistoryRefEntry = {
  ref: string;
  commit_id: string;
};

export type HistoryRefsResponse = {
  run_id: string;
  refs: HistoryRefEntry[];
};

export type HistoryCommitEntry = {
  id: string;
  profile: string;
  content: Record<string, unknown>;
};

export type HistoryCommitsResponse = {
  run_id: string;
  commits: HistoryCommitEntry[];
};

export type HistoryCommitDetailResponse = {
  run_id: string;
  commit: HistoryCommitEntry;
};

export type HistorySemanticGraphResponse = {
  run_id: string;
  commit_id: string;
  nodes: Array<{ id: string; kind: string; value?: unknown }>;
  edges: Array<Record<string, unknown>>;
};

export type RunEntry = {
  run_id: string;
  project_id: string;
  graph_id?: string;
  case_id: string;
  pipeline_id?: string;
  pipeline_run_id?: string;
  stages: Stage[];
  decisions: Decision[];
  semantic?: { entities?: SemanticEntity[]; relations?: SemanticRelation[] };
  semantic_entities?: number;
  semantic_relations?: number;
  answer_quality?: AnswerQualitySummary;
  evaluation?: RunEvaluation | null;
};

export type EvaluationReportPayload = {
  case_id: string;
  report: {
    compression: {
      total_fragments: number;
      unique_fragments: number;
      total_bytes: number;
      unique_bytes: number;
      dedup_ratio: number;
    };
    entities: {
      coverage: number;
      passed: boolean;
    };
    predicates: {
      predicate_coverage: number;
      contradiction_coverage: number;
      passed: boolean;
    };
    exploration: {
      mean_recall: number;
      passed: boolean;
    };
    query: {
      mean_fact_coverage: number;
      mean_quality_score: number;
      passed: boolean;
    };
    passed: boolean;
  };
  details?: {
    pipeline_steps?: string[];
    entities?: Array<{ expected_name: string; found: boolean; matched_label?: string | null }>;
    predicates?: Array<{ label: string; chain_coverage: number; matched_hops: string[] }>;
    contradictions?: Array<{ field_name: string; detected: boolean; score: number }>;
    exploration_queries?: Array<{ query: string; fragments_retrieved: number; relevance_score: number }>;
    query_results?: Array<{ query: string; facts_found: string[]; fact_coverage: number; quality_score: number; answer_text?: string; evidence_fragment_ids?: string[] }>;
  };
  rendered: string;
};

export type AnswerQualityQueryScore = {
  query_id: string;
  oracle: { coverage: number; grounded_precision: number };
  review: { relevance: number; fidelity: number; clarity: number; note: string };
  oracle_score: number;
  reviewer_score: number;
  aqs: number;
  review_mode: 'manual' | 'oracle-defaulted';
};

export type AnswerQualitySummary = {
  aqs: number;
  oracle_score?: number;
  reviewer_score?: number;
  review_mode: 'manual' | 'oracle-defaulted';
  review_coverage: number;
  query_scores: AnswerQualityQueryScore[];
};

export type GraphSummary = {
  graph_id?: string;
  nodes: number;
  edges: number;
  relation_fragments?: number;
  semantic_entities: number;
  semantic_relations: number;
};

export type EnrichmentItem = {
  enrichment_id: string;
  run_id: string;
  graph_id: string;
  relation_id: string;
  relation_kind: string;
  source: string;
  target: string;
  rationale?: string;
  evidence: string[];
  status: 'staged' | 'approved' | 'queued' | 'committed' | 'rejected';
  sequence: number;
  lane_mode: string;
  unresolved: boolean;
};

export type EnrichmentRun = {
  enrichment_id: string;
  run_id: string;
  graph_id: string;
  sequence: number;
  lane_mode: string;
  status: 'staged' | 'queued' | 'committed' | 'rejected';
  relation_count: number;
  unresolved_count: number;
};

export type EnrichmentReceipt = {
  receipt_id: string;
  graph_id: string;
  committed: number;
  committed_edges: number;
  committed_relations: number;
};

export type MergeResult = {
  merge_id: string;
  graph_ids: string[];
  proposed_edges: Array<Record<string, unknown>>;
  proposed_relational_fragments: Array<Record<string, unknown>>;
  applied: boolean;
  apply_result: { edge_updates: number; relational_fragment_updates: number } | null;
};

export type WikiSection = {
  section_id: string;
  title: string;
  generated_markdown: string;
  generation_provenance: {
    model_id: string;
    harness_id: string;
    generated_at: string;
    prompt_fingerprint: string;
    input_snapshot_hash: string;
  };
};

export type WikiDocument = {
  graph_id: string;
  run_id: string;
  sections: WikiSection[];
  ikam_breakdown: WikiSection;
  generated_at: string;
};

export const GRAPH_SEARCH_RESULT_TYPE = 'GraphSearchResult';

export type GraphSearchResult = {
  query: string;
  results: Array<{ node_id: string; group_ids: string[]; confidence: number }>;
  groups: Array<{ id: string; label: string; size: number; centroid_node_id?: string }>;
  evidence_paths: Array<{ node_id: string; path: Array<{ edge_id?: string; node_id: string }> }>;
  explanations: Array<{
    node_id: string;
    summary: string;
    reasons: {
      text_match_tokens?: string[];
      relation_matches?: string[];
      graph_degree?: number;
      weights?: { text: number; relation: number; graph: number };
    };
  }>;
  scores: Array<{ node_id: string; semantic: number; graph: number; evidence: number; confidence: number }>;
  weights?: { text: number; relation: number; graph: number };
};

export type GraphNodeResponse = {
  id: string;
  type?: string;
  kind?: string;
  level?: number;
  label?: string;
  meta?: Record<string, unknown>;
};

export type GraphEdgeResponse = {
  id?: string;
  source: string;
  target: string;
  kind?: string;
  meta?: Record<string, unknown>;
};

export const resolveApiBaseUrl = (options?: { envValue?: string; hostname?: string }) => {
  const envValue = options?.envValue;
  const hostname = options?.hostname ?? window.location.hostname;
  const fallback = `http://${hostname}:8040`;
  if (!envValue) {
    return fallback;
  }
  if (envValue.includes('ikam-perf-report-api') && (hostname === 'localhost' || hostname === '127.0.0.1')) {
    return fallback;
  }
  return envValue;
};

export const API_BASE_URL = resolveApiBaseUrl({
  envValue: import.meta.env.VITE_API_BASE_URL as string | undefined,
  hostname: window.location.hostname,
});

const requestJson = async <T>(path: string, init?: RequestInit): Promise<T> => {
  const response = await fetch(`${API_BASE_URL}${path}`, init);
  if (!response.ok) {
    throw new Error(`Request failed (${response.status}) for ${path}`);
  }
  return (await response.json()) as T;
};

export const getCases = async (): Promise<CaseMeta[]> => {
  const body = await requestJson<{ cases?: CaseMeta[] }>('/benchmarks/cases');
  return body.cases ?? [];
};

export const listRuns = async (): Promise<RunEntry[]> => {
  const runs = await requestJson<RunEntry[]>('/benchmarks/runs');
  return runs.map((run) => ({
    ...run,
    stages: Array.isArray(run.stages) ? run.stages : [],
    decisions: Array.isArray(run.decisions) ? run.decisions : [],
  }));
};

export const runBenchmarks = async (args: {
  caseIds: string[];
  reset: boolean;
  includeEvaluation?: boolean;
  pipelineId?: string;
}): Promise<RunEntry[]> => {
  const params = new URLSearchParams();
  params.set('case_ids', args.caseIds.join(','));
  params.set('reset', args.reset ? 'true' : 'false');
  params.set('include_evaluation', args.includeEvaluation === true ? 'true' : 'false');
  if (args.pipelineId) params.set('pipeline_id', args.pipelineId);
  const body = await requestJson<{ runs?: RunEntry[] }>(`/benchmarks/run?${params.toString()}`, {
    method: 'POST',
  });
  return body.runs ?? [];
};

export const runEvaluation = async (caseId: string): Promise<EvaluationReportPayload> => {
  const params = new URLSearchParams({ case_id: caseId });
  return requestJson<EvaluationReportPayload>(`/evaluations/run?${params.toString()}`, {
    method: 'POST',
  });
};

export const getGraphSummary = async (graphId: string): Promise<GraphSummary> => {
  const params = new URLSearchParams({ graph_id: graphId });
  return requestJson<GraphSummary>(`/graph/summary?${params.toString()}`);
};

export const getDecisions = async (runId: string): Promise<Decision[]> => {
  const body = await requestJson<{ decisions?: Decision[] }>(`/graph/decisions/${runId}`);
  return body.decisions ?? [];
};

export const getGraphNodes = async (graphId: string): Promise<GraphNodeResponse[]> => {
  const params = new URLSearchParams({ graph_id: graphId });
  return requestJson<GraphNodeResponse[]>(`/graph/nodes?${params.toString()}`);
};

export const getGraphEdges = async (graphId: string): Promise<GraphEdgeResponse[]> => {
  const params = new URLSearchParams({ graph_id: graphId });
  return requestJson<GraphEdgeResponse[]>(`/graph/edges?${params.toString()}`);
};

export const getEnrichmentRuns = async (graphId: string): Promise<EnrichmentRun[]> => {
  const params = new URLSearchParams({ graph_id: graphId });
  const body = await requestJson<{ runs?: EnrichmentRun[] }>(`/graph/enrichment/runs?${params.toString()}`);
  return body.runs ?? [];
};

export const getEnrichmentItems = async (graphId: string): Promise<EnrichmentItem[]> => {
  const params = new URLSearchParams({ graph_id: graphId });
  const body = await requestJson<{ items?: EnrichmentItem[] }>(`/graph/enrichment/staged?${params.toString()}`);
  return body.items ?? [];
};

export const approveEnrichment = async (graphId: string, enrichmentId: string): Promise<{ queued: number }> => {
  const params = new URLSearchParams({ graph_id: graphId });
  return requestJson<{ queued: number }>(`/graph/enrichment/${encodeURIComponent(enrichmentId)}/approve?${params.toString()}`, {
    method: 'POST',
  });
};

export const rejectEnrichment = async (graphId: string, enrichmentId: string): Promise<{ rejected: number }> => {
  const params = new URLSearchParams({ graph_id: graphId });
  return requestJson<{ rejected: number }>(`/graph/enrichment/${encodeURIComponent(enrichmentId)}/reject?${params.toString()}`, {
    method: 'POST',
  });
};

export const commitEnrichmentQueue = async (
  graphId: string
): Promise<{ committed: number; receipt: EnrichmentReceipt | null }> => {
  const params = new URLSearchParams({ graph_id: graphId });
  return requestJson<{ committed: number; receipt: EnrichmentReceipt | null }>(
    `/graph/enrichment/commit?${params.toString()}`,
    { method: 'POST' }
  );
};

export const getEnrichmentReceipts = async (graphId: string): Promise<EnrichmentReceipt[]> => {
  const params = new URLSearchParams({ graph_id: graphId });
  const body = await requestJson<{ receipts?: EnrichmentReceipt[] }>(`/graph/enrichment/receipts?${params.toString()}`);
  return body.receipts ?? [];
};

export const runMerge = async (args: { graphIds: string[]; apply: boolean }): Promise<MergeResult> => {
  const params = new URLSearchParams();
  params.set('graph_ids', args.graphIds.join(','));
  params.set('apply', args.apply ? 'true' : 'false');
  return requestJson<MergeResult>(`/benchmarks/merge?${params.toString()}`, { method: 'POST' });
};

export const generateWiki = async (graphId: string): Promise<WikiDocument> => {
  const params = new URLSearchParams({ graph_id: graphId });
  return requestJson<WikiDocument>(`/graph/wiki/generate?${params.toString()}`, { method: 'POST' });
};

export const saveManualReview = async (
  runId: string,
  review: {
    query_id: string;
    relevance: number;
    fidelity: number;
    clarity: number;
    note: string;
  }
): Promise<AnswerQualitySummary> => {
  const body = await requestJson<{ answer_quality?: AnswerQualitySummary }>(`/benchmarks/runs/${runId}/reviews`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(review),
  });
  return (
    body.answer_quality ?? {
      aqs: 0,
      review_mode: 'oracle-defaulted',
      review_coverage: 0,
      query_scores: [],
    }
  );
};

export const getDebugStream = async (args: {
  runId: string;
  pipelineId: string;
  pipelineRunId: string;
}): Promise<DebugStreamResponse> => {
  const params = new URLSearchParams({
    pipeline_id: args.pipelineId,
    pipeline_run_id: args.pipelineRunId,
  });
  return requestJson<DebugStreamResponse>(`/benchmarks/runs/${args.runId}/debug-stream?${params.toString()}`);
};

export const getScopedVerification = async (args: {
  runId: string;
  pipelineId: string;
  pipelineRunId: string;
  envType: string;
  envId: string;
  stepId: string;
  attemptIndex: number;
}): Promise<ScopedVerificationResponse> => {
  const params = new URLSearchParams({
    pipeline_id: args.pipelineId,
    pipeline_run_id: args.pipelineRunId,
    env_type: args.envType,
    env_id: args.envId,
    step_id: args.stepId,
    attempt_index: String(args.attemptIndex),
  });
  return requestJson<ScopedVerificationResponse>(`/benchmarks/runs/${args.runId}/verification?${params.toString()}`);
};

export const getScopedFragments = async (args: {
  runId: string;
  pipelineId: string;
  pipelineRunId: string;
  envType: string;
  envId: string;
  stepId: string;
  attemptIndex: number;
}): Promise<ScopedFragmentsResponse> => {
  const params = new URLSearchParams({
    pipeline_id: args.pipelineId,
    pipeline_run_id: args.pipelineRunId,
    env_type: args.envType,
    env_id: args.envId,
    step_id: args.stepId,
    attempt_index: String(args.attemptIndex),
  });
  return requestJson<ScopedFragmentsResponse>(`/benchmarks/runs/${args.runId}/env/fragments?${params.toString()}`);
};

export const getScopedReconstructionPrograms = async (args: {
  runId: string;
  pipelineId: string;
  pipelineRunId: string;
  envType: string;
  envId: string;
  stepId: string;
  attemptIndex: number;
}): Promise<ScopedReconstructionResponse> => {
  const params = new URLSearchParams({
    pipeline_id: args.pipelineId,
    pipeline_run_id: args.pipelineRunId,
    env_type: args.envType,
    env_id: args.envId,
    step_id: args.stepId,
    attempt_index: String(args.attemptIndex),
  });
  return requestJson<ScopedReconstructionResponse>(
    `/benchmarks/runs/${args.runId}/reconstruction-program?${params.toString()}`
  );
};

export const getEnvironmentSummary = async (args: {
  runId: string;
  pipelineId: string;
  pipelineRunId: string;
  envType: string;
  envId: string;
}): Promise<EnvironmentSummaryResponse> => {
  const params = new URLSearchParams({
    pipeline_id: args.pipelineId,
    pipeline_run_id: args.pipelineRunId,
    env_type: args.envType,
    env_id: args.envId,
  });
  return requestJson<EnvironmentSummaryResponse>(`/benchmarks/runs/${args.runId}/env-summary?${params.toString()}`);
};

export const getDebugStepDetail = async (args: { runId: string; stepId: string }): Promise<DebugStepDetailResponse> => {
  return requestJson<DebugStepDetailResponse>(`/benchmarks/runs/${args.runId}/debug-step/${args.stepId}/detail`);
};

export const getArtifactPreview = async (args: { runId: string; artifactId: string }): Promise<ArtifactPreviewResponse> => {
  const params = new URLSearchParams({ artifact_id: args.artifactId });
  return requestJson<ArtifactPreviewResponse>(`/benchmarks/runs/${args.runId}/artifacts/preview?${params.toString()}`);
};

export const getInspectionSubgraph = async (args: {
  runId: string;
  ref: string;
  maxDepth?: number;
}): Promise<InspectionSubgraphResponse> => {
  const params = new URLSearchParams({
    ref: args.ref,
    max_depth: String(args.maxDepth ?? 1),
  });
  return requestJson<InspectionSubgraphResponse>(`/benchmarks/runs/${args.runId}/inspection?${params.toString()}`);
};

export const getHistoryRefs = async (runId: string): Promise<HistoryRefsResponse> => {
  const params = new URLSearchParams({ run_id: runId });
  return requestJson<HistoryRefsResponse>(`/history/refs?${params.toString()}`);
};

export const getHistoryCommits = async (args: { runId: string; ref?: string }): Promise<HistoryCommitsResponse> => {
  const params = new URLSearchParams({ run_id: args.runId });
  if (args.ref) {
    params.set('ref', args.ref);
  }
  return requestJson<HistoryCommitsResponse>(`/history/commits?${params.toString()}`);
};

export const getHistoryCommitDetail = async (args: {
  runId: string;
  commitId: string;
}): Promise<HistoryCommitDetailResponse> => {
  const params = new URLSearchParams({ run_id: args.runId });
  return requestJson<HistoryCommitDetailResponse>(`/history/commits/${encodeURIComponent(args.commitId)}?${params.toString()}`);
};

export const getHistorySemanticGraph = async (args: {
  runId: string;
  commitId: string;
}): Promise<HistorySemanticGraphResponse> => {
  const params = new URLSearchParams({ run_id: args.runId });
  return requestJson<HistorySemanticGraphResponse>(
    `/history/commits/${encodeURIComponent(args.commitId)}/semantic-graph?${params.toString()}`
  );
};

export const controlRun = async (args: {
  runId: string;
  commandId: string;
  action: 'set_mode' | 'pause' | 'resume' | 'next_step';
  pipelineId: string;
  pipelineRunId: string;
  mode?: 'autonomous' | 'manual';
}): Promise<RunControlResponse> => {
  return requestJson<RunControlResponse>(`/benchmarks/runs/${args.runId}/control`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      command_id: args.commandId,
      action: args.action,
      pipeline_id: args.pipelineId,
      pipeline_run_id: args.pipelineRunId,
      mode: args.mode,
    }),
  });
};

export const getWiki = async (graphId: string): Promise<WikiDocument | null> => {
  const params = new URLSearchParams({ graph_id: graphId });
  const body = await requestJson<WikiDocument | { status: string }>(`/graph/wiki?${params.toString()}`);
  if ((body as { status?: string }).status === 'missing') {
    return null;
  }
  return body as WikiDocument;
};

export const downloadArtifact = async (artifactId: string): Promise<void> => {
  const response = await fetch(`${API_BASE_URL}/artifacts/${artifactId}/download`, {
    method: 'GET',
  });
  if (!response.ok) {
    throw new Error(`Download failed (${response.status}) for artifact ${artifactId}`);
  }
  const blob = await response.blob();
  const disposition = response.headers.get('content-disposition');
  const filenameMatch = disposition?.match(/filename="?([^"]+)"?/);
  const filename = filenameMatch?.[1] ?? `artifact-${artifactId}`;
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  document.body.appendChild(anchor);
  anchor.click();
  document.body.removeChild(anchor);
  URL.revokeObjectURL(url);
};

export interface RegistryEntry {
  key: string;
  type: string;
  fragment_id?: string;
  head_fragment_id?: string;
  title?: string;
  goal?: string;
  registered_at?: string;
  project_id?: string;
  [key: string]: any;
}

export interface RegistryResponse {
  namespace: string;
  version: number;
  entries: RegistryEntry[];
}

export interface SubgraphResponse {
  head: any;
  children: Record<string, any>;
}



export const getRegistryNamespaces = async (): Promise<string[]> => {
  const res = await fetch(`${API_BASE_URL}/registry`);
  if (!res.ok) throw new Error(`Failed to fetch registry namespaces: ${res.statusText}`);
  const data = await res.json();
  return data.namespaces || [];
};

export const getRegistry = async (namespace: string): Promise<RegistryResponse> => {
  const res = await fetch(`${API_BASE_URL}/registry/${namespace}`);
  if (!res.ok) throw new Error(`Failed to fetch registry ${namespace}: ${res.statusText}`);
  return res.json();
};

export const getSubgraph = async (headId: string): Promise<SubgraphResponse> => {
  const res = await fetch(`${API_BASE_URL}/registry/subgraph/${headId}`);
  if (!res.ok) throw new Error(`Failed to fetch subgraph ${headId}: ${res.statusText}`);
  return res.json();
};
