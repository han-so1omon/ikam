import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, vi } from 'vitest';

import App from '../App';

const mockFetch = vi.fn();
const { mockCreateIKAMGraph } = vi.hoisted(() => ({
  mockCreateIKAMGraph: vi.fn(() => ({
    update: vi.fn(),
    setOptions: vi.fn(),
    focusNode: vi.fn(),
    fitToNodes: vi.fn(),
    fitToGroup: vi.fn(),
    fitToData: vi.fn(),
    destroy: vi.fn(),
  })),
}));

vi.mock('../../index', () => ({
  createIKAMGraph: mockCreateIKAMGraph,
  deriveSelectionDetails: () => ({ selectedNode: null, selectedEdge: null, selectedNodeIds: [] }),
  buildGraphStats: (graphData: { nodes: unknown[]; edges: unknown[] }) => ({
    nodes: graphData.nodes.length,
    edges: graphData.edges.length,
    nodeKinds: [{ kind: 'fragment', count: graphData.nodes.length }],
    edgeKinds: [{ kind: 'composition', count: graphData.edges.length }],
  }),
}));

beforeEach(() => {
  mockFetch.mockReset();
  mockCreateIKAMGraph.mockClear();
  vi.stubGlobal('fetch', mockFetch);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

const queueDefaultFetches = () => {
  mockFetch.mockImplementation(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes('/benchmarks/cases')) {
      return {
        ok: true,
        json: async () => ({
          cases: [
            { case_id: 's-construction-v01', domain: 'construction', size_tier: 's' },
            { case_id: 's-local-retail-v01', domain: 'local-retail', size_tier: 's' },
          ],
        }),
      };
    }
    if (url.includes('/benchmarks/history')) {
      return { ok: true, json: async () => [] };
    }
    if (url.includes('/registry/petri_net_runnables')) {
      return { ok: true, json: async () => ({ namespace: 'petri_net_runnables', entries: [{ key: 'mock-pipeline/v1' }] }) };
    }
    return { ok: true, json: async () => ({}) };
  });
};

test('uses localhost fallback when env is internal and running locally', async () => {
  const { resolveApiBaseUrl } = await import('../api/client');
  expect(resolveApiBaseUrl({ envValue: 'http://ikam-perf-report-api:8040', hostname: 'localhost' })).toBe(
    'http://localhost:8040'
  );
});

test('keeps internal base URL when not running on localhost', async () => {
  const { resolveApiBaseUrl } = await import('../api/client');
  expect(resolveApiBaseUrl({ envValue: 'http://ikam-perf-report-api:8040', hostname: 'ikam-graph-viewer' })).toBe(
    'http://ikam-perf-report-api:8040'
  );
});

test('renders IKAM Perf Report heading and tabs', async () => {
  queueDefaultFetches();
  render(<App />);
  expect(screen.getByLabelText('Perf report workspaces')).toHaveClass('glass-nav-shell');
  expect(screen.getByTestId('workspace-sidebar')).toHaveAttribute('data-collapsed', 'true');
  expect(screen.queryByText('IKAM Performance Report')).not.toBeInTheDocument();
  const nav = screen.getByRole('navigation');
  expect(within(nav).getByRole('button', { name: 'Runs' })).toBeInTheDocument();
  expect(within(nav).getByRole('button', { name: 'Graph' })).toBeInTheDocument();
  expect(within(nav).getByRole('button', { name: 'Merge' })).toBeInTheDocument();
  expect(within(nav).getByRole('button', { name: 'History' })).toBeInTheDocument();
  expect(within(nav).getByRole('button', { name: 'Wiki' })).toBeInTheDocument();
  await waitFor(() => expect(screen.getByRole('button', { name: 'Run Pipeline' })).toBeInTheDocument());
});

test('collapses workspace nav while keeping tab navigation accessible', async () => {
  queueDefaultFetches();
  render(<App />);

  const sidebar = screen.getByTestId('workspace-sidebar');
  expect(sidebar).toHaveAttribute('data-collapsed', 'true');
  expect(screen.getByRole('button', { name: /expand workspace nav/i })).toHaveAttribute('aria-expanded', 'false');
  expect(within(screen.getByRole('navigation')).queryByText('Runs')).not.toBeInTheDocument();
  expect(within(screen.getByRole('navigation')).getByRole('button', { name: 'Runs' })).toHaveAttribute('title', 'Runs');

  fireEvent.click(screen.getByRole('button', { name: /expand workspace nav/i }));

  expect(screen.getByTestId('workspace-sidebar')).toHaveAttribute('data-collapsed', 'false');
  expect(screen.getByRole('button', { name: /collapse workspace nav/i })).toHaveAttribute('aria-expanded', 'true');
  expect(screen.getAllByText('Runs').length).toBeGreaterThan(0);

  const graphNavButton = within(screen.getByRole('navigation')).getByRole('button', { name: 'Graph' });
  fireEvent.click(graphNavButton);
  expect(graphNavButton).toHaveClass('tab-active');
  expect(graphNavButton).toHaveAttribute('aria-current', 'page');
  await waitFor(() => expect(screen.getByText('Select a run in Runs workspace to inspect graph context')).toBeInTheDocument());

  fireEvent.click(screen.getByRole('button', { name: /collapse workspace nav/i }));
  expect(screen.getByTestId('workspace-sidebar')).toHaveAttribute('data-collapsed', 'true');
  expect(within(screen.getByRole('navigation')).queryByText('Runs')).not.toBeInTheDocument();
});

test('defaults case selector to s-local-retail-v01 and enables run button', async () => {
  queueDefaultFetches();
  render(<App />);

  await waitFor(() => expect(screen.getByText('s-local-retail-v01')).toBeInTheDocument());
  const runButton = screen.getByRole('button', { name: 'Run Pipeline' });
  expect(screen.getByLabelText(/s-local-retail-v01/i)).toBeChecked();
  expect(runButton).not.toBeDisabled();
});

test('shows evaluation panel in runs workspace', async () => {
  queueDefaultFetches();
  render(<App />);

  await waitFor(() => expect(screen.getByRole('button', { name: 'Run Pipeline' })).toBeInTheDocument());
  expect(screen.queryByText('Runs Workspace')).not.toBeInTheDocument();
  expect(screen.getByText('Select a run to view evaluation')).toBeInTheDocument();
});

test('run button keeps runs workspace and requests debug stream', async () => {
  const errorSpy = vi.spyOn(console, 'error').mockImplementation(() => {});
  mockFetch.mockImplementation(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes('/benchmarks/cases')) {
      return {
        ok: true,
        json: async () => ({ cases: [{ case_id: 's-construction-v01', domain: 'construction', size_tier: 's' }] }),
      };
    }
    if (url.includes('/benchmarks/run?')) {
      return {
        ok: true,
        json: async () => ({
          runs: [
            {
              run_id: 'run-debug-1',
              project_id: 's-construction-v01#1',
              graph_id: 's-construction-v01#1',
              case_id: 's-construction-v01',
              stages: [],
              decisions: [],
            },
          ],
        }),
      };
    }
    if (url.includes('/benchmarks/runs/run-debug-1/debug-stream')) {
      return {
        ok: true,
        json: async () => ({
          status: 'ok',
          run_id: 'run-debug-1',
          pipeline_id: 'compression-rerender/v1',
          pipeline_run_id: 'run-debug-1',
          execution_mode: 'manual',
          execution_state: 'paused',
          events: [
            {
              event_id: 'ev-prepare',
              step_id: 'step-prepare',
              step_name: 'init.initialize',
              status: 'succeeded',
              attempt_index: 1,
              env_type: 'dev',
              env_id: 'dev-1',
            },
          ],
        }),
      };
    }
    if (url.includes('/benchmarks/runs')) {
      return {
        ok: true,
        json: async () => [
          {
            run_id: 'run-debug-1',
            project_id: 's-construction-v01#1',
            graph_id: 's-construction-v01#1',
            case_id: 's-construction-v01',
            stages: [],
            decisions: [],
          },
        ],
      };
    }
    if (url.includes('/graph/summary')) {
      return { ok: true, json: async () => ({ graph_id: 's-construction-v01#1', nodes: 1, edges: 1, semantic_entities: 0, semantic_relations: 0 }) };
    }
    if (url.includes('/graph/nodes')) {
      return { ok: true, json: async () => [] };
    }
    if (url.includes('/graph/edges')) {
      return { ok: true, json: async () => [] };
    }
    if (url.includes('/graph/decisions/')) {
      return { ok: true, json: async () => ({ decisions: [] }) };
    }
    if (url.includes('/graph/enrichment/runs')) {
      return { ok: true, json: async () => ({ runs: [] }) };
    }
    if (url.includes('/graph/enrichment/staged')) {
      return { ok: true, json: async () => ({ items: [] }) };
    }
    if (url.includes('/graph/enrichment/receipts')) {
      return { ok: true, json: async () => ({ receipts: [] }) };
    }
    if (url.includes('/graph/wiki')) {
      return { ok: true, json: async () => ({ status: 'missing' }) };
    }
    if (url.includes('/benchmarks/runs/run-debug-1/verification')) {
      return { ok: true, json: async () => ({ status: 'ok', verification_records: [] }) };
    }
    if (url.includes('/benchmarks/runs/run-debug-1/reconstruction-program')) {
      return { ok: true, json: async () => ({ status: 'ok', reconstruction_programs: [] }) };
    }
    if (url.includes('/benchmarks/runs/run-debug-1/env-summary')) {
      return {
        ok: true,
        json: async () => ({
          status: 'ok',
          summary: { fragment_count: 1, verification_count: 0, reconstruction_program_count: 0 },
        }),
      };
    }
    if (url.includes('/registry/petri_net_runnables')) {
      return { ok: true, json: async () => ({ namespace: 'petri_net_runnables', entries: [{ key: 'mock-pipeline/v1' }] }) };
    }
    return { ok: true, json: async () => ({}) };
  });

  render(<App />);
  await waitFor(() => expect(screen.getByLabelText(/s-construction-v01/i)).toBeInTheDocument());
  fireEvent.click(screen.getByLabelText(/s-construction-v01/i));
  fireEvent.click(screen.getByRole('button', { name: 'Run Pipeline' }));

  await waitFor(() => expect(screen.getByRole('button', { name: 'Run Pipeline' })).toBeInTheDocument());
  await waitFor(() => expect(screen.getByTestId('debug-step-stream')).toBeInTheDocument());
  expect(screen.getByTestId('run-debug-layout')).toBeInTheDocument();
  const errorOutput = errorSpy.mock.calls.flatMap((call) => call.map(String)).join('\n');
  expect(errorOutput).not.toContain('Encountered two children with the same key');

  errorSpy.mockRestore();
});

test('shows merge workspace controls', async () => {
  queueDefaultFetches();
  render(<App />);
  fireEvent.click(screen.getByRole('button', { name: 'Merge' }));
  expect(await screen.findByText('Merge Workspace')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Run Merge' })).toBeInTheDocument();
});

test('shows wiki workspace controls', async () => {
  queueDefaultFetches();
  render(<App />);
  fireEvent.click(screen.getByRole('button', { name: 'Wiki' }));
  expect(await screen.findByText('Wiki Workspace')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: 'Generate Wiki' })).toBeInTheDocument();
});

test('shows history workspace controls', async () => {
  mockFetch.mockImplementation(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes('/benchmarks/cases')) {
      return {
        ok: true,
        json: async () => ({ cases: [{ case_id: 's-construction-v01', domain: 'construction', size_tier: 's' }] }),
      };
    }
    if (url.includes('/benchmarks/runs')) {
      return {
        ok: true,
        json: async () => [
          {
            run_id: 'run-history',
            project_id: 's-construction-v01#1',
            graph_id: 's-construction-v01#1',
            case_id: 's-construction-v01',
            stages: [],
            decisions: [],
          },
        ],
      };
    }
    if (url.includes('/history/refs')) {
      return {
        ok: true,
        json: async () => ({ refs: [{ ref: 'refs/heads/main', commit_id: 'commit-1' }] }),
      };
    }
    if (url.includes('/history/commits/commit-1')) {
      return {
        ok: true,
        json: async () => ({
          commit: {
            id: 'commit-1',
            profile: 'modelado/commit-entry@1',
            content: { ref: 'refs/heads/main', commit_policy: 'semantic_relations_only', parents: [] },
          },
        }),
      };
    }
    if (url.includes('/history/commits?')) {
      return {
        ok: true,
        json: async () => ({ commits: [{ id: 'commit-1', profile: 'modelado/commit-entry@1', content: { ref: 'refs/heads/main' } }] }),
      };
    }
    if (url.includes('/history/commits/commit-1/semantic-graph')) {
      return {
        ok: true,
        json: async () => ({ commit_id: 'commit-1', nodes: [{ id: 'prop-1', kind: 'proposition' }], edges: [] }),
      };
    }
    if (url.includes('/graph/summary')) {
      return { ok: true, json: async () => ({ graph_id: 's-construction-v01#1', nodes: 0, edges: 0, semantic_entities: 0, semantic_relations: 0 }) };
    }
    if (url.includes('/graph/nodes')) {
      return { ok: true, json: async () => [] };
    }
    if (url.includes('/graph/edges')) {
      return { ok: true, json: async () => [] };
    }
    if (url.includes('/graph/decisions/')) {
      return { ok: true, json: async () => ({ decisions: [] }) };
    }
    if (url.includes('/graph/enrichment/runs')) {
      return { ok: true, json: async () => ({ runs: [] }) };
    }
    if (url.includes('/graph/enrichment/staged')) {
      return { ok: true, json: async () => ({ items: [] }) };
    }
    if (url.includes('/graph/enrichment/receipts')) {
      return { ok: true, json: async () => ({ receipts: [] }) };
    }
    if (url.includes('/graph/wiki')) {
      return { ok: true, json: async () => ({ status: 'missing' }) };
    }
    if (url.includes('/registry/petri_net_runnables')) {
      return { ok: true, json: async () => ({ namespace: 'petri_net_runnables', entries: [{ key: 'mock-pipeline/v1' }] }) };
    }
    return { ok: true, json: async () => ({}) };
  });

  render(<App />);
  fireEvent.click(await screen.findByRole('button', { name: 'History' }));
  expect(await screen.findByText('History Workspace')).toBeInTheDocument();
  expect(screen.getByText('refs/heads/main')).toBeInTheDocument();
  expect(screen.getByText('commit-1')).toBeInTheDocument();
});

test('renders graph map for active run', async () => {
  mockFetch.mockImplementation(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes('/benchmarks/cases')) {
      return { ok: true, json: async () => ({ cases: [{ case_id: 's-construction-v01', domain: 'construction', size_tier: 's' }] }) };
    }
    if (url.includes('/benchmarks/runs')) {
      return {
        ok: true,
        json: async () => [
          {
            run_id: 'run-1',
            project_id: 's-construction-v01#1',
            graph_id: 's-construction-v01#1',
            case_id: 's-construction-v01',
            stages: [{ stage_name: 'decompose_artifacts', duration_ms: 10 }],
            decisions: [],
            semantic: {
              entities: [{ id: 'entity-1', kind: 'intent' }],
              relations: [{ id: 'relation-1', kind: 'evaluated' }],
            },
          },
        ],
      };
    }
    if (url.includes('/graph/summary')) {
      return { ok: true, json: async () => ({ graph_id: 's-construction-v01#1', nodes: 1, edges: 1, semantic_entities: 1, semantic_relations: 1 }) };
    }
    if (url.includes('/graph/nodes')) {
      return { ok: true, json: async () => [{ id: 'n1', type: 'fragment', level: 0 }] };
    }
    if (url.includes('/graph/edges')) {
      return { ok: true, json: async () => [{ id: 'e1', source: 'n1', target: 'n1' }] };
    }
    if (url.includes('/graph/decisions/')) {
      return { ok: true, json: async () => ({ decisions: [] }) };
    }
    if (url.includes('/graph/wiki')) {
      return { ok: true, json: async () => ({ status: 'missing' }) };
    }
    if (url.includes('/registry/petri_net_runnables')) {
      return { ok: true, json: async () => ({ namespace: 'petri_net_runnables', entries: [{ key: 'mock-pipeline/v1' }] }) };
    }
    return { ok: true, json: async () => ({}) };
  });

  render(<App />);
  const nav = await screen.findByRole('navigation');
  fireEvent.click(within(nav).getByRole('button', { name: 'Graph' }));

  await waitFor(() => expect(screen.getByLabelText('IKAM graph map')).toBeInTheDocument());
  await waitFor(() => expect(screen.getByTestId('graph-map')).toBeInTheDocument());
  expect(screen.getByText('Graph Workspace')).toBeInTheDocument();
  expect(screen.getByTestId('graph-quick-legend')).toBeInTheDocument();
  expect(screen.getByTestId('graph-legend')).toBeInTheDocument();
  expect(screen.getByTestId('graph-self-loop-note')).toBeInTheDocument();
  expect(screen.getByTestId('graph-color-key')).toBeInTheDocument();
  expect(screen.getByText('Color Key')).toBeInTheDocument();
  expect(screen.getAllByText('Text Fragments').length).toBeGreaterThan(0);
  expect(screen.getByTestId('semantic-explorer')).toBeInTheDocument();
  await waitFor(() => expect(mockCreateIKAMGraph).toHaveBeenCalled());
});

test('shows KPI strip ordered as AQS, Storage, Reliability', async () => {
  mockFetch.mockImplementation(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes('/benchmarks/cases')) {
      return { ok: true, json: async () => ({ cases: [{ case_id: 's-construction-v01', domain: 'construction', size_tier: 's' }] }) };
    }
    if (url.includes('/benchmarks/runs')) {
      return {
        ok: true,
        json: async () => [
          {
            run_id: 'run-1',
            project_id: 's-construction-v01#1',
            graph_id: 's-construction-v01#1',
            case_id: 's-construction-v01',
            stages: [{ stage_name: 'decompose_artifacts', duration_ms: 10 }],
            decisions: [],
            answer_quality: {
              aqs: 0.82,
              review_mode: 'oracle-defaulted',
              review_coverage: 0,
              query_scores: [],
            },
          },
        ],
      };
    }
    if (url.includes('/graph/summary')) {
      return { ok: true, json: async () => ({ graph_id: 's-construction-v01#1', nodes: 10, edges: 15, semantic_entities: 4, semantic_relations: 5 }) };
    }
    if (url.includes('/graph/nodes')) {
      return { ok: true, json: async () => [{ id: 'n1', type: 'fragment', level: 0 }] };
    }
    if (url.includes('/graph/edges')) {
      return { ok: true, json: async () => [{ id: 'e1', source: 'n1', target: 'n1' }] };
    }
    if (url.includes('/graph/decisions/')) {
      return { ok: true, json: async () => ({ decisions: [] }) };
    }
    if (url.includes('/graph/wiki')) {
      return { ok: true, json: async () => ({ status: 'missing' }) };
    }
    if (url.includes('/registry/petri_net_runnables')) {
      return { ok: true, json: async () => ({ namespace: 'petri_net_runnables', entries: [{ key: 'mock-pipeline/v1' }] }) };
    }
    return { ok: true, json: async () => ({}) };
  });

  render(<App />);
  const nav = await screen.findByRole('navigation');
  fireEvent.click(within(nav).getByRole('button', { name: 'Graph' }));

  const labels = await screen.findAllByTestId('kpi-label');
  expect(labels.map((node) => node.textContent)).toEqual(['Answer Quality', 'Storage Gains', 'Reliability']);
});
