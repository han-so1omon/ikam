import { vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { within } from '@testing-library/react';

import RunsWorkspace from '../components/RunsWorkspace';
import { mockFitView } from '../__mocks__/@xyflow/react';

const {
  mockControlRun,
  mockGetDebugStream,
  mockGetScopedFragments,
  mockGetEnvironmentSummary,
  mockGetDebugStepDetail,
  mockGetInspectionSubgraph,
  mockGetArtifactPreview,
  mockGetRegistry,
} = vi.hoisted(() => ({
  mockControlRun: vi.fn(),
  mockGetDebugStream: vi.fn(),
  mockGetScopedFragments: vi.fn(),
  mockGetEnvironmentSummary: vi.fn(),
  mockGetDebugStepDetail: vi.fn(),
  mockGetInspectionSubgraph: vi.fn(),
  mockGetArtifactPreview: vi.fn(),
  mockGetRegistry: vi.fn(),
}));

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('../api/client');
  return {
    ...actual,
    controlRun: mockControlRun,
    getDebugStream: mockGetDebugStream,
    getScopedFragments: mockGetScopedFragments,
    getEnvironmentSummary: mockGetEnvironmentSummary,
    getDebugStepDetail: mockGetDebugStepDetail,
    getInspectionSubgraph: mockGetInspectionSubgraph,
    getArtifactPreview: mockGetArtifactPreview,
    getRegistry: mockGetRegistry,
  };
});

beforeEach(() => {
  mockFitView.mockReset();
  mockGetDebugStream.mockReset();
  mockControlRun.mockReset();
  mockGetScopedFragments.mockReset();
  mockGetEnvironmentSummary.mockReset();
  mockGetDebugStepDetail.mockReset();
  mockGetInspectionSubgraph.mockReset();
  mockGetArtifactPreview.mockReset();
  mockGetRegistry.mockReset();
  mockGetRegistry.mockResolvedValue({ namespace: 'petri_net_runnables', entries: [{ key: 'mock-pipeline/v1' }] });
  mockGetScopedFragments.mockResolvedValue({ status: 'ok', fragments: [] });
  mockGetEnvironmentSummary.mockResolvedValue({
    status: 'ok',
    summary: { fragment_count: 0, verification_count: 0, reconstruction_program_count: 0 },
  });
});

test('loads debug stream and fetches scoped detail for selected step', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-1',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-1',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: {
      can_resume: true,
      can_next_step: true,
    },
    events: [
      {
        event_id: 'ev-1',
        step_id: 'step-a',
        step_name: 'map.conceptual.normalize.discovery',
                status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-1',
      },
      {
        event_id: 'ev-2',
        step_id: 'step-b',
        step_name: 'map.conceptual.verify.discovery_gate',
                status: 'failed',
        attempt_index: 2,
        retry_parent_step_id: 'step-a',
        env_type: 'staging',
        env_id: 'stg-1',
      },
    ],
  });
  mockGetScopedFragments.mockResolvedValue({ status: 'ok', fragments: [] });
  mockGetEnvironmentSummary.mockResolvedValue({
    status: 'ok',
    summary: { fragment_count: 0, verification_count: 0, reconstruction_program_count: 0 },
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-1',
    run_id: 'run-1',
    step_id: 'step-b',
    step_name: 'map.conceptual.verify.discovery_gate',
    attempt_index: 2,
    outcome: { status: 'failed', duration_ms: 101, env_type: 'staging', env_id: 'stg-1' },
    why: { summary: 'Verification failed', policy: { name: 'map.conceptual.verify.discovery_gate', params: {} } },
    inputs: { artifact_ids: ['artifact:run-1'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: ['frag-1'], program_ids: [], pair_ids: [] },
    produced_fragment_ids: ['frag-1'],
    checks: [{ name: 'proof_gate', status: 'fail', details: {} }],
    lineage: {
      roots: ['artifact:run-1'],
      nodes: [
        { node_id: 'artifact:run-1', kind: 'artifact', fragment_id: 'artifact:run-1', cas_id: null, mime_type: 'text/markdown', label: 'artifact:run-1', meta: {} },
        { node_id: 'fragment:frag-1', kind: 'surface', fragment_id: 'frag-1', cas_id: 'cas-1', mime_type: 'text/markdown', label: 'frag-1', meta: { record_type: 'surface_fragment' } },
      ],
      edges: [{ from: 'artifact:run-1', to: 'fragment:frag-1', relation: 'contains', step_name: 'map.conceptual.verify.discovery_gate' }],
    },
  });
  mockControlRun.mockResolvedValue({
    status: 'ok',
    run_id: 'run-1',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-1',
    state: {
      execution_mode: 'manual',
      execution_state: 'paused',
      current_step_name: 'map.conceptual.normalize.discovery',
      current_attempt_index: 1,
    },
  });

  render(
    <RunsWorkspace
      cases={[
        { case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' },
        { case_id: 'xl-construction-v01', domain: 'construction', size_tier: 'xl' },
      ]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-1',
          project_id: 'proj-1',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'compression-rerender/v1', pipeline_run_id: 'pipe-1' } },
          },
        },
      ]}
      activeRunId="run-1"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(screen.getByTestId('debug-step-list')).toBeInTheDocument());
  expect(screen.getByTestId('run-debug-layout')).toBeInTheDocument();
  expect(screen.getByTestId('run-debug-case-selector')).toBeInTheDocument();
  expect(screen.getByTestId('run-debug-case-list')).toBeInTheDocument();
  expect(screen.getByRole('searchbox', { name: 'Search cases' })).toBeInTheDocument();
  expect(screen.getByTestId('run-debug-actions')).toBeInTheDocument();
  expect(screen.getByTestId('run-debug-controls-stack')).toBeInTheDocument();
  const controlsStack = screen.getByTestId('run-debug-controls-stack');
  expect(within(controlsStack).getByRole('button', { name: 'Run Pipeline' })).toBeInTheDocument();
  expect(within(controlsStack).getByText('Reset before run')).toBeInTheDocument();
  await waitFor(() => {
    expect(within(screen.getByTestId('debug-step-list')).getByRole('button', { name: /normalize\.discovery.*succeeded/i })).toBeInTheDocument();
  });
  await waitFor(() => {
    expect(within(screen.getByTestId('debug-step-list')).getByRole('button', { name: /verify\.discovery_gate.*failed/i })).toBeInTheDocument();
  });
  expect(screen.getByText('A2')).toBeInTheDocument();
  expect(screen.getByText('Retry')).toBeInTheDocument();
  expect(screen.getByText('xl-construction-v01')).toBeInTheDocument();

  fireEvent.change(screen.getByRole('searchbox', { name: 'Search cases' }), { target: { value: 'construction' } });
  const selector = screen.getByTestId('run-debug-case-selector');
  expect(within(selector).getByText('xl-construction-v01')).toBeInTheDocument();
  expect(within(selector).queryByText('s-local-retail-v01')).not.toBeInTheDocument();

  await waitFor(() => {
    expect(mockGetDebugStepDetail).toHaveBeenCalled();
  });

  fireEvent.click(screen.getByRole('button', { name: /verify\.discovery_gate.*failed/i }));

  await waitFor(() => {
    expect(screen.getByTestId('execution-environment-panel')).toBeInTheDocument();
    expect(screen.getByTestId('fragment-explorer')).toBeInTheDocument();
  });
  expect(within(screen.getByTestId('fragment-section-output')).getByText('frag-1')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: /normalize\.discovery.*succeeded/i }));
  await waitFor(() => {
    expect(screen.getByTestId('execution-environment-panel')).toBeInTheDocument();
  });
  expect(within(screen.getByTestId('fragment-section-input')).getByText('No inputs')).toBeInTheDocument();

  fireEvent.click(screen.getByRole('button', { name: 'Next Step' }));
  await waitFor(() => {
    expect(mockControlRun).toHaveBeenCalledWith(
      expect.objectContaining({
        runId: 'run-1',
        action: 'next_step',
        pipelineId: 'compression-rerender/v1',
        pipelineRunId: 'pipe-1',
      })
    );
  });
});

test('uses orchestration trace data in selected step detail context', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-trace-ui',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-trace-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: {
      can_resume: true,
      can_next_step: true,
    },
    events: [
      {
        event_id: 'ev-trace-ui',
        step_id: 'step-trace-ui',
        step_name: 'map.conceptual.normalize.discovery',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-trace-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-trace-ui',
    run_id: 'run-trace-ui',
    step_id: 'step-trace-ui',
    step_name: 'map.conceptual.normalize.discovery',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 17, env_type: 'dev', env_id: 'dev-trace-ui' },
    why: { summary: 'Normalization completed', policy: { name: 'map.conceptual.normalize.discovery', params: {} } },
    inputs: { artifact_ids: ['artifact:trace-ui'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: ['frag-trace-ui'], program_ids: [], pair_ids: [] },
    checks: [],
    produced_fragment_ids: ['frag-trace-ui'],
    trace: {
      workflow_id: 'wf-trace-ui',
      request_id: 'req-trace-ui',
      executor_id: 'executor://python-primary',
      executor_kind: 'python-executor',
      transition_id: 'transition:dispatch-parse',
      marking_before_ref: 'marking://before-trace-ui',
      marking_after_ref: 'marking://after-trace-ui',
      enabled_transition_ids: ['transition:review', 'transition:commit'],
      topic_sequence: [
        { topic: 'workflow.events', event_type: 'execution.queued', status: 'queued' },
        { topic: 'execution.progress', event_type: 'execution.running', status: 'running' },
        { topic: 'execution.results', event_type: 'execution.completed', status: 'succeeded' },
      ],
      trace_id: 'trace-ui',
      committed_trace_fragment_id: 'trace-fragment-ui',
    },
    lineage: {
      roots: ['artifact:trace-ui'],
      nodes: [
        { node_id: 'artifact:trace-ui', kind: 'artifact', fragment_id: 'artifact:trace-ui', cas_id: null, mime_type: 'text/markdown', label: 'artifact:trace-ui', meta: {} },
      ],
      edges: [],
    },
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-trace-ui',
          project_id: 'proj-trace-ui',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'compression-rerender/v1', pipeline_run_id: 'pipe-trace-ui' } },
          },
        },
      ]}
      activeRunId="run-trace-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  const environmentPanel = await screen.findByTestId('execution-environment-panel');
  expect(within(environmentPanel).getByRole('heading', { name: 'map.conceptual.normalize.discovery', level: 5 })).toBeInTheDocument();
  expect(within(environmentPanel).getByText('python-executor · executor://python-primary')).toBeInTheDocument();
  expect(within(environmentPanel).getByText('Normalization completed')).toBeInTheDocument();
});

test('shows debug stream loading immediately after run starts', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'missing',
    run_id: 'missing',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'missing',
    events: [],
  });

  const Harness = () => {
    const [running, setRunning] = useState(false);
    const [selectedCaseIds, setSelectedCaseIds] = useState<string[]>([]);
    return (
      <RunsWorkspace
        cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
        loadingCases={false}
        caseError={null}
        selectedCaseIds={selectedCaseIds}
        onToggleCase={(caseId) =>
          setSelectedCaseIds((current) =>
            current.includes(caseId) ? current.filter((item) => item !== caseId) : [...current, caseId]
          )
        }
        reset={false}
        onResetChange={() => {}}
        running={running}
        runError={null}
        onRunCases={() => setRunning(true)}
        runs={[]}
        activeRunId={null}
        onSelectRun={() => {}}
      />
    );
  };

  render(<Harness />);
  fireEvent.click(screen.getByLabelText(/s-local-retail-v01/i));
  fireEvent.click(screen.getByRole('button', { name: 'Run Pipeline' }));

  await waitFor(() => {
    expect(screen.getByText('Loading debug stream...')).toBeInTheDocument();
  });
});

test('renders step detail for embed_mapped step', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-embed',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-embed',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-embed',
        step_id: 'step-embed',
        step_name: 'map.conceptual.embed.discovery_index',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-embed',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-embed',
    run_id: 'run-embed',
    step_id: 'step-embed',
    step_name: 'map.conceptual.embed.discovery_index',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 54, env_type: 'dev', env_id: 'dev-embed' },
    why: { summary: 'Embedded decomposed fragments.', policy: { name: 'map.conceptual.embed.discovery_index', params: {} } },
    inputs: { artifact_ids: ['artifact:run-embed'], fragment_ids: ['frag-1', 'frag-2', 'frag-3'], program_ids: [] },
    outputs: {
      embedding_count: 3,
      embedding_dimensions: 4,
      fragment_ids: ['frag-1', 'frag-2', 'frag-3'],
      embedding_projection: {
        method: 'pca_2d',
        points: [
          { fragment_id: 'frag-1', x: -0.8, y: 0.1, cluster_id: 'cluster-0' },
          { fragment_id: 'frag-2', x: -0.2, y: 0.25, cluster_id: 'cluster-0' },
          { fragment_id: 'frag-3', x: 0.9, y: -0.2, cluster_id: 'cluster-1' },
        ],
        centroids: [
          { cluster_id: 'cluster-0', x: -0.5, y: 0.17, size: 2, avg_similarity: 0.91 },
          { cluster_id: 'cluster-1', x: 0.9, y: -0.2, size: 1, avg_similarity: 1.0 },
        ],
      },
      pairwise_similarity: {
        fragment_ids: ['frag-1', 'frag-2', 'frag-3'],
        matrix: [
          [1.0, 0.82, 0.14],
          [0.82, 1.0, 0.21],
          [0.14, 0.21, 1.0],
        ],
        min: 0.14,
        max: 1.0,
        threshold: 0.7,
      },
      embedding_debug: {
        expected_count: 3,
        embedded_count: 3,
        coverage_ratio: 1.0,
        missing_fragment_ids: [],
        singleton_clusters: 1,
        cluster_count: 2,
        threshold: 0.7,
        embedding_mode: 'deterministic',
      },
    },
    checks: [{ name: 'surface_embedding_coverage', status: 'pass', details: { embedded_count: 3 } }],
    lineage: {
      roots: ['artifact:run-embed'],
      nodes: [
        { node_id: 'artifact:run-embed', kind: 'artifact', fragment_id: 'artifact:run-embed', cas_id: null, mime_type: 'text/markdown', label: 'artifact:run-embed', meta: {} },
        { node_id: 'fragment:frag-1', kind: 'surface', fragment_id: 'frag-1', cas_id: 'cas-1', mime_type: 'text/markdown', label: 'frag-1', meta: { record_type: 'surface_fragment', value_preview: 'Revenue grew in Q1.' } },
        { node_id: 'fragment:frag-2', kind: 'surface', fragment_id: 'frag-2', cas_id: 'cas-2', mime_type: 'text/markdown', label: 'frag-2', meta: { record_type: 'surface_fragment', value_preview: 'Revenue stayed flat in Q2.' } },
        { node_id: 'fragment:frag-3', kind: 'surface', fragment_id: 'frag-3', cas_id: 'cas-3', mime_type: 'text/markdown', label: 'frag-3', meta: { record_type: 'surface_fragment', value_preview: 'Costs increased in Q3.' } },
      ],
      edges: [
        { from: 'artifact:run-embed', to: 'fragment:frag-1', relation: 'contains', step_name: 'map.conceptual.embed.discovery_index' },
        { from: 'artifact:run-embed', to: 'fragment:frag-2', relation: 'contains', step_name: 'map.conceptual.embed.discovery_index' },
        { from: 'artifact:run-embed', to: 'fragment:frag-3', relation: 'contains', step_name: 'map.conceptual.embed.discovery_index' },
      ],
    },
  });
  mockControlRun.mockResolvedValue({
    status: 'ok',
    run_id: 'run-embed',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-embed',
    state: {
      execution_mode: 'manual',
      execution_state: 'paused',
      current_step_name: 'map.conceptual.embed.discovery_index',
      current_attempt_index: 1,
    },
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-embed',
          project_id: 'proj-embed',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 3, unique_fragments: 3, total_bytes: 10, unique_bytes: 10, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'compression-rerender/v1', pipeline_run_id: 'pipe-embed' } },
          },
        },
      ]}
      activeRunId="run-embed"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(screen.getByTestId('execution-environment-panel')).toBeInTheDocument());
  expect(screen.getByText(/Selected step: map\.conceptual\.embed\.discovery_index/i)).toBeInTheDocument();
  expect(screen.getByTestId('fragment-explorer')).toBeInTheDocument();
  const outputSection = within(screen.getByTestId('fragment-section-output'));
  expect(outputSection.getByText('frag-1')).toBeInTheDocument();
  expect(outputSection.getByText('frag-2')).toBeInTheDocument();
  expect(outputSection.getByText('frag-3')).toBeInTheDocument();
});

test('renders step detail for embed_lifted step', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-embed-lifted',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-embed-lifted',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-embed-lifted',
        step_id: 'step-embed-lifted',
        step_name: 'map.reconstructable.embed',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-embed-lifted',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-embed-lifted',
    run_id: 'run-embed-lifted',
    step_id: 'step-embed-lifted',
    step_name: 'map.reconstructable.embed',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 61, env_type: 'dev', env_id: 'dev-embed-lifted' },
    why: { summary: 'Embedded lifted fragments.', policy: { name: 'map.reconstructable.embed', params: {} } },
    inputs: { artifact_ids: ['artifact:run-embed-lifted'], fragment_ids: ['ir-1', 'ir-2', 'ir-3'], program_ids: [] },
    outputs: {
      embedding_count: 3,
      embedding_dimensions: 6,
      fragment_ids: ['ir-1', 'ir-2', 'ir-3'],
      pairwise_similarity: {
        fragment_ids: ['ir-1', 'ir-2', 'ir-3'],
        matrix: [
          [1.0, 0.78, 0.19],
          [0.78, 1.0, 0.22],
          [0.19, 0.22, 1.0],
        ],
        threshold: 0.7,
      },
      embedding_debug: {
        expected_count: 3,
        embedded_count: 3,
        coverage_ratio: 1.0,
        missing_fragment_ids: [],
        singleton_clusters: 1,
        embedding_mode: 'deterministic',
      },
    },
    checks: [{ name: 'lifted_embedding_coverage', status: 'pass', details: { embedded_count: 3 } }],
    lineage: {
      roots: ['artifact:run-embed-lifted'],
      nodes: [
        { node_id: 'artifact:run-embed-lifted', kind: 'artifact', fragment_id: 'artifact:run-embed-lifted', cas_id: null, mime_type: 'text/markdown', label: 'artifact:run-embed-lifted', meta: {} },
        { node_id: 'fragment:surface-1', kind: 'surface', fragment_id: 'surface-1', cas_id: 'surface-1', mime_type: 'text/markdown', label: 'surface-1', meta: { record_type: 'surface_fragment', artifact_id: 'artifact:run-embed-lifted', value_preview: { text: 'Revenue baseline in Q1.' } } },
        { node_id: 'fragment:surface-2', kind: 'surface', fragment_id: 'surface-2', cas_id: 'surface-2', mime_type: 'text/markdown', label: 'surface-2', meta: { record_type: 'surface_fragment', artifact_id: 'artifact:run-embed-lifted', value_preview: { text: 'EBITDA baseline in Q1.' } } },
        { node_id: 'fragment:ir-1', kind: 'ir', fragment_id: 'ir-1', cas_id: 'ir-1', mime_type: 'application/vnd.ikam.claim-ir+json', label: 'ir-1', meta: { record_type: 'ir_fragment', artifact_id: 'artifact:run-embed-lifted', source_surface_fragment_id: 'surface-1', value_preview: { claim: 'Revenue grew 12% YoY' } } },
        { node_id: 'fragment:ir-2', kind: 'ir', fragment_id: 'ir-2', cas_id: 'ir-2', mime_type: 'application/vnd.ikam.claim-ir+json', label: 'ir-2', meta: { record_type: 'ir_fragment', artifact_id: 'artifact:run-embed-lifted', source_surface_fragment_id: 'surface-1', value_preview: { claim: 'Gross margin expanded' } } },
        { node_id: 'fragment:ir-3', kind: 'ir', fragment_id: 'ir-3', cas_id: 'ir-3', mime_type: 'application/vnd.ikam.claim-ir+json', label: 'ir-3', meta: { record_type: 'ir_fragment', artifact_id: 'artifact:run-embed-lifted', source_surface_fragment_id: 'surface-2', value_preview: { claim: 'EBITDA improved by 2pp' } } },
      ],
      edges: [
        { from: 'artifact:run-embed-lifted', to: 'fragment:surface-1', relation: 'contains', step_name: 'map.reconstructable.embed' },
        { from: 'artifact:run-embed-lifted', to: 'fragment:surface-2', relation: 'contains', step_name: 'map.reconstructable.embed' },
        { from: 'fragment:surface-1', to: 'fragment:ir-1', relation: 'lifted_from', step_name: 'map.reconstructable.embed' },
        { from: 'fragment:surface-1', to: 'fragment:ir-2', relation: 'lifted_from', step_name: 'map.reconstructable.embed' },
        { from: 'fragment:surface-2', to: 'fragment:ir-3', relation: 'lifted_from', step_name: 'map.reconstructable.embed' },
      ],
    },
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-embed-lifted',
          project_id: 'proj-embed-lifted',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 3, unique_fragments: 3, total_bytes: 10, unique_bytes: 10, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'compression-rerender/v1', pipeline_run_id: 'pipe-embed-lifted' } },
          },
        },
      ]}
      activeRunId="run-embed-lifted"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(screen.getByTestId('execution-environment-panel')).toBeInTheDocument());
  expect(screen.getByText(/Selected step: map\.reconstructable\.embed/i)).toBeInTheDocument();
  const inputSection = within(screen.getByTestId('fragment-section-input'));
  const outputSection = within(screen.getByTestId('fragment-section-output'));
  expect(inputSection.getByText('ir-1')).toBeInTheDocument();
  expect(inputSection.getByText('ir-2')).toBeInTheDocument();
  expect(inputSection.getByText('ir-3')).toBeInTheDocument();
  expect(outputSection.getByText('ir-1')).toBeInTheDocument();
  expect(outputSection.getByText('ir-2')).toBeInTheDocument();
  expect(outputSection.getByText('ir-3')).toBeInTheDocument();
});

test('renders step detail for lift step with scoped fragments', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-lift',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-lift',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-lift',
        step_id: 'step-lift',
        step_name: 'map.conceptual.normalize.discovery',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-lift',
      },
    ],
  });
  mockGetScopedFragments.mockResolvedValue({ status: 'ok', fragments: [] });
  mockGetEnvironmentSummary.mockResolvedValue({
    status: 'ok',
    summary: { fragment_count: 5, verification_count: 0, reconstruction_program_count: 0 },
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-lift',
    run_id: 'run-lift',
    step_id: 'step-lift',
    step_name: 'map.conceptual.normalize.discovery',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 88, env_type: 'dev', env_id: 'dev-lift' },
    why: { summary: 'Lifted fragments', policy: { name: 'map.conceptual.normalize.discovery', params: {} } },
    inputs: { artifact_ids: ['artifact:run-lift'], fragment_ids: ['surface-1', 'surface-2'], program_ids: [] },
    outputs: {
      artifact_ids: [],
      fragment_ids: ['ir-1', 'ir-2'],
      program_ids: [],
      pair_ids: [],
      lift_transformations: [
        {
          surface_fragment_id: 'surface-1',
          source_artifact_id: 'artifact:run-lift',
          ir_fragment_ids: ['ir-1', 'ir-2'],
          lift_status: 'lifted',
          lift_reason: null,
        },
        {
          surface_fragment_id: 'surface-2',
          source_artifact_id: 'artifact:run-lift',
          ir_fragment_ids: [],
          lift_status: 'surface_only',
          lift_reason: 'no_ir_generated',
        },
      ],
    },
    checks: [{ name: 'lift_provenance_complete', status: 'pass', details: { ir_fragment_count: 2 } }],
    lineage: {
      roots: ['artifact:run-lift'],
      nodes: [
        { node_id: 'artifact:run-lift', kind: 'artifact', fragment_id: 'artifact:run-lift', cas_id: null, mime_type: 'text/markdown', label: 'artifact:run-lift', meta: {} },
        { node_id: 'fragment:surface-1', kind: 'surface', fragment_id: 'surface-1', cas_id: 'surface-1', mime_type: 'text/markdown', label: 'surface-1', meta: { record_type: 'surface_fragment', artifact_id: 'artifact:run-lift', value_preview: { text: 'Revenue baseline for Q1.' } } },
        { node_id: 'fragment:surface-2', kind: 'surface', fragment_id: 'surface-2', cas_id: 'surface-2', mime_type: 'text/markdown', label: 'surface-2', meta: { record_type: 'surface_fragment', artifact_id: 'artifact:run-lift', value_preview: { text: 'No claim could be lifted.' } } },
        { node_id: 'fragment:ir-1', kind: 'ir', fragment_id: 'ir-1', cas_id: 'ir-1', mime_type: 'application/vnd.ikam.claim-ir+json', label: 'ir-1', meta: { record_type: 'ir_fragment', source_surface_fragment_id: 'surface-1', artifact_id: 'artifact:run-lift', value_preview: { relation_type: 'growth_change', slot_bindings: { metric: 'revenue', delta_pct: 14, period: 'Q1' } } } },
        { node_id: 'fragment:ir-2', kind: 'ir', fragment_id: 'ir-2', cas_id: 'ir-2', mime_type: 'application/vnd.ikam.claim-ir+json', label: 'ir-2', meta: { record_type: 'ir_fragment', source_surface_fragment_id: 'surface-1', artifact_id: 'artifact:run-lift', value_preview: { claim: 'EBITDA margin improved by 2pp' } } },
      ],
      edges: [
        { from: 'artifact:run-lift', to: 'fragment:surface-1', relation: 'contains', step_name: 'map.conceptual.normalize.discovery' },
        { from: 'artifact:run-lift', to: 'fragment:surface-2', relation: 'contains', step_name: 'map.conceptual.normalize.discovery' },
        { from: 'artifact:run-lift', to: 'fragment:ir-1', relation: 'contains', step_name: 'map.conceptual.normalize.discovery' },
        { from: 'artifact:run-lift', to: 'fragment:ir-2', relation: 'contains', step_name: 'map.conceptual.normalize.discovery' },
        { from: 'fragment:surface-1', to: 'fragment:ir-1', relation: 'lifted_from', step_name: 'map.conceptual.normalize.discovery' },
        { from: 'fragment:surface-1', to: 'fragment:ir-2', relation: 'lifted_from', step_name: 'map.conceptual.normalize.discovery' },
      ],
    },
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-lift',
          project_id: 'proj-lift',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 2, unique_fragments: 2, total_bytes: 10, unique_bytes: 10, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'compression-rerender/v1', pipeline_run_id: 'pipe-lift' } },
          },
        },
      ]}
      activeRunId="run-lift"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(screen.getByTestId('execution-environment-panel')).toBeInTheDocument());
  expect(screen.getByText(/Selected step: map\.conceptual\.normalize\.discovery/i)).toBeInTheDocument();
  const inputSection = within(screen.getByTestId('fragment-section-input'));
  const outputSection = within(screen.getByTestId('fragment-section-output'));
  expect(inputSection.getByText('surface-1')).toBeInTheDocument();
  expect(inputSection.getByText('surface-2')).toBeInTheDocument();
  expect(outputSection.getByText('ir-1')).toBeInTheDocument();
  expect(outputSection.getByText('ir-2')).toBeInTheDocument();
});

test('renders step detail for normalize step with normalized outputs', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-normalize',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-normalize',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-normalize',
        step_id: 'step-normalize',
        step_name: 'map.reconstructable.normalize',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-normalize',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-normalize',
    run_id: 'run-normalize',
    step_id: 'step-normalize',
    step_name: 'map.reconstructable.normalize',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 61, env_type: 'dev', env_id: 'dev-normalize' },
    why: { summary: 'Normalized fragments', policy: { name: 'map.reconstructable.normalize', params: {} } },
    inputs: { artifact_ids: ['artifact:run-normalize'], fragment_ids: ['ir-1'], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: ['norm-1'], program_ids: [], pair_ids: [] },
    checks: [{ name: 'normalization_complete', status: 'pass', details: {} }],
    lineage: {
      roots: ['artifact:run-normalize'],
      nodes: [
        { node_id: 'artifact:run-normalize', kind: 'artifact', fragment_id: 'artifact:run-normalize', cas_id: null, mime_type: 'text/markdown', label: 'artifact:run-normalize', meta: {} },
        { node_id: 'fragment:surface-1', kind: 'surface', fragment_id: 'surface-1', cas_id: 'surface-1', mime_type: 'text/markdown', label: 'surface-1', meta: { artifact_id: 'artifact:run-normalize', record_type: 'surface_fragment' } },
        { node_id: 'fragment:ir-1', kind: 'ir', fragment_id: 'ir-1', cas_id: 'ir-1', mime_type: 'application/vnd.ikam.claim-ir+json', label: 'ir-1', meta: { artifact_id: 'artifact:run-normalize', source_surface_fragment_id: 'surface-1', record_type: 'ir_fragment' } },
        { node_id: 'fragment:norm-1', kind: 'normalized', fragment_id: 'norm-1', cas_id: 'norm-1', mime_type: 'application/vnd.ikam.normalized+json', label: 'norm-1', meta: { artifact_id: 'artifact:run-normalize', record_type: 'normalized_fragment' } },
      ],
      edges: [
        { from: 'artifact:run-normalize', to: 'fragment:surface-1', relation: 'contains', step_name: 'map.conceptual.lift.surface_fragments' },
        { from: 'fragment:surface-1', to: 'fragment:ir-1', relation: 'lifted_from', step_name: 'map.conceptual.normalize.discovery' },
        { from: 'fragment:ir-1', to: 'fragment:norm-1', relation: 'normalized_to', step_name: 'map.reconstructable.normalize' },
      ],
    },
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-normalize',
          project_id: 'proj-normalize',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 10, unique_bytes: 10, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'compression-rerender/v1', pipeline_run_id: 'pipe-normalize' } },
          },
        },
      ]}
      activeRunId="run-normalize"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(screen.getByTestId('execution-environment-panel')).toBeInTheDocument());
  expect(screen.getByText(/Selected step: map\.reconstructable\.normalize/i)).toBeInTheDocument();
  expect(within(screen.getByTestId('fragment-section-input')).getByText('ir-1')).toBeInTheDocument();
  expect(within(screen.getByTestId('fragment-section-output')).getByText('norm-1')).toBeInTheDocument();
});

test('keeps Resume and Next Step disabled unless backend marks controls available', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-2',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-2',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: {
      can_resume: false,
      can_next_step: false,
    },
    events: [
      {
        event_id: 'ev-10',
        step_id: 'step-10',
        step_name: 'init.initialize',
        status: 'running',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-2',
      },
    ],
  });
  mockGetScopedFragments.mockResolvedValue({ status: 'ok', fragments: [] });
  mockGetEnvironmentSummary.mockResolvedValue({
    status: 'ok',
    summary: { fragment_count: 0, verification_count: 0, reconstruction_program_count: 0 },
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-2',
    run_id: 'run-2',
    step_id: 'step-10',
    step_name: 'init.initialize',
    attempt_index: 1,
    outcome: { status: 'running', duration_ms: 0, env_type: 'dev', env_id: 'dev-2' },
    why: { summary: 'ready', policy: { name: 'init.initialize', params: {} } },
    inputs: { artifact_ids: ['artifact:run-2'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: ['artifact:run-2'], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    lineage: {
      roots: ['artifact:run-2'],
      nodes: [{ node_id: 'artifact:run-2', kind: 'artifact', fragment_id: 'artifact:run-2', cas_id: null, mime_type: 'text/markdown', label: 'artifact:run-2', meta: {} }],
      edges: [],
    },
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-2',
          project_id: 'proj-2',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'compression-rerender/v1', pipeline_run_id: 'pipe-2' } },
          },
        },
      ]}
      activeRunId="run-2"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(screen.getByTestId('debug-step-list')).toBeInTheDocument());
  expect(screen.getByRole('button', { name: 'Resume' })).toBeDisabled();
  expect(screen.getByRole('button', { name: 'Next Step' })).toBeDisabled();
});

test('shows explicit error when step detail request fails', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-err',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-err',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: {
      can_resume: true,
      can_next_step: true,
    },
    events: [
      {
        event_id: 'ev-err',
        step_id: 'step-err',
        step_name: 'map.conceptual.lift.surface_fragments',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-err',
      },
    ],
  });
  mockGetDebugStepDetail.mockRejectedValue(new Error('Request failed (500)'));

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-err',
          project_id: 'proj-err',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'compression-rerender/v1', pipeline_run_id: 'pipe-err' } },
          },
        },
      ]}
      activeRunId="run-err"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => {
    expect(mockGetDebugStepDetail).toHaveBeenCalled();
  });
  expect(screen.getByText('Failed to load step detail. Check backend logs for /debug-step/{step_id}/detail.')).toBeInTheDocument();
});

test('renders compact drill-through labels and requests artifact preview', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-viewer',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-viewer',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: {
      can_resume: true,
      can_next_step: true,
    },
    events: [
      {
        event_id: 'ev-viewer',
        step_id: 'step-viewer',
        step_name: 'map.conceptual.lift.surface_fragments',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-viewer',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-viewer',
    run_id: 'run-viewer',
    step_id: 'step-viewer',
    step_name: 'map.conceptual.lift.surface_fragments',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 20, env_type: 'dev', env_id: 'dev-viewer' },
    why: { summary: 'map', policy: { name: 'map.conceptual.lift.surface_fragments', params: {} } },
    inputs: { artifact_ids: ['proj:customer-personas.md'], fragment_ids: [], program_ids: [] },
    outputs: {
      artifact_ids: [],
      fragment_ids: ['frag-1'],
      program_ids: [],
      pair_ids: [],
      decomposition: {
        structural_fragment_ids: ['frag-1'],
        root_fragment_ids: ['frag-1', 'root-rel-1'],
        boundary_diagnostics: [
          {
            artifact_id: 'proj:customer-personas.md',
            file_name: 'customer-personas.md',
            mime_type: 'text/markdown',
            boundary_count: 1,
            avg_chunk_chars: 120,
            max_chunk_chars: 120,
            structural_coverage_ratio: 1,
            status: 'coarse',
            status_reason: 'single section',
            policy_version: 'v1',
            chunk_distribution: [120],
          },
        ],
      },
    },
    checks: [],
    lineage: {
      roots: ['artifact:proj:customer-personas.md'],
      nodes: [
        {
          node_id: 'artifact:proj:customer-personas.md',
          kind: 'artifact',
          fragment_id: 'proj:customer-personas.md',
          cas_id: null,
          mime_type: 'text/markdown',
          label: 'customer-personas.md',
          meta: { filename: 'customer-personas.md', record_type: 'artifact' },
        },
        {
          node_id: 'fragment:frag-1',
          kind: 'surface',
          fragment_id: 'frag-1',
          cas_id: 'cas-1',
          mime_type: 'text/ikam-paragraph',
          label: 'frag-1',
          meta: { file_name: 'customer-personas.md', record_type: 'surface_fragment' },
        },
      ],
      edges: [{ from: 'artifact:proj:customer-personas.md', to: 'fragment:frag-1', relation: 'contains', step_name: 'map.conceptual.lift.surface_fragments' }],
    },
  });
  mockGetArtifactPreview.mockResolvedValue({
    kind: 'text',
    mime_type: 'text/markdown',
    file_name: 'customer-personas.md',
    metadata: { size_bytes: 12 },
    preview: { text: '# Personas' },
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-viewer',
          project_id: 'proj-viewer',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'compression-rerender/v1', pipeline_run_id: 'pipe-viewer' } },
          },
        },
      ]}
      activeRunId="run-viewer"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => {
    expect(screen.getByTestId('fragment-explorer')).toBeInTheDocument();
  });

  expect(screen.getByTestId('fragment-explorer')).toBeInTheDocument();
  expect(screen.getByTestId('execution-environment-panel')).toBeInTheDocument();
  expect(screen.getByText(/Selected step: map\.conceptual\.lift\.surface_fragments/i)).toBeInTheDocument();
  expect(screen.getByText(/Step Status/i)).toBeInTheDocument();
  expect(screen.getAllByText(/succeeded/i).length).toBeGreaterThan(0);
  expect(screen.getByText('Fragment Explorer')).toBeInTheDocument();

  await waitFor(() => {
    expect(mockGetArtifactPreview).toHaveBeenCalledWith({ runId: 'run-viewer', artifactId: 'proj:customer-personas.md' });
  });
});

test('renders map toc from backend lineage roots and nodes', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-map-toc',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-map-toc',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: {
      can_resume: true,
      can_next_step: true,
    },
    events: [
      {
        event_id: 'ev-map-toc',
        step_id: 'step-map-toc',
        step_name: 'map.conceptual.lift.surface_fragments',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-map-toc',
      },
    ],
  });

  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'compression-rerender/v1',
    pipeline_run_id: 'pipe-map-toc',
    run_id: 'run-map-toc',
    step_id: 'step-map-toc',
    step_name: 'map.conceptual.lift.surface_fragments',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 24, env_type: 'dev', env_id: 'dev-map-toc' },
    why: { summary: 'map', policy: { name: 'map.conceptual.lift.surface_fragments', params: {} } },
    inputs: { artifact_ids: ['proj:deck.docx'], fragment_ids: [], program_ids: [] },
    outputs: {
      artifact_ids: [],
      fragment_ids: ['frag-docx-1'],
      program_ids: [],
      pair_ids: [],
      map: {
        status: 'ok',
        root_node_id: 'map:run-map-toc:root',
        preview_mode_default: 'semantic_map',
        node_summaries: {
          'map:run-map-toc:root': 'Corpus summary: one artifact, one section, one surface fragment.',
          'map:artifact:proj:deck.docx': 'Artifact summary: deck.docx includes an executive summary section.',
          'map:surface:frag-docx-1': 'Section summary: Executive Summary with YoY growth highlights.',
        },
        node_constituents: {
          'map:run-map-toc:root': ['frag-docx-1'],
          'map:artifact:proj:deck.docx': ['frag-docx-1'],
          'map:surface:frag-docx-1': ['frag-docx-1'],
        },
        relationships: [
          { type: 'map_contains', source: 'map:run-map-toc:root', target: 'map:artifact:proj:deck.docx' },
          { type: 'map_contains', source: 'map:artifact:proj:deck.docx', target: 'map:surface:frag-docx-1' },
        ],
      },
    },
    checks: [],
    lineage: {
      roots: ['artifact:proj:deck.docx', 'map:run-map-toc:root'],
      nodes: [
        {
          node_id: 'artifact:proj:deck.docx',
          kind: 'artifact',
          fragment_id: 'proj:deck.docx',
          cas_id: null,
          mime_type: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
          label: 'deck.docx',
          meta: { filename: 'deck.docx', record_type: 'artifact' },
        },
        {
          node_id: 'fragment:frag-docx-1',
          kind: 'surface',
          fragment_id: 'frag-docx-1',
          cas_id: 'cas-docx-1',
          mime_type: 'text/ikam-paragraph',
          label: 'frag-docx-1',
          meta: {
            artifact_id: 'proj:deck.docx',
            record_type: 'surface_fragment',
            value_preview: 'Executive Summary\n\nRevenue expanded 23% year-over-year.',
          },
        },
        {
          node_id: 'map:run-map-toc:root',
          kind: 'map_root',
          fragment_id: null,
          cas_id: null,
          mime_type: 'application/vnd.ikam.map-node+json',
          label: 'Corpus Outline',
          meta: { record_type: 'map_node', kind: 'corpus', level: 0, parent_id: null },
        },
        {
          node_id: 'map:artifact:proj:deck.docx',
          kind: 'map_node',
          fragment_id: null,
          cas_id: null,
          mime_type: 'application/vnd.ikam.map-node+json',
          label: 'deck.docx',
          meta: { record_type: 'map_node', kind: 'artifact', level: 1, parent_id: 'map:run-map-toc:root' },
        },
        {
          node_id: 'map:surface:frag-docx-1',
          kind: 'map_node',
          fragment_id: null,
          cas_id: null,
          mime_type: 'application/vnd.ikam.map-node+json',
          label: 'Executive Summary',
          meta: { record_type: 'map_node', kind: 'surface_fragment', level: 2, parent_id: 'map:artifact:proj:deck.docx' },
        },
      ],
      edges: [
        { from: 'artifact:proj:deck.docx', to: 'fragment:frag-docx-1', relation: 'contains', step_name: 'map.conceptual.lift.surface_fragments' },
        { from: 'map:run-map-toc:root', to: 'map:artifact:proj:deck.docx', relation: 'map_contains', step_name: 'map.conceptual.lift.surface_fragments' },
        { from: 'map:artifact:proj:deck.docx', to: 'map:surface:frag-docx-1', relation: 'map_contains', step_name: 'map.conceptual.lift.surface_fragments' },
        { from: 'map:artifact:proj:deck.docx', to: 'artifact:proj:deck.docx', relation: 'map_to_artifact', step_name: 'map.conceptual.lift.surface_fragments' },
        { from: 'map:surface:frag-docx-1', to: 'fragment:frag-docx-1', relation: 'map_to_surface', step_name: 'map.conceptual.lift.surface_fragments' },
      ],
    },
  });
  mockGetArtifactPreview.mockResolvedValue({
    kind: 'doc',
    mime_type: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
    file_name: 'deck.docx',
    metadata: { size_bytes: 101 },
    preview: { paragraphs: ['Executive Summary'] },
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-map-toc',
          project_id: 'proj-map-toc',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'compression-rerender/v1', pipeline_run_id: 'pipe-map-toc' } },
          },
        },
      ]}
      activeRunId="run-map-toc"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => {
    expect(screen.getByTestId('fragment-explorer')).toBeInTheDocument();
  });

  expect(screen.getByText(/Selected step: map\.conceptual\.lift\.surface_fragments/i)).toBeInTheDocument();
  expect(screen.getByTestId('execution-environment-panel')).toBeInTheDocument();
  expect(mockGetArtifactPreview).not.toHaveBeenCalled();
});

test('shows inspection unavailable when chunk set explorer data lacks hydrated drill state', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-chunk-distinguishable-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-chunk-distinguishable-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-chunk-distinguishable-ui',
        step_id: 'step-chunk-distinguishable-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-chunk-distinguishable-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-chunk-distinguishable-ui',
    run_id: 'run-chunk-distinguishable-ui',
    step_id: 'step-chunk-distinguishable-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 20, env_type: 'dev', env_id: 'dev-chunk-distinguishable-ui' },
    why: { summary: 'Chunked document', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: {
              kind: 'chunk_extraction_set',
              source_subgraph_ref: 'hot://run-chunk-distinguishable-ui/document_set/step-load',
              subgraph_ref: 'hot://run-chunk-distinguishable-ui/chunk_extraction_set/step-chunk-distinguishable-ui',
              extraction_refs: ['frag-chunk-distinguishable-1', 'frag-chunk-distinguishable-2'],
            },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 2 refs',
              refs: ['frag-chunk-distinguishable-1', 'frag-chunk-distinguishable-2'],
              content: {
                kind: 'chunk_extraction_set',
                source_subgraph_ref: 'hot://run-chunk-distinguishable-ui/document_set/step-load',
                subgraph_ref: 'hot://run-chunk-distinguishable-ui/chunk_extraction_set/step-chunk-distinguishable-ui',
                extraction_refs: ['frag-chunk-distinguishable-1', 'frag-chunk-distinguishable-2'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-chunk-distinguishable-1',
                  cas_id: 'frag-chunk-distinguishable-1',
                  mime_type: 'application/vnd.ikam.chunk-extraction+json',
                  name: 'alpha.md',
                  inspection_ref: 'inspect://fragment/frag-chunk-distinguishable-1',
                  value: {
                    chunk_id: 'doc-ui-distinguishable:chunk:0',
                    document_id: 'doc-ui-distinguishable',
                    source_document_fragment_id: 'frag-doc-ui-distinguishable',
                    artifact_id: 'artifact:alpha-md',
                    filename: 'alpha.md',
                    text: 'alpha one',
                    span: { start: 0, end: 9 },
                    order: 0,
                  },
                },
                {
                  fragment_id: 'frag-chunk-distinguishable-2',
                  cas_id: 'frag-chunk-distinguishable-2',
                  mime_type: 'application/vnd.ikam.chunk-extraction+json',
                  name: 'alpha.md',
                  inspection_ref: 'inspect://fragment/frag-chunk-distinguishable-2',
                  value: {
                    chunk_id: 'doc-ui-distinguishable:chunk:1',
                    document_id: 'doc-ui-distinguishable',
                    source_document_fragment_id: 'frag-doc-ui-distinguishable',
                    artifact_id: 'artifact:alpha-md',
                    filename: 'alpha.md',
                    text: 'alpha two',
                    span: { start: 10, end: 19 },
                    order: 1,
                  },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-chunk-distinguishable-ui',
          project_id: 'proj-chunk-distinguishable-ui',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-chunk-distinguishable-ui' } },
          },
        },
      ]}
      activeRunId="run-chunk-distinguishable-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());

  const outputSection = within(screen.getByTestId('fragment-section-output'));
  expect(outputSection.getByRole('button', { name: /chunk_extraction_set .*step-chunk-distinguishable-ui/i })).toBeInTheDocument();

  fireEvent.click(outputSection.getByRole('button', { name: /chunk_extraction_set .*step-chunk-distinguishable-ui/i }));

  const explorer = within(screen.getByTestId('fragment-explorer'));
  await waitFor(() => expect(explorer.getByText(/Inspection unavailable/i)).toBeInTheDocument());
});

test('preserves parent set context during chunk to document drill through', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-chunk-parent-context-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-chunk-parent-context-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-chunk-parent-context-ui',
        step_id: 'step-chunk-parent-context-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-chunk-parent-context-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-chunk-parent-context-ui',
    run_id: 'run-chunk-parent-context-ui',
    step_id: 'step-chunk-parent-context-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 22, env_type: 'dev', env_id: 'dev-chunk-parent-context-ui' },
    why: { summary: 'Chunked document', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: {
              kind: 'chunk_extraction_set',
              source_subgraph_ref: 'hot://run-chunk-parent-context-ui/document_set/step-load',
              subgraph_ref: 'hot://run-chunk-parent-context-ui/chunk_extraction_set/step-chunk-parent-context-ui',
              extraction_refs: ['frag-chunk-parent-1'],
            },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 1 ref',
              refs: ['frag-chunk-parent-1'],
              content: {
                kind: 'chunk_extraction_set',
                source_subgraph_ref: 'hot://run-chunk-parent-context-ui/document_set/step-load',
                subgraph_ref: 'hot://run-chunk-parent-context-ui/chunk_extraction_set/step-chunk-parent-context-ui',
                extraction_refs: ['frag-chunk-parent-1'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-chunk-parent-1',
                  cas_id: 'frag-chunk-parent-1',
                  mime_type: 'application/vnd.ikam.chunk-extraction+json',
                  name: 'doc-parent-1:chunk:0',
                  inspection_ref: 'inspect://fragment/frag-chunk-parent-1',
                  value: {
                    chunk_id: 'doc-parent-1:chunk:0',
                    document_id: 'doc-parent-1',
                    source_document_fragment_id: 'frag-doc-parent-1',
                    artifact_id: 'artifact:alpha-md',
                    filename: 'alpha.md',
                    text: 'alpha body',
                    span: { start: 0, end: 10 },
                    order: 0,
                  },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockImplementation(async ({ ref }: { ref: string }) => {
    if (ref === 'inspect://fragment/frag-chunk-parent-1') {
      return {
        schema_version: 'v1',
        root_node_id: 'fragment:frag-chunk-parent-1',
        navigation: { focus: { node_id: 'fragment:frag-chunk-parent-1' } },
        nodes: [
          {
            id: 'fragment:frag-chunk-parent-1',
            kind: 'fragment',
            ir_kind: 'chunk_extraction',
            label: 'doc-parent-1:chunk:0',
            payload: {
              cas_id: 'frag-chunk-parent-1',
              mime_type: 'application/vnd.ikam.chunk-extraction+json',
              value: {
                chunk_id: 'doc-parent-1:chunk:0',
                document_id: 'doc-parent-1',
                source_document_fragment_id: 'frag-doc-parent-1',
                artifact_id: 'artifact:alpha-md',
                filename: 'alpha.md',
                text: 'alpha body',
                span: { start: 0, end: 10 },
                order: 0,
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-chunk-parent-1' }, hint: null } },
            provenance: { source_backend: 'hot' },
          },
          {
            id: 'fragment:frag-doc-parent-1',
            kind: 'fragment',
            ir_kind: 'document',
            label: 'alpha.md',
            payload: {
              cas_id: 'frag-doc-parent-1',
              mime_type: 'application/vnd.ikam.loaded-document+json',
              value: {
                document_id: 'doc-parent-1',
                artifact_id: 'artifact:alpha-md',
                filename: 'alpha.md',
                text: 'alpha body full',
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-doc-parent-1' }, hint: null } },
            provenance: { source_backend: 'hot' },
          },
          {
            id: 'subgraph:hot://run-chunk-parent-context-ui/chunk_extraction_set/step-chunk-parent-context-ui',
            kind: 'subgraph',
            ir_kind: 'chunk_extraction_set',
            label: 'hot://run-chunk-parent-context-ui/chunk_extraction_set/step-chunk-parent-context-ui',
            payload: {},
            refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-chunk-parent-context-ui/chunk_extraction_set/step-chunk-parent-context-ui' }, hint: null } },
            provenance: { source_backend: 'hot' },
          },
        ],
        edges: [
          { id: 'edge:derives', from: 'fragment:frag-chunk-parent-1', to: 'fragment:frag-doc-parent-1', relation: 'derives', label: null, summary: null, payload: {}, refs: {}, provenance: { source_backend: 'hot' }, capabilities: {} },
          { id: 'edge:contains', from: 'subgraph:hot://run-chunk-parent-context-ui/chunk_extraction_set/step-chunk-parent-context-ui', to: 'fragment:frag-chunk-parent-1', relation: 'contains', label: null, summary: null, payload: {}, refs: {}, provenance: { source_backend: 'hot' }, capabilities: {} },
        ],
      };
    }
    if (ref === 'inspect://fragment/frag-doc-parent-1') {
      return {
        schema_version: 'v1',
        root_node_id: 'fragment:frag-doc-parent-1',
        navigation: { focus: { node_id: 'fragment:frag-doc-parent-1' } },
        nodes: [
          {
            id: 'fragment:frag-doc-parent-1',
            kind: 'fragment',
            ir_kind: 'document',
            label: 'alpha.md',
            payload: {
              cas_id: 'frag-doc-parent-1',
              mime_type: 'application/vnd.ikam.loaded-document+json',
              value: {
                document_id: 'doc-parent-1',
                artifact_id: 'artifact:alpha-md',
                filename: 'alpha.md',
                text: 'alpha body full',
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-doc-parent-1' }, hint: null } },
            provenance: { source_backend: 'hot' },
          },
          {
            id: 'subgraph:hot://run-chunk-parent-context-ui/document_set/step-load',
            kind: 'subgraph',
            ir_kind: 'document_set',
            label: 'hot://run-chunk-parent-context-ui/document_set/step-load',
            payload: {},
            refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-chunk-parent-context-ui/document_set/step-load' }, hint: null } },
            provenance: { source_backend: 'hot' },
          },
          {
            id: 'fragment:frag-doc-chunk-set-parent-1',
            kind: 'fragment',
            ir_kind: 'json',
            label: 'document_chunk_set',
            payload: {
              cas_id: 'frag-doc-chunk-set-parent-1',
              mime_type: 'application/vnd.ikam.document-chunk-set+json',
              value: {
                kind: 'document_chunk_set',
                document_id: 'doc-parent-1',
                source_document_fragment_id: 'frag-doc-parent-1',
                chunk_refs: ['frag-chunk-parent-1'],
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-doc-chunk-set-parent-1' }, hint: null } },
            provenance: { source_backend: 'hot' },
          },
        ],
        edges: [
          { id: 'edge:document-set-contains', from: 'subgraph:hot://run-chunk-parent-context-ui/document_set/step-load', to: 'fragment:frag-doc-parent-1', relation: 'contains', label: null, summary: null, payload: {}, refs: {}, provenance: { source_backend: 'hot' }, capabilities: {} },
          { id: 'edge:document-refers-group', from: 'fragment:frag-doc-chunk-set-parent-1', to: 'fragment:frag-doc-parent-1', relation: 'references', label: null, summary: null, payload: {}, refs: {}, provenance: { source_backend: 'hot' }, capabilities: {} },
        ],
      };
    }
    throw new Error(`Unexpected inspection ref: ${ref}`);
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-chunk-parent-context-ui',
          project_id: 'proj-chunk-parent-context-ui',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-chunk-parent-context-ui' } },
          },
        },
      ]}
      activeRunId="run-chunk-parent-context-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());

  const outputSection = within(screen.getByTestId('fragment-section-output'));
  fireEvent.click(outputSection.getByRole('button', { name: /chunk_extraction_set .*step-chunk-parent-context-ui/i }));

  const explorer = within(screen.getByTestId('fragment-explorer'));
  await waitFor(() => expect(explorer.getByText('chunk_extraction_set 1 ref')).toBeInTheDocument());
  fireEvent.click(explorer.getByRole('button', { name: /doc-parent-1:chunk:0/i }));
  await waitFor(() => expect(mockGetInspectionSubgraph).toHaveBeenCalledWith({
    runId: 'run-chunk-parent-context-ui',
    ref: 'inspect://fragment/frag-chunk-parent-1',
    maxDepth: 1,
  }));
  const chunkConnectedList = within(screen.getByTestId('connected-fragments-list'));
  expect(explorer.getByText('Connected Fragments')).toBeInTheDocument();
  expect(chunkConnectedList.getByRole('button', { name: /chunk_extraction_set/i })).toBeInTheDocument();
  expect(chunkConnectedList.getByRole('button', { name: /alpha\.md/i })).toBeInTheDocument();

  fireEvent.click(chunkConnectedList.getByRole('button', { name: /alpha\.md/i }));
  await waitFor(() => expect(mockGetInspectionSubgraph).toHaveBeenCalledWith({
    runId: 'run-chunk-parent-context-ui',
    ref: 'inspect://fragment/frag-doc-parent-1',
    maxDepth: 1,
  }));
  const documentConnectedList = within(screen.getByTestId('connected-fragments-list'));
  expect(documentConnectedList.getByRole('button', { name: /doc-parent-1:chunk:0/i })).toBeInTheDocument();
  expect(documentConnectedList.getByRole('button', { name: /chunk_extraction_set/i })).toBeInTheDocument();
  expect(documentConnectedList.getByRole('button', { name: /document_set/i })).toBeInTheDocument();
  expect(documentConnectedList.getByRole('button', { name: /document_chunk_set/i })).toBeInTheDocument();
});

test('run fragment graph renders for document_set drill selections that only expose subgraph focus', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-document-set-graph-focus-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-document-set-graph-focus-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-document-set-graph-focus-ui',
        step_id: 'step-document-set-graph-focus-ui',
        step_name: 'load.documents',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-document-set-graph-focus-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-document-set-graph-focus-ui',
    run_id: 'run-document-set-graph-focus-ui',
    step_id: 'step-document-set-graph-focus-ui',
    step_name: 'load.documents',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 772, env_type: 'dev', env_id: 'dev-document-set-graph-focus-ui' },
    why: { summary: 'Loaded documents', policy: { name: 'load.documents', params: {} } },
    inputs: { artifact_ids: ['artifact:source-docs'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-document-set', direction: 'output', kind: 'type', config: { schema: { title: 'document_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        document_set: [
          {
            value: {
              kind: 'document_set',
              subgraph_ref: 'hot://run-document-set-graph-focus-ui/document_set/step-load',
              document_refs: ['frag-document-set-doc-1'],
            },
            inspection: {
              value_kind: 'document_set',
              summary: 'document_set 1 ref',
              refs: ['frag-document-set-doc-1'],
              content: {
                kind: 'document_set',
                subgraph_ref: 'hot://run-document-set-graph-focus-ui/document_set/step-load',
                document_refs: ['frag-document-set-doc-1'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-document-set-doc-1',
                  cas_id: 'frag-document-set-doc-1',
                  mime_type: 'application/vnd.ikam.loaded-document+json',
                  name: 'quarterly-report.md',
                  inspection_ref: 'inspect://fragment/frag-document-set-doc-1',
                  value: {
                    document_id: 'doc-document-set-1',
                    filename: 'quarterly-report.md',
                  },
                },
              ],
              subgraph: {
                schema_version: 'v1',
                root_node_id: 'subgraph:hot://run-document-set-graph-focus-ui/document_set/step-load',
                navigation: { focus: { node_id: 'subgraph:hot://run-document-set-graph-focus-ui/document_set/step-load' } },
                nodes: [
                  {
                    id: 'subgraph:hot://run-document-set-graph-focus-ui/document_set/step-load',
                    kind: 'subgraph',
                    ir_kind: 'document_set',
                    label: 'hot://run-document-set-graph-focus-ui/document_set/step-load',
                    payload: {},
                    refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-document-set-graph-focus-ui/document_set/step-load' } } },
                  },
                  {
                    id: 'fragment:frag-document-set-doc-1',
                    kind: 'fragment',
                    ir_kind: 'document',
                    label: 'quarterly-report.md',
                    payload: {
                      cas_id: 'frag-document-set-doc-1',
                      value: {
                        document_id: 'doc-document-set-1',
                        filename: 'quarterly-report.md',
                      },
                    },
                    refs: { self: { backend: 'hot', locator: { cas_id: 'frag-document-set-doc-1' } } },
                  },
                ],
                edges: [
                  {
                    id: 'edge-document-set-focus-1',
                    from: 'subgraph:hot://run-document-set-graph-focus-ui/document_set/step-load',
                    to: 'fragment:frag-document-set-doc-1',
                    relation: 'contains',
                  },
                ],
              },
            },
          },
        ],
      },
      results: [
        { name: 'output-document-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.document_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockResolvedValue({
    schema_version: 'v1',
    root_node_id: 'fragment:frag-document-set-doc-1',
    navigation: { focus: { node_id: 'fragment:frag-document-set-doc-1' } },
    nodes: [
      {
        id: 'fragment:frag-document-set-doc-1',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'quarterly-report.md',
        payload: {
          cas_id: 'frag-document-set-doc-1',
          value: {
            document_id: 'doc-document-set-1',
            filename: 'quarterly-report.md',
          },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-document-set-doc-1' } } },
      },
      {
        id: 'subgraph:hot://run-document-set-graph-focus-ui/document_set/step-load',
        kind: 'subgraph',
        ir_kind: 'document_set',
        label: 'hot://run-document-set-graph-focus-ui/document_set/step-load',
        payload: {},
        refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-document-set-graph-focus-ui/document_set/step-load' } } },
      },
    ],
    edges: [
      {
        id: 'edge-document-set-focus-2',
        from: 'subgraph:hot://run-document-set-graph-focus-ui/document_set/step-load',
        to: 'fragment:frag-document-set-doc-1',
        relation: 'contains',
      },
    ],
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-document-set-graph-focus-ui',
          project_id: 'proj-document-set-graph-focus-ui',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-document-set-graph-focus-ui' } },
          },
        },
      ]}
      activeRunId="run-document-set-graph-focus-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /document_set .*step-load/i }));

  const graphPanel = await screen.findByTestId('run-fragment-graph-panel');
  expect(within(graphPanel).getByRole('button', { name: /document set/i })).toHaveClass('inspection-graph-node-button-root');
  expect(within(graphPanel).getByRole('button', { name: /quarter\.\.\./i })).toHaveAttribute('title', 'quarterly-report.md');
  expect(screen.queryByTestId('inspection-graph-panel')).not.toBeInTheDocument();
});

test('document_set drill fetches inspection subgraph when summary inspection only exposes subgraph_ref', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-document-set-hydration-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-document-set-hydration-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-document-set-hydration-ui',
        step_id: 'step-document-set-hydration-ui',
        step_name: 'load.documents',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-document-set-hydration-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-document-set-hydration-ui',
    run_id: 'run-document-set-hydration-ui',
    step_id: 'step-document-set-hydration-ui',
    step_name: 'load.documents',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 772, env_type: 'dev', env_id: 'dev-document-set-hydration-ui' },
    why: { summary: 'Loaded documents', policy: { name: 'load.documents', params: {} } },
    inputs: { artifact_ids: ['artifact:source-docs'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-document-set', direction: 'output', kind: 'type', config: { schema: { title: 'document_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        document_set: [
          {
            value: {
              kind: 'document_set',
              subgraph_ref: 'hot://run-document-set-hydration-ui/document_set/step-load',
              document_refs: ['frag-document-set-doc-1'],
            },
            inspection: {
              value_kind: 'document_set',
              summary: 'document_set 1 ref',
              refs: ['frag-document-set-doc-1'],
              content: {
                kind: 'document_set',
                subgraph_ref: 'hot://run-document-set-hydration-ui/document_set/step-load',
                document_refs: ['frag-document-set-doc-1'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-document-set-doc-1',
                  cas_id: 'frag-document-set-doc-1',
                  mime_type: 'application/vnd.ikam.loaded-document+json',
                  name: 'quarterly-report.md',
                  inspection_ref: 'inspect://fragment/frag-document-set-doc-1',
                  value: {
                    document_id: 'doc-document-set-1',
                    filename: 'quarterly-report.md',
                  },
                },
              ],
            },
            inspection_stub: {
              inspection_ref: 'inspect://subgraph/hot://run-document-set-hydration-ui/document_set/step-load',
            },
          },
        ],
      },
      results: [
        { name: 'output-document-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.document_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockResolvedValue({
    schema_version: 'v1',
    root_node_id: 'subgraph:hot://run-document-set-hydration-ui/document_set/step-load',
    navigation: { focus: { node_id: 'subgraph:hot://run-document-set-hydration-ui/document_set/step-load' } },
    nodes: [
      {
        id: 'subgraph:hot://run-document-set-hydration-ui/document_set/step-load',
        kind: 'subgraph',
        ir_kind: 'document_set',
        label: 'hot://run-document-set-hydration-ui/document_set/step-load',
        payload: {},
        refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-document-set-hydration-ui/document_set/step-load' } } },
      },
      {
        id: 'fragment:frag-document-set-doc-1',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'quarterly-report.md',
        payload: {
          cas_id: 'frag-document-set-doc-1',
          value: {
            document_id: 'doc-document-set-1',
            filename: 'quarterly-report.md',
          },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-document-set-doc-1' } } },
      },
    ],
    edges: [
      {
        id: 'edge-document-set-hydration-1',
        from: 'subgraph:hot://run-document-set-hydration-ui/document_set/step-load',
        to: 'fragment:frag-document-set-doc-1',
        relation: 'contains',
      },
    ],
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-document-set-hydration-ui',
          project_id: 'proj-document-set-hydration-ui',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-document-set-hydration-ui' } },
          },
        },
      ]}
      activeRunId="run-document-set-hydration-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /document_set .*step-load/i }));

  await waitFor(() => expect(mockGetInspectionSubgraph).toHaveBeenCalledWith({
    runId: 'run-document-set-hydration-ui',
    ref: 'inspect://subgraph/hot://run-document-set-hydration-ui/document_set/step-load',
    maxDepth: 1,
  }));
  expect(await screen.findByTestId('run-fragment-graph-panel')).toBeInTheDocument();
});

test('dense document_set graphs get a taller canvas for large fan-out views', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-dense-document-set-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-dense-document-set-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-dense-document-set-ui',
        step_id: 'step-dense-document-set-ui',
        step_name: 'load.documents',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-dense-document-set-ui',
      },
    ],
  });

  const denseResolvedRefs = Array.from({ length: 18 }, (_, index) => ({
    fragment_id: `frag-dense-doc-${index + 1}`,
    cas_id: `frag-dense-doc-${index + 1}`,
    mime_type: 'application/vnd.ikam.loaded-document+json',
    name: `dense-document-${index + 1}.md`,
    inspection_ref: `inspect://fragment/frag-dense-doc-${index + 1}`,
    value: {
      document_id: `doc-dense-${index + 1}`,
      filename: `dense-document-${index + 1}.md`,
    },
  }));

  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-dense-document-set-ui',
    run_id: 'run-dense-document-set-ui',
    step_id: 'step-dense-document-set-ui',
    step_name: 'load.documents',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 772, env_type: 'dev', env_id: 'dev-dense-document-set-ui' },
    why: { summary: 'Loaded many documents', policy: { name: 'load.documents', params: {} } },
    inputs: { artifact_ids: ['artifact:dense-source'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-document-set', direction: 'output', kind: 'type', config: { schema: { title: 'document_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        document_set: [
          {
            value: {
              kind: 'document_set',
              subgraph_ref: 'hot://run-dense-document-set-ui/document_set/step-load',
              document_refs: denseResolvedRefs.map((item) => item.fragment_id),
            },
            inspection: {
              value_kind: 'document_set',
              summary: `document_set ${denseResolvedRefs.length} refs`,
              refs: denseResolvedRefs.map((item) => item.fragment_id),
              content: {
                kind: 'document_set',
                subgraph_ref: 'hot://run-dense-document-set-ui/document_set/step-load',
                document_refs: denseResolvedRefs.map((item) => item.fragment_id),
              },
              resolved_refs: denseResolvedRefs,
              subgraph: {
                schema_version: 'v1',
                root_node_id: 'subgraph:hot://run-dense-document-set-ui/document_set/step-load',
                navigation: { focus: { node_id: 'subgraph:hot://run-dense-document-set-ui/document_set/step-load' } },
                nodes: [
                  {
                    id: 'subgraph:hot://run-dense-document-set-ui/document_set/step-load',
                    kind: 'subgraph',
                    ir_kind: 'document_set',
                    label: 'hot://run-dense-document-set-ui/document_set/step-load',
                    payload: {},
                    refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-dense-document-set-ui/document_set/step-load' } } },
                  },
                  ...denseResolvedRefs.map((item) => ({
                    id: `fragment:${item.fragment_id}`,
                    kind: 'fragment',
                    ir_kind: 'document',
                    label: item.name,
                    payload: {
                      cas_id: item.cas_id,
                      value: item.value,
                    },
                    refs: { self: { backend: 'hot', locator: { cas_id: item.cas_id } } },
                  })),
                ],
                edges: denseResolvedRefs.map((item, index) => ({
                  id: `edge-dense-document-set-${index + 1}`,
                  from: 'subgraph:hot://run-dense-document-set-ui/document_set/step-load',
                  to: `fragment:${item.fragment_id}`,
                  relation: 'contains',
                })),
              },
            },
          },
        ],
      },
      results: [
        { name: 'output-document-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.document_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockResolvedValue({ schema_version: 'v1', root_node_id: 'unused', nodes: [], edges: [] });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-dense-document-set-ui',
          project_id: 'proj-dense-document-set-ui',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-dense-document-set-ui' } },
          },
        },
      ]}
      activeRunId="run-dense-document-set-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /document_set .*step-load/i }));

  const graphCanvas = await within(await screen.findByTestId('run-fragment-graph-panel')).findByTestId('run-fragment-graph-canvas');
  expect(graphCanvas).toHaveStyle({ height: '520px' });
  await waitFor(() => expect(mockFitView).toHaveBeenCalled());
  expect(mockFitView).toHaveBeenLastCalledWith(expect.objectContaining({ padding: 0.4, maxZoom: 0.75 }));
});

test('inspection graph renders loaded subgraph and drills through on node click', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-inspection-graph-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-inspection-graph-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-inspection-graph-ui',
        step_id: 'step-inspection-graph-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-inspection-graph-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-inspection-graph-ui',
    run_id: 'run-inspection-graph-ui',
    step_id: 'step-inspection-graph-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 22, env_type: 'dev', env_id: 'dev-inspection-graph-ui' },
    why: { summary: 'Chunked document', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: {
              kind: 'chunk_extraction_set',
              source_subgraph_ref: 'hot://run-inspection-graph-ui/document_set/step-load',
              subgraph_ref: 'hot://run-inspection-graph-ui/chunk_extraction_set/step-inspection-graph-ui',
              extraction_refs: ['frag-chunk-graph-1'],
            },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 1 ref',
              refs: ['frag-chunk-graph-1'],
              content: {
                kind: 'chunk_extraction_set',
                source_subgraph_ref: 'hot://run-inspection-graph-ui/document_set/step-load',
                subgraph_ref: 'hot://run-inspection-graph-ui/chunk_extraction_set/step-inspection-graph-ui',
                extraction_refs: ['frag-chunk-graph-1'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-chunk-graph-1',
                  cas_id: 'frag-chunk-graph-1',
                  mime_type: 'application/vnd.ikam.chunk-extraction+json',
                  name: 'doc-graph-1:chunk:0',
                  inspection_ref: 'inspect://fragment/frag-chunk-graph-1',
                  value: {
                    chunk_id: 'doc-graph-1:chunk:0',
                    document_id: 'doc-graph-1',
                    source_document_fragment_id: 'frag-doc-graph-1',
                    artifact_id: 'artifact:alpha-md',
                    filename: 'alpha.md',
                    text: 'alpha body',
                    span: { start: 0, end: 10 },
                    order: 0,
                  },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockImplementation(async ({ ref }: { ref: string }) => {
    if (ref === 'inspect://fragment/frag-chunk-graph-1') {
      return {
        schema_version: 'v1',
        root_node_id: 'fragment:frag-chunk-graph-1',
        navigation: { focus: { node_id: 'fragment:frag-chunk-graph-1' } },
        nodes: [
          {
            id: 'fragment:frag-chunk-graph-1',
            kind: 'fragment',
            ir_kind: 'chunk_extraction',
            label: 'doc-graph-1:chunk:0',
            payload: {
              cas_id: 'frag-chunk-graph-1',
              mime_type: 'application/vnd.ikam.chunk-extraction+json',
              value: {
                chunk_id: 'doc-graph-1:chunk:0',
                document_id: 'doc-graph-1',
                source_document_fragment_id: 'frag-doc-graph-1',
                artifact_id: 'artifact:alpha-md',
                filename: 'alpha.md',
                text: 'alpha body',
                span: { start: 0, end: 10 },
                order: 0,
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-chunk-graph-1' }, hint: null } },
            provenance: { source_backend: 'hot' },
          },
          {
            id: 'fragment:frag-doc-graph-1',
            kind: 'fragment',
            ir_kind: 'document',
            label: 'alpha.md',
            payload: {
              cas_id: 'frag-doc-graph-1',
              mime_type: 'application/vnd.ikam.loaded-document+json',
              value: {
                document_id: 'doc-graph-1',
                artifact_id: 'artifact:alpha-md',
                filename: 'alpha.md',
                text: 'alpha body full',
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-doc-graph-1' }, hint: null } },
            provenance: { source_backend: 'hot' },
          },
          {
            id: 'subgraph:hot://run-inspection-graph-ui/chunk_extraction_set/step-inspection-graph-ui',
            kind: 'subgraph',
            ir_kind: 'chunk_extraction_set',
            label: 'hot://run-inspection-graph-ui/chunk_extraction_set/step-inspection-graph-ui',
            payload: {},
            refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-inspection-graph-ui/chunk_extraction_set/step-inspection-graph-ui' }, hint: null } },
            provenance: { source_backend: 'hot' },
          },
        ],
        edges: [
          { id: 'edge:chunk-derives-doc', from: 'fragment:frag-chunk-graph-1', to: 'fragment:frag-doc-graph-1', relation: 'derives', label: null, summary: null, payload: {}, refs: {}, provenance: { source_backend: 'hot' }, capabilities: {} },
          { id: 'edge:set-contains-chunk', from: 'subgraph:hot://run-inspection-graph-ui/chunk_extraction_set/step-inspection-graph-ui', to: 'fragment:frag-chunk-graph-1', relation: 'contains', label: null, summary: null, payload: {}, refs: {}, provenance: { source_backend: 'hot' }, capabilities: {} },
        ],
      };
    }
    if (ref === 'inspect://fragment/frag-doc-graph-1') {
      return {
        schema_version: 'v1',
        root_node_id: 'fragment:frag-doc-graph-1',
        navigation: { focus: { node_id: 'fragment:frag-doc-graph-1' } },
        nodes: [
          {
            id: 'fragment:frag-doc-graph-1',
            kind: 'fragment',
            ir_kind: 'document',
            label: 'alpha.md',
            payload: {
              cas_id: 'frag-doc-graph-1',
              mime_type: 'application/vnd.ikam.loaded-document+json',
              value: {
                document_id: 'doc-graph-1',
                artifact_id: 'artifact:alpha-md',
                filename: 'alpha.md',
                text: 'alpha body full',
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-doc-graph-1' }, hint: null } },
            provenance: { source_backend: 'hot' },
          },
        ],
        edges: [],
      };
    }
    throw new Error(`Unexpected inspection ref: ${ref}`);
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-inspection-graph-ui',
          project_id: 'proj-inspection-graph-ui',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-inspection-graph-ui' } },
          },
        },
      ]}
      activeRunId="run-inspection-graph-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());

  const outputSection = within(screen.getByTestId('fragment-section-output'));
  fireEvent.click(outputSection.getByRole('button', { name: /chunk_extraction_set .*step-inspection-graph-ui/i }));

  const explorer = within(screen.getByTestId('fragment-explorer'));
  await waitFor(() => expect(explorer.getByText('chunk_extraction_set 1 ref')).toBeInTheDocument());
  fireEvent.click(explorer.getByRole('button', { name: /doc-graph-1:chunk:0/i }));

  await waitFor(() => expect(mockGetInspectionSubgraph).toHaveBeenCalledWith({
    runId: 'run-inspection-graph-ui',
    ref: 'inspect://fragment/frag-chunk-graph-1',
    maxDepth: 1,
  }));

  const graphPanel = await screen.findByTestId('run-fragment-graph-panel');
  expect(within(graphPanel).getByText('Fragment Graph')).toBeInTheDocument();
  expect(within(graphPanel).getByRole('button', { name: /doc-graph-1:chunk:0/i })).toBeInTheDocument();
  expect(within(graphPanel).getByRole('button', { name: /alpha\.md/i })).toBeInTheDocument();

  fireEvent.click(within(graphPanel).getByRole('button', { name: /alpha\.md/i }));

  await waitFor(() => expect(mockGetInspectionSubgraph).toHaveBeenCalledWith({
    runId: 'run-inspection-graph-ui',
    ref: 'inspect://fragment/frag-doc-graph-1',
    maxDepth: 1,
  }));
  await waitFor(() => expect(explorer.getByText('document', { selector: '.runs-fragment-drill-summary' })).toBeInTheDocument());
  await waitFor(() => expect(within(screen.getByTestId('connected-fragments-list')).getByRole('button', { name: /doc-graph-1:chunk:0/i })).toBeInTheDocument());
});

test('inspection graph subgraph and root nodes resolve through explicit selection payloads', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-inspection-graph-root-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-inspection-graph-root-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-inspection-graph-root-ui',
        step_id: 'step-inspection-graph-root-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-inspection-graph-root-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-inspection-graph-root-ui',
    run_id: 'run-inspection-graph-root-ui',
    step_id: 'step-inspection-graph-root-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 22, env_type: 'dev', env_id: 'dev-inspection-graph-root-ui' },
    why: { summary: 'Chunked document', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: {
              kind: 'chunk_extraction_set',
              subgraph_ref: 'hot://run-inspection-graph-root-ui/chunk_extraction_set/step-inspection-graph-root-ui',
            },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 2 refs',
              refs: ['frag-root-chunk', 'inspect://subgraph/hot://run-inspection-graph-root-ui/chunk_extraction_set/step-inspection-graph-root-ui'],
              content: {
                kind: 'chunk_extraction_set',
                subgraph_ref: 'hot://run-inspection-graph-root-ui/chunk_extraction_set/step-inspection-graph-root-ui',
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-root-chunk',
                  cas_id: 'frag-root-chunk',
                  name: 'doc-root:chunk:0',
                  inspection_ref: 'inspect://fragment/frag-root-chunk',
                  value: { chunk_id: 'doc-root:chunk:0' },
                },
                {
                  fragment_id: 'hot://run-inspection-graph-root-ui/chunk_extraction_set/step-inspection-graph-root-ui',
                  cas_id: 'hot://run-inspection-graph-root-ui/chunk_extraction_set/step-inspection-graph-root-ui',
                  name: 'chunk_extraction_set',
                  inspection_ref: 'inspect://subgraph/hot://run-inspection-graph-root-ui/chunk_extraction_set/step-inspection-graph-root-ui',
                  value: { kind: 'chunk_extraction_set' },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockImplementation(async ({ ref }: { ref: string }) => {
    if (ref === 'inspect://fragment/frag-root-chunk') {
      return {
        schema_version: 'v1',
        root_node_id: 'subgraph:hot://run-inspection-graph-root-ui/chunk_extraction_set/step-inspection-graph-root-ui',
        navigation: {},
        nodes: [
          {
            id: 'subgraph:hot://run-inspection-graph-root-ui/chunk_extraction_set/step-inspection-graph-root-ui',
            kind: 'subgraph',
            ir_kind: 'chunk_extraction_set',
            label: 'hot://run-inspection-graph-root-ui/chunk_extraction_set/step-inspection-graph-root-ui',
            payload: {},
            refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-inspection-graph-root-ui/chunk_extraction_set/step-inspection-graph-root-ui' } } },
          },
          {
            id: 'fragment:frag-root-chunk',
            kind: 'fragment',
            ir_kind: 'chunk_extraction',
            label: 'doc-root:chunk:0',
            payload: { cas_id: 'frag-root-chunk', value: { chunk_id: 'doc-root:chunk:0' } },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-root-chunk' } } },
          },
        ],
        edges: [
          { id: 'edge-root', from: 'subgraph:hot://run-inspection-graph-root-ui/chunk_extraction_set/step-inspection-graph-root-ui', to: 'fragment:frag-root-chunk', relation: 'contains' },
        ],
      };
    }
    return {
      schema_version: 'v1',
      root_node_id: 'fragment:resolved',
      navigation: {},
      nodes: [{ id: 'fragment:resolved', kind: 'fragment', ir_kind: 'chunk_extraction', label: ref, payload: { cas_id: 'resolved' }, refs: { self: { backend: 'hot', locator: { cas_id: 'resolved' } } } }],
      edges: [],
    };
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[{
        run_id: 'run-inspection-graph-root-ui',
        project_id: 'proj-inspection-graph-root-ui',
        case_id: 's-local-retail-v01',
        stages: [],
        decisions: [],
        evaluation: {
          report: {
            compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
            entities: { coverage: 1, passed: true },
            predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
            exploration: { mean_recall: 1, passed: true },
            query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
            passed: true,
          },
          rendered: 'ok',
          details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-inspection-graph-root-ui' } },
        },
      }]}
      activeRunId="run-inspection-graph-root-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /chunk_extraction_set/i }));
  fireEvent.click(within(screen.getByTestId('connected-fragments-list')).getByRole('button', { name: /doc-root:chunk:0/i }));
  const graphPanel = await screen.findByTestId('run-fragment-graph-panel');

  fireEvent.click(within(graphPanel).getByRole('button', { name: /Chunk extraction set/i }));
  await waitFor(() => expect(within(screen.getByTestId('fragment-explorer')).getByText('chunk_extraction_set 2 refs')).toBeInTheDocument());
  expect(mockGetInspectionSubgraph).toHaveBeenCalledWith({
    runId: 'run-inspection-graph-root-ui',
    ref: 'inspect://fragment/frag-root-chunk',
    maxDepth: 1,
  });
});

test('inspection graph ignores unresolved node clicks safely', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-inspection-graph-noop-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-inspection-graph-noop-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-inspection-graph-noop-ui',
        step_id: 'step-inspection-graph-noop-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-inspection-graph-noop-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-inspection-graph-noop-ui',
    run_id: 'run-inspection-graph-noop-ui',
    step_id: 'step-inspection-graph-noop-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 12, env_type: 'dev', env_id: 'dev-inspection-graph-noop-ui' },
    why: { summary: 'Chunked document', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: { kind: 'chunk_extraction_set', subgraph_ref: 'hot://run-inspection-graph-noop-ui/chunk_extraction_set/step-inspection-graph-noop-ui' },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 1 ref',
              refs: ['frag-noop-chunk'],
              content: { kind: 'chunk_extraction_set' },
              resolved_refs: [
                {
                  fragment_id: 'frag-noop-chunk',
                  cas_id: 'frag-noop-chunk',
                  name: 'doc-noop:chunk:0',
                  inspection_ref: 'inspect://fragment/frag-noop-chunk',
                  value: { chunk_id: 'doc-noop:chunk:0' },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockImplementation(async ({ ref }: { ref: string }) => {
    if (ref === 'inspect://fragment/frag-noop-chunk') {
      return {
        schema_version: 'v1',
        root_node_id: 'fragment:frag-noop-chunk',
        navigation: {},
        nodes: [
          {
            id: 'fragment:frag-noop-chunk',
            kind: 'fragment',
            ir_kind: 'chunk_extraction',
            label: 'doc-noop:chunk:0',
            payload: { cas_id: 'frag-noop-chunk', value: { chunk_id: 'doc-noop:chunk:0' } },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-noop-chunk' } } },
          },
          {
            id: 'fragment:unresolved-node',
            kind: 'fragment',
            ir_kind: 'document',
            label: 'orphan-node',
            payload: { cas_id: 'unresolved-node', value: { filename: 'orphan.md' } },
            refs: { self: { backend: 'hot', locator: { cas_id: 'unresolved-node' } } },
          },
        ],
        edges: [],
      };
    }
    throw new Error(`Unexpected inspection ref: ${ref}`);
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[{
        run_id: 'run-inspection-graph-noop-ui',
        project_id: 'proj-inspection-graph-noop-ui',
        case_id: 's-local-retail-v01',
        stages: [],
        decisions: [],
        evaluation: {
          report: {
            compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
            entities: { coverage: 1, passed: true },
            predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
            exploration: { mean_recall: 1, passed: true },
            query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
            passed: true,
          },
          rendered: 'ok',
          details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-inspection-graph-noop-ui' } },
        },
      }]}
      activeRunId="run-inspection-graph-noop-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /chunk_extraction_set/i }));
  fireEvent.click(within(screen.getByTestId('connected-fragments-list')).getByRole('button', { name: /doc-noop:chunk:0/i }));
  const graphPanel = await screen.findByTestId('run-fragment-graph-panel');

  fireEvent.click(within(graphPanel).getByRole('button', { name: /expand one hop/i }));
  const callCountBefore = mockGetInspectionSubgraph.mock.calls.length;
  expect(within(graphPanel).queryByRole('button', { name: /orphan-node/i })).not.toBeInTheDocument();
  expect(mockGetInspectionSubgraph.mock.calls.length).toBe(callCountBefore);
  expect(screen.getByText('chunk_extraction', { selector: '.runs-fragment-drill-summary' })).toBeInTheDocument();
});

test('run fragment graph starts focused, expands neighbors, and filters visible nodes by kind', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-fragment-graph-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-run-fragment-graph-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-run-fragment-graph-ui',
        step_id: 'step-run-fragment-graph-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-run-fragment-graph-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-run-fragment-graph-ui',
    run_id: 'run-fragment-graph-ui',
    step_id: 'step-run-fragment-graph-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 18, env_type: 'dev', env_id: 'dev-run-fragment-graph-ui' },
    why: { summary: 'Chunked document', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: {
              kind: 'chunk_extraction_set',
              subgraph_ref: 'hot://run-fragment-graph-ui/chunk_extraction_set/step-run-fragment-graph-ui',
              extraction_refs: ['frag-run-graph-chunk-1'],
            },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 1 ref',
              refs: ['frag-run-graph-chunk-1'],
              content: {
                kind: 'chunk_extraction_set',
                subgraph_ref: 'hot://run-fragment-graph-ui/chunk_extraction_set/step-run-fragment-graph-ui',
                extraction_refs: ['frag-run-graph-chunk-1'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-run-graph-chunk-1',
                  cas_id: 'frag-run-graph-chunk-1',
                  mime_type: 'application/vnd.ikam.chunk-extraction+json',
                  name: 'alpha:chunk:0',
                  inspection_ref: 'inspect://fragment/frag-run-graph-chunk-1',
                  value: {
                    chunk_id: 'alpha:chunk:0',
                    document_id: 'doc-run-graph-1',
                    source_document_fragment_id: 'frag-run-graph-doc-1',
                  },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockResolvedValue({
    schema_version: 'v1',
    root_node_id: 'fragment:frag-run-graph-chunk-1',
    navigation: { focus: { node_id: 'fragment:frag-run-graph-chunk-1' } },
    nodes: [
      {
        id: 'fragment:frag-run-graph-chunk-1',
        kind: 'fragment',
        ir_kind: 'chunk_extraction',
        label: 'alpha:chunk:0',
        payload: {
          cas_id: 'frag-run-graph-chunk-1',
          value: {
            chunk_id: 'alpha:chunk:0',
            document_id: 'doc-run-graph-1',
            source_document_fragment_id: 'frag-run-graph-doc-1',
          },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-run-graph-chunk-1' } } },
      },
      {
        id: 'fragment:frag-run-graph-doc-1',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'alpha.md',
        payload: {
          cas_id: 'frag-run-graph-doc-1',
          value: {
            document_id: 'doc-run-graph-1',
            filename: 'alpha.md',
          },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-run-graph-doc-1' } } },
      },
      {
        id: 'subgraph:hot://run-fragment-graph-ui/chunk_extraction_set/step-run-fragment-graph-ui',
        kind: 'subgraph',
        ir_kind: 'chunk_extraction_set',
        label: 'chunk set',
        payload: {},
        refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-fragment-graph-ui/chunk_extraction_set/step-run-fragment-graph-ui' } } },
      },
      {
        id: 'fragment:frag-run-graph-note-1',
        kind: 'fragment',
        ir_kind: 'annotation',
        label: 'note.txt',
        payload: {
          cas_id: 'frag-run-graph-note-1',
          value: { filename: 'note.txt' },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-run-graph-note-1' } } },
      },
    ],
    edges: [
      { id: 'edge-run-graph-1', from: 'fragment:frag-run-graph-chunk-1', to: 'fragment:frag-run-graph-doc-1', relation: 'derives' },
      { id: 'edge-run-graph-2', from: 'subgraph:hot://run-fragment-graph-ui/chunk_extraction_set/step-run-fragment-graph-ui', to: 'fragment:frag-run-graph-chunk-1', relation: 'contains' },
      { id: 'edge-run-graph-3', from: 'fragment:frag-run-graph-doc-1', to: 'fragment:frag-run-graph-note-1', relation: 'annotates' },
    ],
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[{
        run_id: 'run-fragment-graph-ui',
        project_id: 'proj-run-fragment-graph-ui',
        case_id: 's-local-retail-v01',
        stages: [],
        decisions: [],
        evaluation: {
          report: {
            compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
            entities: { coverage: 1, passed: true },
            predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
            exploration: { mean_recall: 1, passed: true },
            query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
            passed: true,
          },
          rendered: 'ok',
          details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-run-fragment-graph-ui' } },
        },
      }]}
      activeRunId="run-fragment-graph-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /chunk_extraction_set/i }));
  const graphPanel = await screen.findByTestId('run-fragment-graph-panel');
  fireEvent.click(within(graphPanel).getByRole('button', { name: /alpha:chunk:0/i }));
  expect(within(graphPanel).getByText('Fragment Graph')).toBeInTheDocument();
  expect(screen.queryByTestId('inspection-graph-panel')).not.toBeInTheDocument();
  expect(within(graphPanel).getByTestId('run-fragment-graph-canvas')).toHaveAttribute('data-nodes-draggable', 'true');
  expect(within(graphPanel).getByRole('button', { name: /alpha:chunk:0/i })).toBeInTheDocument();
  expect(within(graphPanel).getByRole('button', { name: /alpha\.md/i })).toBeInTheDocument();
  expect(within(graphPanel).queryByRole('button', { name: /note\.txt/i })).not.toBeInTheDocument();

  fireEvent.click(within(graphPanel).getByRole('button', { name: /expand one hop/i }));
  await waitFor(() => expect(within(graphPanel).getByRole('button', { name: /note\.txt/i })).toBeInTheDocument());

  fireEvent.change(within(graphPanel).getByLabelText(/relation filter/i), { target: { value: 'annotates' } });
  await waitFor(() => expect(within(graphPanel).getByRole('button', { name: /alpha:chunk:0/i })).toBeInTheDocument());
  expect(within(graphPanel).getByRole('button', { name: /alpha\.md/i })).toBeInTheDocument();
  expect(within(graphPanel).getByRole('button', { name: /note\.txt/i })).toBeInTheDocument();
  fireEvent.change(within(graphPanel).getByLabelText(/relation filter/i), { target: { value: 'all' } });

  fireEvent.change(within(graphPanel).getByRole('searchbox', { name: /search graph/i }), { target: { value: 'note' } });
  await waitFor(() => expect(within(graphPanel).getByRole('button', { name: /alpha:chunk:0/i })).toBeInTheDocument());
  expect(within(graphPanel).getByRole('button', { name: /note\.txt/i })).toBeInTheDocument();

  fireEvent.click(within(screen.getByTestId('fragment-explorer')).getByRole('button', { name: /show raw/i }));
  expect(within(graphPanel).getByRole('button', { name: /note\.txt/i })).toBeInTheDocument();
  fireEvent.click(within(screen.getByTestId('fragment-explorer')).getByRole('button', { name: /hide raw/i }));
  expect(within(graphPanel).getByRole('button', { name: /note\.txt/i })).toBeInTheDocument();

  fireEvent.change(within(graphPanel).getByRole('searchbox', { name: /search graph/i }), { target: { value: '' } });
  fireEvent.change(within(graphPanel).getByLabelText(/node kind filter/i), { target: { value: 'document' } });
  await waitFor(() => expect(within(graphPanel).getByRole('button', { name: /alpha:chunk:0/i })).toBeInTheDocument());
  expect(within(graphPanel).getByRole('button', { name: /alpha\.md/i })).toBeInTheDocument();
  expect(within(graphPanel).queryByRole('button', { name: /note\.txt/i })).not.toBeInTheDocument();

  fireEvent.click(within(graphPanel).getByRole('button', { name: /collapse to focus/i }));
  await waitFor(() => expect(within(graphPanel).getByRole('button', { name: /alpha:chunk:0/i })).toBeInTheDocument());
  expect(within(graphPanel).getByRole('button', { name: /alpha\.md/i })).toBeInTheDocument();
  expect(within(graphPanel).queryByRole('button', { name: /note\.txt/i })).not.toBeInTheDocument();
});

test('graph labels prefer readable primary labels over raw hot refs when a set kind can be inferred', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-graph-labels-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-graph-labels-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-graph-labels-ui',
        step_id: 'step-graph-labels-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-graph-labels-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-graph-labels-ui',
    run_id: 'run-graph-labels-ui',
    step_id: 'step-graph-labels-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 17, env_type: 'dev', env_id: 'dev-graph-labels-ui' },
    why: { summary: 'Graph labels', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: {
              kind: 'chunk_extraction_set',
              subgraph_ref: 'hot://run-graph-labels-ui/chunk_extraction_set/step-graph-labels-ui',
              extraction_refs: ['frag-graph-labels-chunk-1'],
            },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 1 ref',
              refs: ['frag-graph-labels-chunk-1'],
              content: {
                kind: 'chunk_extraction_set',
                subgraph_ref: 'hot://run-graph-labels-ui/chunk_extraction_set/step-graph-labels-ui',
                extraction_refs: ['frag-graph-labels-chunk-1'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-graph-labels-chunk-1',
                  cas_id: 'frag-graph-labels-chunk-1',
                  mime_type: 'application/vnd.ikam.chunk-extraction+json',
                  name: 'alpha:chunk:0',
                  inspection_ref: 'inspect://fragment/frag-graph-labels-chunk-1',
                  value: {
                    chunk_id: 'alpha:chunk:0',
                    document_id: 'doc-graph-labels-1',
                    source_document_fragment_id: 'frag-graph-labels-doc-1',
                  },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockResolvedValue({
    schema_version: 'v1',
    root_node_id: 'fragment:frag-graph-labels-chunk-1',
    navigation: { focus: { node_id: 'fragment:frag-graph-labels-chunk-1' } },
    nodes: [
      {
        id: 'fragment:frag-graph-labels-chunk-1',
        kind: 'fragment',
        ir_kind: 'chunk_extraction',
        label: 'alpha:chunk:0',
        payload: {
          cas_id: 'frag-graph-labels-chunk-1',
          value: {
            chunk_id: 'alpha:chunk:0',
            document_id: 'doc-graph-labels-1',
            source_document_fragment_id: 'frag-graph-labels-doc-1',
          },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-graph-labels-chunk-1' } } },
      },
      {
        id: 'subgraph:hot://run-graph-labels-ui/document_set/step-load',
        kind: 'subgraph',
        label: 'hot://run-graph-labels-ui/document_set/step-load',
        payload: {},
        refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-graph-labels-ui/document_set/step-load' } } },
      },
    ],
    edges: [
      { id: 'edge-graph-labels-1', from: 'subgraph:hot://run-graph-labels-ui/document_set/step-load', to: 'fragment:frag-graph-labels-chunk-1', relation: 'contains' },
    ],
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-graph-labels-ui',
          project_id: 'proj-graph-labels-ui',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-graph-labels-ui' } },
          },
        },
      ]}
      activeRunId="run-graph-labels-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /chunk_extraction_set/i }));
  fireEvent.click(within(screen.getByTestId('connected-fragments-list')).getByRole('button', { name: /alpha:chunk:0/i }));

  const graphPanel = await screen.findByTestId('run-fragment-graph-panel');
  const readableSetNode = within(graphPanel).getByRole('button', { name: /document set/i });
  expect(readableSetNode).toBeInTheDocument();
  expect(readableSetNode.querySelector('strong')).toHaveTextContent('Document set');
  expect(within(graphPanel).getByText('hot://run-graph-labels-ui/document_set/step-load')).toBeInTheDocument();
});

test('fragment graph uses icon-first nodes with max-10 captions and hover titles', async () => {
  const consoleErrorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-compact-graph-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-compact-graph-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-compact-graph-ui',
        step_id: 'step-compact-graph-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-compact-graph-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-compact-graph-ui',
    run_id: 'run-compact-graph-ui',
    step_id: 'step-compact-graph-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 18, env_type: 'dev', env_id: 'dev-compact-graph-ui' },
    why: { summary: 'Compact graph labels', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:compact'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: {
              kind: 'chunk_extraction_set',
              subgraph_ref: 'hot://run-compact-graph-ui/chunk_extraction_set/step-compact-graph-ui',
              extraction_refs: ['frag-compact-chunk'],
            },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 1 ref',
              refs: ['frag-compact-chunk'],
              content: {
                kind: 'chunk_extraction_set',
                subgraph_ref: 'hot://run-compact-graph-ui/chunk_extraction_set/step-compact-graph-ui',
                extraction_refs: ['frag-compact-chunk'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-compact-chunk',
                  cas_id: 'frag-compact-chunk',
                  mime_type: 'application/vnd.ikam.chunk-extraction+json',
                  name: 'very-long-source-bookkeeping-q4-2025-summary.md#chunk-17',
                  inspection_ref: 'inspect://fragment/frag-compact-chunk',
                  value: {
                    chunk_id: 'very-long-source-bookkeeping-q4-2025-summary.md#chunk-17',
                    document_id: 'doc-compact-1',
                    source_document_fragment_id: 'frag-compact-doc',
                  },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockResolvedValue({
    schema_version: 'v1',
    root_node_id: 'fragment:frag-compact-chunk',
    navigation: { focus: { node_id: 'fragment:frag-compact-chunk' } },
    nodes: [
      {
        id: 'fragment:frag-compact-chunk',
        kind: 'fragment',
        ir_kind: 'chunk',
        label: 'very-long-source-bookkeeping-q4-2025-summary.md#chunk-17',
        payload: {
          cas_id: 'frag-compact-chunk',
          value: {
            chunk_id: 'very-long-source-bookkeeping-q4-2025-summary.md#chunk-17',
            document_id: 'doc-compact-1',
            source_document_fragment_id: 'frag-compact-doc',
          },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-compact-chunk' } } },
      },
      {
        id: 'fragment:frag-compact-doc',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'very-long-source-bookkeeping-q4-2025-summary.md',
        payload: {
          cas_id: 'frag-compact-doc',
          value: {
            document_id: 'doc-compact-1',
            filename: 'very-long-source-bookkeeping-q4-2025-summary.md',
          },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-compact-doc' } } },
      },
    ],
    edges: [
      { id: 'edge-compact-1', from: 'fragment:frag-compact-doc', to: 'fragment:frag-compact-chunk', relation: 'contains' },
    ],
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[{
        run_id: 'run-compact-graph-ui',
        project_id: 'proj-compact-graph-ui',
        case_id: 's-local-retail-v01',
        stages: [],
        decisions: [],
        evaluation: {
          report: {
            compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
            entities: { coverage: 1, passed: true },
            predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
            exploration: { mean_recall: 1, passed: true },
            query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
            passed: true,
          },
          rendered: 'ok',
          details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-compact-graph-ui' } },
        },
      }]}
      activeRunId="run-compact-graph-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /chunk_extraction_set/i }));
  fireEvent.click(within(await screen.findByTestId('connected-fragments-list')).getByRole('button', { name: /^very-long-source-bookkeeping-q4-2025-summary\.md/i }));

  const compactGraphPanel = await screen.findByTestId('run-fragment-graph-panel');
  const chunkNode = within(compactGraphPanel).getByRole('button', { name: /chunk-17/i });
  const documentNode = within(compactGraphPanel).getByRole('button', { name: /very-lo\.\.\./i });

  expect(chunkNode).toHaveAttribute('title', 'very-long-source-bookkeeping-q4-2025-summary.md#chunk-17');
  expect(documentNode).toHaveAttribute('title', 'very-long-source-bookkeeping-q4-2025-summary.md');
  expect(within(chunkNode).getByText('CHK')).toBeInTheDocument();
  expect(within(documentNode).getByText('MD')).toBeInTheDocument();
  expect(within(documentNode).queryByText('very-long-source-bookkeeping-q4-2025-summary.md')).not.toBeInTheDocument();
  expect(
    consoleErrorSpy.mock.calls.some((call) => call.some((value) => String(value).includes('Encountered two children with the same key')))
  ).toBe(false);
  consoleErrorSpy.mockRestore();
});

test('density caps large run graph expansions and prompts for a narrower expansion', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-density-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-density-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-density-ui',
        step_id: 'step-density-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-density-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-density-ui',
    run_id: 'run-density-ui',
    step_id: 'step-density-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 18, env_type: 'dev', env_id: 'dev-density-ui' },
    why: { summary: 'Dense graph', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: {
              kind: 'chunk_extraction_set',
              subgraph_ref: 'hot://run-density-ui/chunk_extraction_set/step-density-ui',
              extraction_refs: ['frag-density-focus'],
            },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 1 ref',
              refs: ['frag-density-focus'],
              content: {
                kind: 'chunk_extraction_set',
                subgraph_ref: 'hot://run-density-ui/chunk_extraction_set/step-density-ui',
                extraction_refs: ['frag-density-focus'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-density-focus',
                  cas_id: 'frag-density-focus',
                  mime_type: 'application/vnd.ikam.chunk-extraction+json',
                  name: 'density:chunk:0',
                  inspection_ref: 'inspect://fragment/frag-density-focus',
                  value: {
                    chunk_id: 'density:chunk:0',
                    document_id: 'doc-density-1',
                    source_document_fragment_id: 'frag-density-doc-0',
                  },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockResolvedValue({
    schema_version: 'v1',
    root_node_id: 'fragment:frag-density-focus',
    navigation: { focus: { node_id: 'fragment:frag-density-focus' } },
    nodes: [
      {
        id: 'fragment:frag-density-focus',
        kind: 'fragment',
        ir_kind: 'chunk_extraction',
        label: 'density:chunk:0',
        payload: {
          cas_id: 'frag-density-focus',
          value: {
            chunk_id: 'density:chunk:0',
            document_id: 'doc-density-1',
            source_document_fragment_id: 'frag-density-doc-0',
          },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-focus' } } },
      },
      {
        id: 'fragment:frag-density-doc-0',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'density-0.md',
        payload: {
          cas_id: 'frag-density-doc-0',
          value: { document_id: 'doc-density-0', filename: 'density-0.md' },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-doc-0' } } },
      },
      {
        id: 'fragment:frag-density-doc-1',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'density-1.md',
        payload: {
          cas_id: 'frag-density-doc-1',
          value: { document_id: 'doc-density-1', filename: 'density-1.md' },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-doc-1' } } },
      },
      {
        id: 'fragment:frag-density-doc-2',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'density-2.md',
        payload: {
          cas_id: 'frag-density-doc-2',
          value: { document_id: 'doc-density-2', filename: 'density-2.md' },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-doc-2' } } },
      },
      {
        id: 'fragment:frag-density-doc-3',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'density-3.md',
        payload: {
          cas_id: 'frag-density-doc-3',
          value: { document_id: 'doc-density-3', filename: 'density-3.md' },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-doc-3' } } },
      },
      {
        id: 'fragment:frag-density-doc-4',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'density-4.md',
        payload: {
          cas_id: 'frag-density-doc-4',
          value: { document_id: 'doc-density-4', filename: 'density-4.md' },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-doc-4' } } },
      },
      {
        id: 'fragment:frag-density-doc-5',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'density-5.md',
        payload: {
          cas_id: 'frag-density-doc-5',
          value: { document_id: 'doc-density-5', filename: 'density-5.md' },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-doc-5' } } },
      },
    ],
    edges: [
      { id: 'edge-density-0', from: 'fragment:frag-density-focus', to: 'fragment:frag-density-doc-0', relation: 'derives' },
      { id: 'edge-density-1', from: 'fragment:frag-density-doc-0', to: 'fragment:frag-density-doc-1', relation: 'references' },
      { id: 'edge-density-2', from: 'fragment:frag-density-doc-0', to: 'fragment:frag-density-doc-2', relation: 'references' },
      { id: 'edge-density-3', from: 'fragment:frag-density-doc-0', to: 'fragment:frag-density-doc-3', relation: 'references' },
      { id: 'edge-density-4', from: 'fragment:frag-density-doc-0', to: 'fragment:frag-density-doc-4', relation: 'references' },
      { id: 'edge-density-5', from: 'fragment:frag-density-doc-0', to: 'fragment:frag-density-doc-5', relation: 'references' },
    ],
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-density-ui',
          project_id: 'proj-density-ui',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-density-ui' } },
          },
        },
      ]}
      activeRunId="run-density-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /chunk_extraction_set/i }));
  fireEvent.click(within(screen.getByTestId('connected-fragments-list')).getByRole('button', { name: /density:chunk:0/i }));

  const graphPanel = await screen.findByTestId('run-fragment-graph-panel');
  fireEvent.click(within(graphPanel).getByRole('button', { name: /expand one hop/i }));

  expect(within(graphPanel).getByText(/too many neighbors to expand at once/i)).toBeInTheDocument();
  expect(within(graphPanel).queryByRole('button', { name: /density-5\.md/i })).not.toBeInTheDocument();
});

test('density warning clears after narrowing and allows a smaller expansion path', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-density-clear-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-density-clear-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-density-clear-ui',
        step_id: 'step-density-clear-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-density-clear-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-density-clear-ui',
    run_id: 'run-density-clear-ui',
    step_id: 'step-density-clear-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 18, env_type: 'dev', env_id: 'dev-density-clear-ui' },
    why: { summary: 'Dense graph clear', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: {
              kind: 'chunk_extraction_set',
              subgraph_ref: 'hot://run-density-clear-ui/chunk_extraction_set/step-density-clear-ui',
              extraction_refs: ['frag-density-clear-focus'],
            },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 1 ref',
              refs: ['frag-density-clear-focus'],
              content: {
                kind: 'chunk_extraction_set',
                subgraph_ref: 'hot://run-density-clear-ui/chunk_extraction_set/step-density-clear-ui',
                extraction_refs: ['frag-density-clear-focus'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-density-clear-focus',
                  cas_id: 'frag-density-clear-focus',
                  mime_type: 'application/vnd.ikam.chunk-extraction+json',
                  name: 'density-clear:chunk:0',
                  inspection_ref: 'inspect://fragment/frag-density-clear-focus',
                  value: {
                    chunk_id: 'density-clear:chunk:0',
                    document_id: 'doc-density-clear-0',
                    source_document_fragment_id: 'frag-density-clear-doc-0',
                  },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockResolvedValue({
    schema_version: 'v1',
    root_node_id: 'fragment:frag-density-clear-focus',
    navigation: { focus: { node_id: 'fragment:frag-density-clear-focus' } },
    nodes: [
      {
        id: 'fragment:frag-density-clear-focus',
        kind: 'fragment',
        ir_kind: 'chunk_extraction',
        label: 'density-clear:chunk:0',
        payload: {
          cas_id: 'frag-density-clear-focus',
          value: {
            chunk_id: 'density-clear:chunk:0',
            document_id: 'doc-density-clear-0',
            source_document_fragment_id: 'frag-density-clear-doc-0',
          },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-clear-focus' } } },
      },
      {
        id: 'fragment:frag-density-clear-doc-0',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'clear-0.md',
        payload: { cas_id: 'frag-density-clear-doc-0', value: { document_id: 'doc-density-clear-0', filename: 'clear-0.md' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-clear-doc-0' } } },
      },
      {
        id: 'fragment:frag-density-clear-doc-1',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'clear-1.md',
        payload: { cas_id: 'frag-density-clear-doc-1', value: { document_id: 'doc-density-clear-1', filename: 'clear-1.md' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-clear-doc-1' } } },
      },
      {
        id: 'fragment:frag-density-clear-doc-2',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'clear-2.md',
        payload: { cas_id: 'frag-density-clear-doc-2', value: { document_id: 'doc-density-clear-2', filename: 'clear-2.md' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-clear-doc-2' } } },
      },
      {
        id: 'fragment:frag-density-clear-doc-3',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'clear-3.md',
        payload: { cas_id: 'frag-density-clear-doc-3', value: { document_id: 'doc-density-clear-3', filename: 'clear-3.md' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-clear-doc-3' } } },
      },
      {
        id: 'fragment:frag-density-clear-doc-4',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'clear-4.md',
        payload: { cas_id: 'frag-density-clear-doc-4', value: { document_id: 'doc-density-clear-4', filename: 'clear-4.md' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-clear-doc-4' } } },
      },
      {
        id: 'fragment:frag-density-clear-doc-5',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'clear-5.md',
        payload: { cas_id: 'frag-density-clear-doc-5', value: { document_id: 'doc-density-clear-5', filename: 'clear-5.md' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-clear-doc-5' } } },
      },
      {
        id: 'fragment:frag-density-clear-note-root',
        kind: 'fragment',
        ir_kind: 'annotation',
        label: 'clear-note-root.txt',
        payload: { cas_id: 'frag-density-clear-note-root', value: { filename: 'clear-note-root.txt' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-clear-note-root' } } },
      },
      {
        id: 'fragment:frag-density-clear-note-1',
        kind: 'fragment',
        ir_kind: 'annotation',
        label: 'clear-note-1.txt',
        payload: { cas_id: 'frag-density-clear-note-1', value: { filename: 'clear-note-1.txt' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-clear-note-1' } } },
      },
      {
        id: 'fragment:frag-density-clear-note-2',
        kind: 'fragment',
        ir_kind: 'annotation',
        label: 'clear-note-2.txt',
        payload: { cas_id: 'frag-density-clear-note-2', value: { filename: 'clear-note-2.txt' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-clear-note-2' } } },
      },
      {
        id: 'fragment:frag-density-clear-note-3',
        kind: 'fragment',
        ir_kind: 'annotation',
        label: 'clear-note-3.txt',
        payload: { cas_id: 'frag-density-clear-note-3', value: { filename: 'clear-note-3.txt' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-clear-note-3' } } },
      },
      {
        id: 'fragment:frag-density-clear-note-4',
        kind: 'fragment',
        ir_kind: 'annotation',
        label: 'clear-note-4.txt',
        payload: { cas_id: 'frag-density-clear-note-4', value: { filename: 'clear-note-4.txt' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-clear-note-4' } } },
      },
    ],
    edges: [
      { id: 'edge-density-clear-0', from: 'fragment:frag-density-clear-focus', to: 'fragment:frag-density-clear-doc-0', relation: 'derives' },
      { id: 'edge-density-clear-root-note', from: 'fragment:frag-density-clear-focus', to: 'fragment:frag-density-clear-note-root', relation: 'annotates' },
      { id: 'edge-density-clear-1', from: 'fragment:frag-density-clear-doc-0', to: 'fragment:frag-density-clear-doc-1', relation: 'references' },
      { id: 'edge-density-clear-2', from: 'fragment:frag-density-clear-doc-0', to: 'fragment:frag-density-clear-doc-2', relation: 'references' },
      { id: 'edge-density-clear-3', from: 'fragment:frag-density-clear-doc-0', to: 'fragment:frag-density-clear-doc-3', relation: 'references' },
      { id: 'edge-density-clear-4', from: 'fragment:frag-density-clear-doc-0', to: 'fragment:frag-density-clear-doc-4', relation: 'references' },
      { id: 'edge-density-clear-5', from: 'fragment:frag-density-clear-doc-0', to: 'fragment:frag-density-clear-doc-5', relation: 'references' },
      { id: 'edge-density-clear-6', from: 'fragment:frag-density-clear-note-root', to: 'fragment:frag-density-clear-note-1', relation: 'annotates' },
      { id: 'edge-density-clear-7', from: 'fragment:frag-density-clear-note-root', to: 'fragment:frag-density-clear-note-2', relation: 'annotates' },
      { id: 'edge-density-clear-8', from: 'fragment:frag-density-clear-note-root', to: 'fragment:frag-density-clear-note-3', relation: 'annotates' },
      { id: 'edge-density-clear-9', from: 'fragment:frag-density-clear-note-root', to: 'fragment:frag-density-clear-note-4', relation: 'annotates' },
    ],
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-density-clear-ui',
          project_id: 'proj-density-clear-ui',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-density-clear-ui' } },
          },
        },
      ]}
      activeRunId="run-density-clear-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /chunk_extraction_set/i }));
  fireEvent.click(within(screen.getByTestId('connected-fragments-list')).getByRole('button', { name: /density-clear:chunk:0/i }));

  const graphPanel = await screen.findByTestId('run-fragment-graph-panel');
  fireEvent.click(within(graphPanel).getByRole('button', { name: /expand one hop/i }));
  expect(within(graphPanel).getByText(/too many neighbors to expand at once/i)).toBeInTheDocument();

  fireEvent.change(within(graphPanel).getByLabelText(/node kind filter/i), { target: { value: 'annotation' } });
  expect(within(graphPanel).queryByText(/too many neighbors to expand at once/i)).not.toBeInTheDocument();

  fireEvent.click(within(graphPanel).getByRole('button', { name: /expand one hop/i }));
  expect(within(graphPanel).getByRole('button', { name: /clear-note-root\.txt/i })).toBeInTheDocument();
  expect(within(graphPanel).queryByRole('button', { name: /clear-5\.md/i })).not.toBeInTheDocument();
});

test('density warning also clears when search narrowing leaves only inferred set-kind expansion candidates', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-density-search-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-density-search-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-density-search-ui',
        step_id: 'step-density-search-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-density-search-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-density-search-ui',
    run_id: 'run-density-search-ui',
    step_id: 'step-density-search-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 16, env_type: 'dev', env_id: 'dev-density-search-ui' },
    why: { summary: 'Dense graph search', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: {
              kind: 'chunk_extraction_set',
              subgraph_ref: 'hot://run-density-search-ui/chunk_extraction_set/step-density-search-ui',
              extraction_refs: ['frag-density-search-focus'],
            },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 1 ref',
              refs: ['frag-density-search-focus'],
              content: {
                kind: 'chunk_extraction_set',
                subgraph_ref: 'hot://run-density-search-ui/chunk_extraction_set/step-density-search-ui',
                extraction_refs: ['frag-density-search-focus'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-density-search-focus',
                  cas_id: 'frag-density-search-focus',
                  mime_type: 'application/vnd.ikam.chunk-extraction+json',
                  name: 'density-search:chunk:0',
                  inspection_ref: 'inspect://fragment/frag-density-search-focus',
                  value: {
                    chunk_id: 'density-search:chunk:0',
                    document_id: 'doc-density-search',
                    source_document_fragment_id: 'frag-density-search-doc',
                  },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockResolvedValue({
    schema_version: 'v1',
    root_node_id: 'fragment:frag-density-search-focus',
    navigation: { focus: { node_id: 'fragment:frag-density-search-focus' } },
    nodes: [
      {
        id: 'fragment:frag-density-search-focus',
        kind: 'fragment',
        ir_kind: 'chunk_extraction',
        label: 'density-search:chunk:0',
        payload: {
          cas_id: 'frag-density-search-focus',
          value: {
            chunk_id: 'density-search:chunk:0',
            document_id: 'doc-density-search',
            source_document_fragment_id: 'frag-density-search-doc',
          },
        },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-search-focus' } } },
      },
      {
        id: 'subgraph:hot://run-density-search-ui/document_set/step-load',
        kind: 'subgraph',
        label: 'hot://run-density-search-ui/document_set/step-load',
        payload: {},
        refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-density-search-ui/document_set/step-load' } } },
      },
      {
        id: 'subgraph:hot://run-density-search-ui/document_set/step-scan',
        kind: 'subgraph',
        label: 'hot://run-density-search-ui/document_set/step-scan',
        payload: {},
        refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-density-search-ui/document_set/step-scan' } } },
      },
      {
        id: 'subgraph:hot://run-density-search-ui/document_set/step-merge',
        kind: 'subgraph',
        label: 'hot://run-density-search-ui/document_set/step-merge',
        payload: {},
        refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-density-search-ui/document_set/step-merge' } } },
      },
      {
        id: 'subgraph:hot://run-density-search-ui/document_set/step-publish',
        kind: 'subgraph',
        label: 'hot://run-density-search-ui/document_set/step-publish',
        payload: {},
        refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-density-search-ui/document_set/step-publish' } } },
      },
      {
        id: 'fragment:frag-density-search-doc',
        kind: 'fragment',
        ir_kind: 'document',
        label: 'density-search.md',
        payload: { cas_id: 'frag-density-search-doc', value: { document_id: 'doc-density-search', filename: 'density-search.md' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-search-doc' } } },
      },
      {
        id: 'fragment:frag-density-search-note-1',
        kind: 'fragment',
        ir_kind: 'annotation',
        label: 'search-note-1.txt',
        payload: { cas_id: 'frag-density-search-note-1', value: { filename: 'search-note-1.txt' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-search-note-1' } } },
      },
      {
        id: 'fragment:frag-density-search-note-2',
        kind: 'fragment',
        ir_kind: 'annotation',
        label: 'search-note-2.txt',
        payload: { cas_id: 'frag-density-search-note-2', value: { filename: 'search-note-2.txt' } },
        refs: { self: { backend: 'hot', locator: { cas_id: 'frag-density-search-note-2' } } },
      },
    ],
    edges: [
      { id: 'edge-density-search-0', from: 'fragment:frag-density-search-focus', to: 'fragment:frag-density-search-doc', relation: 'derives' },
      { id: 'edge-density-search-1', from: 'fragment:frag-density-search-doc', to: 'subgraph:hot://run-density-search-ui/document_set/step-load', relation: 'contains' },
      { id: 'edge-density-search-2', from: 'fragment:frag-density-search-doc', to: 'subgraph:hot://run-density-search-ui/document_set/step-scan', relation: 'contains' },
      { id: 'edge-density-search-3', from: 'fragment:frag-density-search-doc', to: 'subgraph:hot://run-density-search-ui/document_set/step-merge', relation: 'contains' },
      { id: 'edge-density-search-4', from: 'fragment:frag-density-search-doc', to: 'subgraph:hot://run-density-search-ui/document_set/step-publish', relation: 'contains' },
      { id: 'edge-density-search-5', from: 'fragment:frag-density-search-doc', to: 'fragment:frag-density-search-note-1', relation: 'annotates' },
      { id: 'edge-density-search-6', from: 'fragment:frag-density-search-doc', to: 'fragment:frag-density-search-note-2', relation: 'annotates' },
    ],
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-density-search-ui',
          project_id: 'proj-density-search-ui',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-density-search-ui' } },
          },
        },
      ]}
      activeRunId="run-density-search-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /chunk_extraction_set/i }));
  fireEvent.click(within(screen.getByTestId('connected-fragments-list')).getByRole('button', { name: /density-search:chunk:0/i }));

  const graphPanel = await screen.findByTestId('run-fragment-graph-panel');
  fireEvent.click(within(graphPanel).getByRole('button', { name: /expand one hop/i }));
  expect(within(graphPanel).getByText(/too many neighbors to expand at once/i)).toBeInTheDocument();

  fireEvent.change(within(graphPanel).getByRole('searchbox', { name: /search graph/i }), { target: { value: 'publish' } });
  expect(within(graphPanel).queryByText(/too many neighbors to expand at once/i)).not.toBeInTheDocument();

  fireEvent.click(within(graphPanel).getByRole('button', { name: /expand one hop/i }));
  expect(within(graphPanel).queryByText(/too many neighbors to expand at once/i)).not.toBeInTheDocument();
  expect(within(graphPanel).queryByRole('button', { name: /search-note-1\.txt/i })).not.toBeInTheDocument();
});

test('document_chunk_set graph nodes are visually distinguishable from document_set and chunk_extraction_set', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-document_chunk_set-graph-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-document_chunk_set-graph-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-document_chunk_set-graph-ui',
        step_id: 'step-document_chunk_set-graph-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-document_chunk_set-graph-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-document_chunk_set-graph-ui',
    run_id: 'run-document_chunk_set-graph-ui',
    step_id: 'step-document_chunk_set-graph-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 22, env_type: 'dev', env_id: 'dev-document_chunk_set-graph-ui' },
    why: { summary: 'Document chunk styling', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: {
              kind: 'chunk_extraction_set',
              source_subgraph_ref: 'hot://run-document_chunk_set-graph-ui/document_set/step-load',
              subgraph_ref: 'hot://run-document_chunk_set-graph-ui/chunk_extraction_set/step-document_chunk_set-graph-ui',
              extraction_refs: ['frag-document_chunk_set-graph-chunk-1'],
            },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 1 ref',
              refs: ['frag-document_chunk_set-graph-chunk-1'],
              content: {
                kind: 'chunk_extraction_set',
                source_subgraph_ref: 'hot://run-document_chunk_set-graph-ui/document_set/step-load',
                subgraph_ref: 'hot://run-document_chunk_set-graph-ui/chunk_extraction_set/step-document_chunk_set-graph-ui',
                extraction_refs: ['frag-document_chunk_set-graph-chunk-1'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-document_chunk_set-graph-chunk-1',
                  cas_id: 'frag-document_chunk_set-graph-chunk-1',
                  mime_type: 'application/vnd.ikam.chunk-extraction+json',
                  name: 'doc-style:chunk:0',
                  inspection_ref: 'inspect://fragment/frag-document_chunk_set-graph-chunk-1',
                  value: {
                    chunk_id: 'doc-style:chunk:0',
                    document_id: 'doc-style',
                    source_document_fragment_id: 'frag-document_chunk_set-graph-doc-1',
                  },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockImplementation(async ({ ref }: { ref: string }) => {
    if (ref === 'inspect://fragment/frag-document_chunk_set-graph-chunk-1') {
      return {
        schema_version: 'v1',
        root_node_id: 'fragment:frag-document_chunk_set-graph-chunk-1',
        navigation: { focus: { node_id: 'fragment:frag-document_chunk_set-graph-chunk-1' } },
        nodes: [
          {
            id: 'fragment:frag-document_chunk_set-graph-chunk-1',
            kind: 'fragment',
            ir_kind: 'chunk_extraction',
            label: 'doc-style:chunk:0',
            payload: {
              cas_id: 'frag-document_chunk_set-graph-chunk-1',
              value: {
                chunk_id: 'doc-style:chunk:0',
                document_id: 'doc-style',
                source_document_fragment_id: 'frag-document_chunk_set-graph-doc-1',
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-document_chunk_set-graph-chunk-1' } } },
          },
          {
            id: 'subgraph:hot://run-document_chunk_set-graph-ui/chunk_extraction_set/step-document_chunk_set-graph-ui',
            kind: 'subgraph',
            ir_kind: 'chunk_extraction_set',
            label: 'hot://run-document_chunk_set-graph-ui/chunk_extraction_set/step-document_chunk_set-graph-ui',
            payload: {},
            refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-document_chunk_set-graph-ui/chunk_extraction_set/step-document_chunk_set-graph-ui' } } },
          },
          {
            id: 'subgraph:hot://run-document_chunk_set-graph-ui/document_set/step-load',
            kind: 'subgraph',
            ir_kind: 'document_set',
            label: 'hot://run-document_chunk_set-graph-ui/document_set/step-load',
            payload: {},
            refs: { self: { backend: 'hot', locator: { subgraph_ref: 'hot://run-document_chunk_set-graph-ui/document_set/step-load' } } },
          },
          {
            id: 'fragment:frag-document_chunk_set-graph-document-chunks',
            kind: 'fragment',
            ir_kind: 'json',
            label: 'document_chunk_set',
            payload: {
              cas_id: 'frag-document_chunk_set-graph-document-chunks',
              mime_type: 'application/vnd.ikam.document-chunk-set+json',
              value: {
                kind: 'document_chunk_set',
                document_id: 'doc-style',
                source_document_fragment_id: 'frag-document_chunk_set-graph-doc-1',
                chunk_refs: ['frag-document_chunk_set-graph-chunk-1'],
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-document_chunk_set-graph-document-chunks' } } },
          },
        ],
        edges: [
          { id: 'edge-document_chunk_set-1', from: 'subgraph:hot://run-document_chunk_set-graph-ui/chunk_extraction_set/step-document_chunk_set-graph-ui', to: 'fragment:frag-document_chunk_set-graph-chunk-1', relation: 'contains' },
          { id: 'edge-document_chunk_set-2', from: 'subgraph:hot://run-document_chunk_set-graph-ui/document_set/step-load', to: 'fragment:frag-document_chunk_set-graph-document-chunks', relation: 'contains' },
          { id: 'edge-document_chunk_set-3', from: 'fragment:frag-document_chunk_set-graph-document-chunks', to: 'fragment:frag-document_chunk_set-graph-chunk-1', relation: 'references' },
        ],
      };
    }
    throw new Error(`Unexpected inspection ref: ${ref}`);
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[
        {
          run_id: 'run-document_chunk_set-graph-ui',
          project_id: 'proj-document_chunk_set-graph-ui',
          case_id: 's-local-retail-v01',
          stages: [],
          decisions: [],
          evaluation: {
            report: {
              compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
              entities: { coverage: 1, passed: true },
              predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
              exploration: { mean_recall: 1, passed: true },
              query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
              passed: true,
            },
            rendered: 'ok',
            details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-document_chunk_set-graph-ui' } },
          },
        },
      ]}
      activeRunId="run-document_chunk_set-graph-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /chunk_extraction_set/i }));
  fireEvent.click(within(screen.getByTestId('connected-fragments-list')).getByRole('button', { name: /doc-style:chunk:0/i }));

  const graphPanel = await screen.findByTestId('run-fragment-graph-panel');
  const chunkSetNode = within(graphPanel).getByRole('button', { name: /chunk extraction set/i });
  const documentChunkSetNode = within(graphPanel).getByRole('button', { name: /document chunk set/i });

  expect(chunkSetNode).toHaveClass('inspection-graph-node-button-kind-chunk-extraction-set');
  expect(documentChunkSetNode).toHaveClass('inspection-graph-node-button-kind-document-chunk-set');
  expect(chunkSetNode).toHaveStyle({ backgroundColor: 'rgba(201, 143, 59, 0.14)', borderColor: 'rgba(201, 143, 59, 0.34)' });
  expect(documentChunkSetNode).toHaveStyle({ backgroundColor: 'rgba(47, 143, 111, 0.12)', borderColor: 'rgba(47, 143, 111, 0.34)' });
});

test('run fragment graph sync resolves duplicate graph labels by stable identifiers', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-graph-sync-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-graph-sync-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-graph-sync-ui',
        step_id: 'step-graph-sync-ui',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-graph-sync-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockResolvedValue({
    schema_version: 'v1',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-graph-sync-ui',
    run_id: 'run-graph-sync-ui',
    step_id: 'step-graph-sync-ui',
    step_name: 'parse.chunk',
    attempt_index: 1,
    outcome: { status: 'succeeded', duration_ms: 19, env_type: 'dev', env_id: 'dev-graph-sync-ui' },
    why: { summary: 'Chunked document', policy: { name: 'parse.chunk', params: {} } },
    inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
    outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
    checks: [],
    transition_validation: {
      specs: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
      ],
      resolved_inputs: {},
      resolved_outputs: {
        chunk_extraction_set: [
          {
            value: {
              kind: 'chunk_extraction_set',
              subgraph_ref: 'hot://run-graph-sync-ui/chunk_extraction_set/step-graph-sync-ui',
              extraction_refs: ['frag-graph-sync-chunk-1'],
            },
            inspection: {
              value_kind: 'chunk_extraction_set',
              summary: 'chunk_extraction_set 1 ref',
              refs: ['frag-graph-sync-chunk-1'],
              content: {
                kind: 'chunk_extraction_set',
                subgraph_ref: 'hot://run-graph-sync-ui/chunk_extraction_set/step-graph-sync-ui',
                extraction_refs: ['frag-graph-sync-chunk-1'],
              },
              resolved_refs: [
                {
                  fragment_id: 'frag-graph-sync-chunk-1',
                  cas_id: 'frag-graph-sync-chunk-1',
                  mime_type: 'application/vnd.ikam.chunk-extraction+json',
                  name: 'alpha:chunk:0',
                  inspection_ref: 'inspect://fragment/frag-graph-sync-chunk-1',
                  value: {
                    chunk_id: 'alpha:chunk:0',
                    document_id: 'doc-graph-sync-1',
                    source_document_fragment_id: 'frag-graph-sync-doc-1',
                  },
                },
              ],
            },
          },
        ],
      },
      results: [
        { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
      ],
    },
    lineage: { roots: [], nodes: [], edges: [] },
  });
  mockGetInspectionSubgraph.mockImplementation(async ({ ref }: { ref: string }) => {
    if (ref === 'inspect://fragment/frag-graph-sync-chunk-1') {
      return {
        schema_version: 'v1',
        root_node_id: 'fragment:frag-graph-sync-chunk-1',
        navigation: { focus: { node_id: 'fragment:frag-graph-sync-chunk-1' } },
        nodes: [
          {
            id: 'fragment:frag-graph-sync-chunk-1',
            kind: 'fragment',
            ir_kind: 'chunk_extraction',
            label: 'alpha:chunk:0',
            payload: {
              cas_id: 'frag-graph-sync-chunk-1',
              value: {
                chunk_id: 'alpha:chunk:0',
                document_id: 'doc-graph-sync-1',
                source_document_fragment_id: 'frag-graph-sync-doc-1',
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-graph-sync-chunk-1' } } },
          },
          {
            id: 'fragment:frag-graph-sync-doc-1',
            kind: 'fragment',
            ir_kind: 'document',
            label: 'duplicate.md',
            payload: {
              cas_id: 'frag-graph-sync-doc-1',
              value: {
                document_id: 'doc-graph-sync-1',
                filename: 'duplicate.md',
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-graph-sync-doc-1' } } },
          },
          {
            id: 'fragment:frag-graph-sync-doc-2',
            kind: 'fragment',
            ir_kind: 'document',
            label: 'duplicate.md',
            payload: {
              cas_id: 'frag-graph-sync-doc-2',
              value: {
                document_id: 'doc-graph-sync-2',
                filename: 'duplicate.md',
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-graph-sync-doc-2' } } },
          },
        ],
        edges: [
          { id: 'edge-graph-sync-1', from: 'fragment:frag-graph-sync-chunk-1', to: 'fragment:frag-graph-sync-doc-1', relation: 'derives' },
          { id: 'edge-graph-sync-1b', from: 'fragment:frag-graph-sync-chunk-1', to: 'fragment:frag-graph-sync-doc-2', relation: 'references' },
        ],
      };
    }
    if (ref === 'inspect://fragment/frag-graph-sync-doc-1') {
      return {
        schema_version: 'v1',
        root_node_id: 'fragment:frag-graph-sync-doc-1',
        navigation: { focus: { node_id: 'fragment:frag-graph-sync-doc-1' } },
        nodes: [
          {
            id: 'fragment:frag-graph-sync-doc-1',
            kind: 'fragment',
            ir_kind: 'document',
            label: 'duplicate.md',
            payload: {
              cas_id: 'frag-graph-sync-doc-1',
              value: {
                document_id: 'doc-graph-sync-1',
                filename: 'duplicate.md',
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-graph-sync-doc-1' } } },
          },
          {
            id: 'fragment:frag-graph-sync-chunk-1',
            kind: 'fragment',
            ir_kind: 'chunk_extraction',
            label: 'alpha:chunk:0',
            payload: {
              cas_id: 'frag-graph-sync-chunk-1',
              value: {
                chunk_id: 'alpha:chunk:0',
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-graph-sync-chunk-1' } } },
          },
        ],
        edges: [
          { id: 'edge-graph-sync-2', from: 'fragment:frag-graph-sync-doc-1', to: 'fragment:frag-graph-sync-chunk-1', relation: 'contains' },
        ],
      };
    }
    if (ref === 'inspect://fragment/frag-graph-sync-doc-2') {
      return {
        schema_version: 'v1',
        root_node_id: 'fragment:frag-graph-sync-doc-2',
        navigation: { focus: { node_id: 'fragment:frag-graph-sync-doc-2' } },
        nodes: [
          {
            id: 'fragment:frag-graph-sync-doc-2',
            kind: 'fragment',
            ir_kind: 'document',
            label: 'duplicate.md',
            payload: {
              cas_id: 'frag-graph-sync-doc-2',
              value: {
                document_id: 'doc-graph-sync-2',
                filename: 'duplicate.md',
              },
            },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-graph-sync-doc-2' } } },
          },
        ],
        edges: [],
      };
    }
    throw new Error(`Unexpected inspection ref: ${ref}`);
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[{
        run_id: 'run-graph-sync-ui',
        project_id: 'proj-graph-sync-ui',
        case_id: 's-local-retail-v01',
        stages: [],
        decisions: [],
        evaluation: {
          report: {
            compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
            entities: { coverage: 1, passed: true },
            predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
            exploration: { mean_recall: 1, passed: true },
            query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
            passed: true,
          },
          rendered: 'ok',
          details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-graph-sync-ui' } },
        },
      }]}
      activeRunId="run-graph-sync-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalled());
  fireEvent.click(within(screen.getByTestId('fragment-section-output')).getByRole('button', { name: /chunk_extraction_set/i }));
  fireEvent.click(within(screen.getByTestId('connected-fragments-list')).getByRole('button', { name: /alpha:chunk:0/i }));

  await waitFor(() => expect(mockGetInspectionSubgraph).toHaveBeenCalledWith({
    runId: 'run-graph-sync-ui',
    ref: 'inspect://fragment/frag-graph-sync-chunk-1',
    maxDepth: 1,
  }));

  const graphPanel = await screen.findByTestId('run-fragment-graph-panel');
  const duplicateNodes = within(graphPanel).getAllByTitle('duplicate.md');
  expect(duplicateNodes).toHaveLength(2);
  fireEvent.click(duplicateNodes[1]);

  await waitFor(() => expect(mockGetInspectionSubgraph).toHaveBeenCalledWith({
    runId: 'run-graph-sync-ui',
    ref: 'inspect://fragment/frag-graph-sync-doc-2',
    maxDepth: 1,
  }));
  await waitFor(() => expect(within(screen.getByTestId('fragment-explorer')).getByText('duplicate.md', { selector: '.runs-fragment-drill-breadcrumb' })).toBeInTheDocument());

  fireEvent.click(within(screen.getByTestId('connected-fragments-list')).getByRole('button', { name: /alpha:chunk:0/i }));
  await waitFor(() => expect(within(screen.getByTestId('fragment-explorer')).getByText('chunk_extraction 2 refs', { selector: '.runs-fragment-drill-summary' })).toBeInTheDocument());

  const focusedGraphPanel = await screen.findByTestId('run-fragment-graph-panel');
  expect(within(focusedGraphPanel).getByRole('button', { name: /alpha:chunk:0/i })).toBeInTheDocument();
  expect(within(focusedGraphPanel).getAllByTitle('duplicate.md')).toHaveLength(1);
});

test('graph focus ignores stale inspection responses after step switch', async () => {
  let resolveOldInspection: ((value: Record<string, unknown>) => void) | null = null;
  let resolveNewInspection: ((value: Record<string, unknown>) => void) | null = null;

  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-stale-graph-sync-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-stale-graph-sync-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-stale-graph-sync-ui-1',
        step_id: 'step-stale-graph-sync-ui-1',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-stale-graph-sync-ui',
      },
      {
        event_id: 'ev-stale-graph-sync-ui-2',
        step_id: 'step-stale-graph-sync-ui-2',
        step_name: 'parse.embed',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-stale-graph-sync-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockImplementation(async ({ stepId }: { stepId: string }) => {
    if (stepId === 'step-stale-graph-sync-ui-1') {
      return {
        schema_version: 'v1',
        pipeline_id: 'ingestion-early-parse',
        pipeline_run_id: 'pipe-stale-graph-sync-ui',
        run_id: 'run-stale-graph-sync-ui',
        step_id: 'step-stale-graph-sync-ui-1',
        step_name: 'parse.chunk',
        attempt_index: 1,
        outcome: { status: 'succeeded', duration_ms: 7, env_type: 'dev', env_id: 'dev-stale-graph-sync-ui' },
        why: { summary: 'Old step', policy: { name: 'parse.chunk', params: {} } },
        inputs: { artifact_ids: [], fragment_ids: [], program_ids: [] },
        outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
        checks: [],
        transition_validation: {
          specs: [
            { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
          ],
          resolved_inputs: {},
          resolved_outputs: {
            chunk_extraction_set: [
              {
                value: { kind: 'chunk_extraction_set' },
                inspection_stub: { inspection_ref: 'inspect://fragment/stale-old-focus' },
              },
            ],
          },
          results: [
            { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
          ],
        },
        lineage: { roots: [], nodes: [], edges: [] },
      };
    }
    return {
      schema_version: 'v1',
      pipeline_id: 'ingestion-early-parse',
      pipeline_run_id: 'pipe-stale-graph-sync-ui',
      run_id: 'run-stale-graph-sync-ui',
      step_id: 'step-stale-graph-sync-ui-2',
      step_name: 'parse.embed',
      attempt_index: 1,
      outcome: { status: 'succeeded', duration_ms: 9, env_type: 'dev', env_id: 'dev-stale-graph-sync-ui' },
      why: { summary: 'New step', policy: { name: 'parse.embed', params: {} } },
      inputs: { artifact_ids: [], fragment_ids: [], program_ids: [] },
      outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
      checks: [],
      transition_validation: {
        specs: [
          { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
        ],
        resolved_inputs: {},
        resolved_outputs: {
          chunk_extraction_set: [
            {
              value: { kind: 'chunk_extraction_set' },
              inspection_stub: { inspection_ref: 'inspect://fragment/stale-new-focus' },
            },
          ],
        },
        results: [
          { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
        ],
      },
      lineage: { roots: [], nodes: [], edges: [] },
    };
  });
  mockGetInspectionSubgraph.mockImplementation(({ ref }: { ref: string }) => {
    if (ref === 'inspect://fragment/stale-old-focus') {
      return new Promise((resolve) => {
        resolveOldInspection = resolve as (value: Record<string, unknown>) => void;
      });
    }
    if (ref === 'inspect://fragment/stale-new-focus') {
      return new Promise((resolve) => {
        resolveNewInspection = resolve as (value: Record<string, unknown>) => void;
      });
    }
    throw new Error(`Unexpected inspection ref: ${ref}`);
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[{
        run_id: 'run-stale-graph-sync-ui',
        project_id: 'proj-stale-graph-sync-ui',
        case_id: 's-local-retail-v01',
        stages: [],
        decisions: [],
        evaluation: {
          report: {
            compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
            entities: { coverage: 1, passed: true },
            predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
            exploration: { mean_recall: 1, passed: true },
            query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
            passed: true,
          },
          rendered: 'ok',
          details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-stale-graph-sync-ui' } },
        },
      }]}
      activeRunId="run-stale-graph-sync-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetInspectionSubgraph).toHaveBeenCalledWith({
    runId: 'run-stale-graph-sync-ui',
    ref: 'inspect://fragment/stale-old-focus',
    maxDepth: 1,
  }));

  fireEvent.click(screen.getByRole('button', { name: /parse\.embed.*succeeded/i }));

  await waitFor(() => expect(mockGetInspectionSubgraph).toHaveBeenCalledWith({
    runId: 'run-stale-graph-sync-ui',
    ref: 'inspect://fragment/stale-new-focus',
    maxDepth: 1,
  }));

  resolveNewInspection?.({
    value_kind: 'chunk_extraction',
    summary: 'fresh focus',
    refs: [],
    content: { chunk_id: 'new:chunk:0' },
    resolved_refs: [],
  });

  await waitFor(() => expect(within(screen.getByTestId('fragment-explorer')).getByText('fresh focus', { selector: '.runs-fragment-drill-summary' })).toBeInTheDocument());

  resolveOldInspection?.({
    value_kind: 'chunk_extraction',
    summary: 'stale focus',
    refs: [],
    content: { chunk_id: 'old:chunk:0' },
    resolved_refs: [],
  });

  await waitFor(() => expect(within(screen.getByTestId('fragment-explorer')).getByText('fresh focus', { selector: '.runs-fragment-drill-summary' })).toBeInTheDocument());
  expect(within(screen.getByTestId('fragment-explorer')).queryByText('stale focus', { selector: '.runs-fragment-drill-summary' })).not.toBeInTheDocument();
});

test('graph focus ignores stale artifact preview responses after step switch', async () => {
  let resolveOldPreview: ((value: Record<string, unknown>) => void) | null = null;
  let resolveNewPreview: ((value: Record<string, unknown>) => void) | null = null;

  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-stale-preview-sync-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-stale-preview-sync-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-stale-preview-sync-ui-1',
        step_id: 'step-stale-preview-sync-ui-1',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-stale-preview-sync-ui',
      },
      {
        event_id: 'ev-stale-preview-sync-ui-2',
        step_id: 'step-stale-preview-sync-ui-2',
        step_name: 'parse.embed',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-stale-preview-sync-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockImplementation(async ({ stepId }: { stepId: string }) => {
    const isOld = stepId === 'step-stale-preview-sync-ui-1';
    const artifactId = isOld ? 'artifact:old-preview' : 'artifact:new-preview';
    const summary = isOld ? 'old preview focus' : 'new preview focus';
    return {
      schema_version: 'v1',
      pipeline_id: 'ingestion-early-parse',
      pipeline_run_id: 'pipe-stale-preview-sync-ui',
      run_id: 'run-stale-preview-sync-ui',
      step_id: stepId,
      step_name: isOld ? 'parse.chunk' : 'parse.embed',
      attempt_index: 1,
      outcome: { status: 'succeeded', duration_ms: 9, env_type: 'dev', env_id: 'dev-stale-preview-sync-ui' },
      why: { summary, policy: { name: isOld ? 'parse.chunk' : 'parse.embed', params: {} } },
      inputs: { artifact_ids: [], fragment_ids: [], program_ids: [] },
      outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
      checks: [],
      transition_validation: {
        specs: [
          { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
        ],
        resolved_inputs: {},
        resolved_outputs: {
          chunk_extraction_set: [
            {
              value: { kind: 'chunk_extraction_set', artifact_id: artifactId },
              inspection_stub: { inspection_ref: isOld ? 'inspect://fragment/stale-preview-old' : 'inspect://fragment/stale-preview-new' },
            },
          ],
        },
        results: [
          { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
        ],
      },
      lineage: { roots: [], nodes: [], edges: [] },
    };
  });
  mockGetInspectionSubgraph.mockImplementation(async ({ ref }: { ref: string }) => ({
    value_kind: 'chunk_extraction',
    summary: ref === 'inspect://fragment/stale-preview-old' ? 'old preview focus' : 'new preview focus',
    refs: [],
    content: { artifact_id: ref === 'inspect://fragment/stale-preview-old' ? 'artifact:old-preview' : 'artifact:new-preview' },
    resolved_refs: [],
  }));
  mockGetArtifactPreview.mockImplementation(({ artifactId }: { artifactId: string }) => {
    if (artifactId === 'artifact:old-preview') {
      return new Promise((resolve) => {
        resolveOldPreview = resolve as (value: Record<string, unknown>) => void;
      });
    }
    if (artifactId === 'artifact:new-preview') {
      return new Promise((resolve) => {
        resolveNewPreview = resolve as (value: Record<string, unknown>) => void;
      });
    }
    throw new Error(`Unexpected artifact preview request: ${artifactId}`);
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[{
        run_id: 'run-stale-preview-sync-ui',
        project_id: 'proj-stale-preview-sync-ui',
        case_id: 's-local-retail-v01',
        stages: [],
        decisions: [],
        evaluation: {
          report: {
            compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
            entities: { coverage: 1, passed: true },
            predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
            exploration: { mean_recall: 1, passed: true },
            query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
            passed: true,
          },
          rendered: 'ok',
          details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-stale-preview-sync-ui' } },
        },
      }]}
      activeRunId="run-stale-preview-sync-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetArtifactPreview).toHaveBeenCalledWith({ runId: 'run-stale-preview-sync-ui', artifactId: 'artifact:old-preview' }));

  fireEvent.click(screen.getByRole('button', { name: /parse\.embed.*succeeded/i }));

  await waitFor(() => expect(mockGetArtifactPreview).toHaveBeenCalledWith({ runId: 'run-stale-preview-sync-ui', artifactId: 'artifact:new-preview' }));

  resolveNewPreview?.({
    kind: 'text',
    mime_type: 'text/plain',
    file_name: 'new.txt',
    metadata: {},
    preview: { text: 'new preview text' },
  });

  await waitFor(() => expect(screen.getByText('new preview text')).toBeInTheDocument());

  resolveOldPreview?.({
    kind: 'text',
    mime_type: 'text/plain',
    file_name: 'old.txt',
    metadata: {},
    preview: { text: 'old preview text' },
  });

  await waitFor(() => expect(screen.getByText('new preview text')).toBeInTheDocument());
  expect(screen.queryByText('old preview text')).not.toBeInTheDocument();
});

test('timeline focus reseeds the run fragment graph when selecting another step', async () => {
  mockGetDebugStream.mockResolvedValue({
    status: 'ok',
    run_id: 'run-focus-sync-ui',
    pipeline_id: 'ingestion-early-parse',
    pipeline_run_id: 'pipe-focus-sync-ui',
    execution_mode: 'manual',
    execution_state: 'paused',
    control_availability: { can_resume: true, can_next_step: true },
    events: [
      {
        event_id: 'ev-focus-sync-ui-1',
        step_id: 'step-focus-sync-ui-1',
        step_name: 'parse.chunk',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-focus-sync-ui',
      },
      {
        event_id: 'ev-focus-sync-ui-2',
        step_id: 'step-focus-sync-ui-2',
        step_name: 'parse.embed',
        status: 'succeeded',
        attempt_index: 1,
        env_type: 'dev',
        env_id: 'dev-focus-sync-ui',
      },
    ],
  });
  mockGetDebugStepDetail.mockImplementation(async ({ stepId }: { stepId: string }) => {
    if (stepId === 'step-focus-sync-ui-1') {
      return {
        schema_version: 'v1',
        pipeline_id: 'ingestion-early-parse',
        pipeline_run_id: 'pipe-focus-sync-ui',
        run_id: 'run-focus-sync-ui',
        step_id: 'step-focus-sync-ui-1',
        step_name: 'parse.chunk',
        attempt_index: 1,
        outcome: { status: 'succeeded', duration_ms: 11, env_type: 'dev', env_id: 'dev-focus-sync-ui' },
        why: { summary: 'Chunked alpha', policy: { name: 'parse.chunk', params: {} } },
        inputs: { artifact_ids: ['artifact:alpha-md'], fragment_ids: [], program_ids: [] },
        outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
        checks: [],
        transition_validation: {
          specs: [
            { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
          ],
          resolved_inputs: {},
          resolved_outputs: {
            chunk_extraction_set: [
              {
                value: { kind: 'chunk_extraction_set', extraction_refs: ['frag-focus-sync-chunk-a'] },
                inspection_stub: { inspection_ref: 'inspect://fragment/frag-focus-sync-chunk-a' },
              },
            ],
          },
          results: [
            { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
          ],
        },
        lineage: { roots: [], nodes: [], edges: [] },
      };
    }
    return {
      schema_version: 'v1',
      pipeline_id: 'ingestion-early-parse',
      pipeline_run_id: 'pipe-focus-sync-ui',
      run_id: 'run-focus-sync-ui',
      step_id: 'step-focus-sync-ui-2',
      step_name: 'parse.embed',
      attempt_index: 1,
      outcome: { status: 'succeeded', duration_ms: 13, env_type: 'dev', env_id: 'dev-focus-sync-ui' },
      why: { summary: 'Chunked beta', policy: { name: 'parse.embed', params: {} } },
      inputs: { artifact_ids: ['artifact:beta-md'], fragment_ids: [], program_ids: [] },
      outputs: { artifact_ids: [], fragment_ids: [], program_ids: [], pair_ids: [] },
      checks: [],
      transition_validation: {
        specs: [
          { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', config: { schema: { title: 'chunk_extraction_set' } } },
        ],
        resolved_inputs: {},
        resolved_outputs: {
          chunk_extraction_set: [
            {
              value: { kind: 'chunk_extraction_set', extraction_refs: ['frag-focus-sync-chunk-b'] },
              inspection_stub: { inspection_ref: 'inspect://fragment/frag-focus-sync-chunk-b' },
            },
          ],
        },
        results: [
          { name: 'output-chunk-extraction-set', direction: 'output', kind: 'type', status: 'passed', matched_fragment_ids: ['output.chunk_extraction_set'] },
        ],
      },
      lineage: { roots: [], nodes: [], edges: [] },
    };
  });
  mockGetInspectionSubgraph.mockImplementation(async ({ ref }: { ref: string }) => {
    if (ref === 'inspect://fragment/frag-focus-sync-chunk-a') {
      return {
        schema_version: 'v1',
        root_node_id: 'fragment:frag-focus-sync-chunk-a',
        navigation: { focus: { node_id: 'fragment:frag-focus-sync-chunk-a' } },
        nodes: [
          {
            id: 'fragment:frag-focus-sync-chunk-a',
            kind: 'fragment',
            ir_kind: 'chunk_extraction',
            label: 'alpha:chunk:0',
            payload: { cas_id: 'frag-focus-sync-chunk-a', value: { chunk_id: 'alpha:chunk:0' } },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-focus-sync-chunk-a' } } },
          },
        ],
        edges: [],
      };
    }
    if (ref === 'inspect://fragment/frag-focus-sync-chunk-b') {
      return {
        schema_version: 'v1',
        root_node_id: 'fragment:frag-focus-sync-chunk-b',
        navigation: { focus: { node_id: 'fragment:frag-focus-sync-chunk-b' } },
        nodes: [
          {
            id: 'fragment:frag-focus-sync-chunk-b',
            kind: 'fragment',
            ir_kind: 'chunk_extraction',
            label: 'beta:chunk:0',
            payload: { cas_id: 'frag-focus-sync-chunk-b', value: { chunk_id: 'beta:chunk:0' } },
            refs: { self: { backend: 'hot', locator: { cas_id: 'frag-focus-sync-chunk-b' } } },
          },
        ],
        edges: [],
      };
    }
    throw new Error(`Unexpected inspection ref: ${ref}`);
  });

  render(
    <RunsWorkspace
      cases={[{ case_id: 's-local-retail-v01', domain: 'retail', size_tier: 's' }]}
      loadingCases={false}
      caseError={null}
      selectedCaseIds={[]}
      onToggleCase={() => {}}
      reset={false}
      onResetChange={() => {}}
      running={false}
      runError={null}
      onRunCases={() => {}}
      runs={[{
        run_id: 'run-focus-sync-ui',
        project_id: 'proj-focus-sync-ui',
        case_id: 's-local-retail-v01',
        stages: [],
        decisions: [],
        evaluation: {
          report: {
            compression: { total_fragments: 1, unique_fragments: 1, total_bytes: 1, unique_bytes: 1, dedup_ratio: 0 },
            entities: { coverage: 1, passed: true },
            predicates: { predicate_coverage: 1, contradiction_coverage: 1, passed: true },
            exploration: { mean_recall: 1, passed: true },
            query: { mean_fact_coverage: 1, mean_quality_score: 10, passed: true },
            passed: true,
          },
          rendered: 'ok',
          details: { debug_pipeline: { pipeline_id: 'ingestion-early-parse', pipeline_run_id: 'pipe-focus-sync-ui' } },
        },
      }]}
      activeRunId="run-focus-sync-ui"
      onSelectRun={() => {}}
    />
  );

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalledWith({ runId: 'run-focus-sync-ui', stepId: 'step-focus-sync-ui-1' }));
  await waitFor(() => expect(mockGetInspectionSubgraph).toHaveBeenCalledWith({
    runId: 'run-focus-sync-ui',
    ref: 'inspect://fragment/frag-focus-sync-chunk-a',
    maxDepth: 1,
  }));
  await waitFor(() => expect(screen.getByTestId('run-fragment-graph-panel')).toBeInTheDocument());
  expect(within(screen.getByTestId('run-fragment-graph-panel')).getByRole('button', { name: /alpha:chunk:0/i })).toHaveClass('inspection-graph-node-button-root');

  fireEvent.click(screen.getByRole('button', { name: /parse\.embed.*succeeded/i }));

  await waitFor(() => expect(mockGetDebugStepDetail).toHaveBeenCalledWith({ runId: 'run-focus-sync-ui', stepId: 'step-focus-sync-ui-2' }));
  await waitFor(() => expect(mockGetInspectionSubgraph).toHaveBeenCalledWith({
    runId: 'run-focus-sync-ui',
    ref: 'inspect://fragment/frag-focus-sync-chunk-b',
    maxDepth: 1,
  }));
  await waitFor(() => expect(within(screen.getByTestId('run-fragment-graph-panel')).getByRole('button', { name: /beta:chunk:0/i })).toHaveClass('inspection-graph-node-button-root'));
  expect(within(screen.getByTestId('run-fragment-graph-panel')).queryByRole('button', { name: /alpha:chunk:0/i })).not.toBeInTheDocument();
});
