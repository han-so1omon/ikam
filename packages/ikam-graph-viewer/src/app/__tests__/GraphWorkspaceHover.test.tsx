import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import GraphWorkspace from '../components/GraphWorkspace';
import type { GraphData, GraphOptions } from '../../types';
import { GRAPH_SEARCH_RESULT_TYPE } from '../api/client';
import type { GraphSearchResult, RunEntry } from '../api/client';

const mockCreateIKAMGraph = vi.fn();
const mockFetch = vi.fn();
let anchorClickSpy: ReturnType<typeof vi.spyOn>;

vi.mock('../../index', async () => {
  const actual = await vi.importActual<typeof import('../../index')>('../../index');
  return {
    ...actual,
    createIKAMGraph: (...args: any[]) => mockCreateIKAMGraph(...args),
    buildGraphStats: () => ({ nodes: 1, edges: 1, nodeKinds: [], edgeKinds: [] }),
  };
});

const activeRun = {
  run_id: 'run-1',
  project_id: 'case-1',
  graph_id: 'graph-1',
  case_id: 'case-1',
  stages: [],
  decisions: [],
  answer_quality: {
    aqs: 0.82,
    review_mode: 'oracle-defaulted',
    review_coverage: 0,
    query_scores: [
      {
        query_id: 'q-1',
        score: 0.72,
        review_mode: 'oracle-defaulted',
      },
    ],
  },
} as RunEntry;

const graphData: GraphData = {
  nodes: [
    {
      id: 'node-1',
      type: 'fragment',
      label: 'Revenue Node',
      meta: {
        origin: 'semantic',
        semantic_entity_ids: ['entity-1'],
        semantic_entity_id: 'entity-1',
        semantic_entity_label: 'Revenue',
        semantic_relation_ids: ['relation-1'],
      },
    },
  ],
  edges: [
    {
      id: 'edge-1',
      source: 'node-1',
      target: 'node-1',
      kind: 'composition',
      meta: {
        origin: 'map',
        semantic_relation_ids: ['relation-9'],
      },
    },
  ],
};

const renderWorkspace = () => {
  render(
    <GraphWorkspace
      activeRun={activeRun}
      summary={{ nodes: 1, edges: 1, semantic_entities: 1, semantic_relations: 1 }}
      loadingSummary={false}
      summaryError={null}
      decisions={[]}
      loadingDecisions={false}
      decisionError={null}
      graphData={graphData}
      loadingGraphData={false}
      graphDataError={null}
      semanticEntities={[]}
      semanticRelations={[]}
      onSaveReview={async () => {}}
    />
  );
};

beforeEach(() => {
  mockFetch.mockReset();
  vi.stubGlobal('fetch', mockFetch);
  anchorClickSpy = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
});

afterEach(() => {
  anchorClickSpy.mockRestore();
  vi.unstubAllGlobals();
});

test('renders hover preview for nodes and edges', async () => {
  let capturedOptions: GraphOptions | undefined;
  mockCreateIKAMGraph.mockImplementation((_container: HTMLElement, _data: GraphData, options: GraphOptions) => {
    capturedOptions = options;
    return {
      update: vi.fn(),
      setOptions: vi.fn(),
      focusNode: vi.fn(),
      fitToNodes: vi.fn(),
      fitToGroup: vi.fn(),
      fitToData: vi.fn(),
      destroy: vi.fn(),
    };
  });

  renderWorkspace();
  await waitFor(() => expect(mockCreateIKAMGraph).toHaveBeenCalled());

  const mapShell = await screen.findByTestId('graph-map-shell');
  vi.spyOn(mapShell, 'getBoundingClientRect').mockReturnValue({
    x: 0,
    y: 0,
    left: 0,
    top: 0,
    right: 800,
    bottom: 600,
    width: 800,
    height: 600,
    toJSON: () => {},
  } as DOMRect);

  fireEvent.pointerMove(mapShell, { clientX: 120, clientY: 80 });
  act(() => {
    capturedOptions?.onPointerMove?.({ x: 120, y: 80 });
    capturedOptions?.onNodeHover?.(graphData.nodes[0]);
  });

  await waitFor(() => expect(screen.getByTestId('hover-preview')).toBeInTheDocument());
  expect(screen.getByText('Revenue Node')).toBeInTheDocument();
  expect(screen.getByText(/Origin · semantic/i)).toBeInTheDocument();
  expect(screen.getByText(/Entities:\s*entity-1/i)).toBeInTheDocument();

  act(() => {
    capturedOptions?.onEdgeHover?.(graphData.edges[0]);
  });

  expect(screen.getByText('composition')).toBeInTheDocument();
  expect(screen.getByText(/Relations:\s*relation-9/i)).toBeInTheDocument();
});

test('renders a locked selection drawer', () => {
  renderWorkspace();
  expect(screen.queryByTestId('graph-drawer')).not.toBeInTheDocument();
  expect(screen.queryByTestId('graph-inspector')).not.toBeInTheDocument();
});

test('keeps KPI strip and review panel visible in glass overlays', async () => {
  renderWorkspace();
  expect(await screen.findByTestId('kpi-strip')).toBeInTheDocument();
  const reviewPanel = await screen.findByTestId('review-panel');
  expect(reviewPanel).toBeInTheDocument();
  expect(reviewPanel).toHaveClass('glass-panel');
  expect(screen.getByRole('button', { name: /save review/i })).toBeEnabled();
});

test('shows cached suggestions while discovery runs', async () => {
  let resolveFirst: ((value: any) => void) | null = null;
  let resolveSecond: ((value: any) => void) | null = null;
  const firstPromise = new Promise((resolve) => {
    resolveFirst = resolve;
  });
  const secondPromise = new Promise((resolve) => {
    resolveSecond = resolve;
  });
  mockFetch.mockReturnValueOnce(firstPromise as Promise<Response>);
  mockFetch.mockReturnValueOnce(secondPromise as Promise<Response>);

  renderWorkspace();
  const input = screen.getByPlaceholderText('Search semantic group');

  fireEvent.change(input, { target: { value: 'rev' } });
  expect(screen.getByText('Press Enter to search')).toBeInTheDocument();

  fireEvent.submit(input.closest('form')!);
  expect(screen.getByText('Revenue')).toBeInTheDocument();

  fireEvent.change(input, { target: { value: 'zzz' } });
  fireEvent.submit(input.closest('form')!);
  expect(screen.getByText('Revenue')).toBeInTheDocument();

  await act(async () => {
    resolveSecond?.({
      ok: true,
      json: async () => ({ query: 'zzz', results: [], groups: [], evidence_paths: [], explanations: [], scores: [] }),
    });
  });

  await waitFor(() => expect(screen.getByText(/No matches/i)).toBeInTheDocument());

  await act(async () => {
    resolveFirst?.({
      ok: true,
      json: async () => ({
        query: 'rev',
        results: [{ node_id: 'node-1', group_ids: ['group-1'], confidence: 0.9 }],
        groups: [{ id: 'group-1', label: 'Revenue', size: 1, centroid_node_id: 'node-1' }],
        evidence_paths: [],
        explanations: [],
        scores: [{ node_id: 'node-1', semantic: 0.8, graph: 0.7, evidence: 0.6, confidence: 0.9 }],
      }),
    });
  });
});

test('lists group labels from graph search results', async () => {
  let resolveFetch: ((value: any) => void) | null = null;
  const fetchPromise = new Promise((resolve) => {
    resolveFetch = resolve;
  });
  mockFetch.mockReturnValueOnce(fetchPromise as Promise<Response>);

  renderWorkspace();
  const input = screen.getByPlaceholderText('Search semantic group');

  fireEvent.change(input, { target: { value: 'growth drivers' } });
  fireEvent.submit(input.closest('form')!);

  expect(screen.getByText('Revenue')).toBeInTheDocument();

  await act(async () => {
    resolveFetch?.({
      ok: true,
      json: async () => ({
        query: 'growth drivers',
        results: [{ node_id: 'node-1', group_ids: ['group-4'], confidence: 0.9 }],
        groups: [{ id: 'group-4', label: 'Unit Economics', size: 4, centroid_node_id: 'node-1' }],
        evidence_paths: [{ node_id: 'node-1', path: [{ node_id: 'node-1' }] }],
        explanations: [{ node_id: 'node-1', summary: 'Evidence summary.' }],
        scores: [{ node_id: 'node-1', semantic: 0.8, graph: 0.7, evidence: 0.6, confidence: 0.9 }],
      }),
    });
  });

  await waitFor(() => expect(screen.getByText('Unit Economics')).toBeInTheDocument());
});

test('focuses first search result and highlights nodes', async () => {
  const updateSpy = vi.fn();
  const focusNodeSpy = vi.fn();
  const fitToNodesSpy = vi.fn();
  const setOptionsSpy = vi.fn();
  mockCreateIKAMGraph.mockImplementation((_container: HTMLElement, _data: GraphData, options: GraphOptions) => {
    return {
      update: updateSpy,
      setOptions: setOptionsSpy,
      focusNode: focusNodeSpy,
      fitToNodes: fitToNodesSpy,
      fitToGroup: vi.fn(),
      fitToData: vi.fn(),
      destroy: vi.fn(),
    };
  });

  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: async () => ({
      query: 'revenue',
      results: [
        { node_id: 'node-1', group_ids: ['group-1'], confidence: 0.9 },
        { node_id: 'node-2', group_ids: ['group-1'], confidence: 0.8 },
      ],
      groups: [{ id: 'group-1', label: 'Revenue', size: 1 }],
      evidence_paths: [],
      explanations: [],
      scores: [
        { node_id: 'node-1', semantic: 0.8, graph: 0.7, evidence: 0.6, confidence: 0.9 },
        { node_id: 'node-2', semantic: 0.7, graph: 0.6, evidence: 0.55, confidence: 0.8 },
      ],
    }),
  } as Response);

  renderWorkspace();

  const input = screen.getByPlaceholderText('Search semantic group');
  fireEvent.change(input, { target: { value: 'revenue' } });
  fireEvent.submit(input.closest('form')!);

  await waitFor(() => expect(focusNodeSpy).toHaveBeenCalledWith('node-1'));
  await waitFor(() => expect(fitToNodesSpy).toHaveBeenCalledWith(['node-1', 'node-2']));
  await waitFor(() => expect(updateSpy).toHaveBeenCalled());

  const updatedGraph = updateSpy.mock.calls[0][0] as GraphData;
  const highlightedNode = updatedGraph.nodes.find((node) => node.id === 'node-1');
  expect(highlightedNode?.meta?.highlighted).toBe(true);
});

test('boosts node and edge visibility defaults', async () => {
  let capturedOptions: GraphOptions | undefined;
  mockCreateIKAMGraph.mockImplementation((_container: HTMLElement, _data: GraphData, options: GraphOptions) => {
    capturedOptions = options;
    return {
      update: vi.fn(),
      setOptions: vi.fn(),
      focusNode: vi.fn(),
      fitToNodes: vi.fn(),
      fitToGroup: vi.fn(),
      fitToData: vi.fn(),
      destroy: vi.fn(),
    };
  });

  renderWorkspace();
  await waitFor(() => expect(mockCreateIKAMGraph).toHaveBeenCalled());

  expect(capturedOptions?.nodeSize).toBe(12);
  expect(capturedOptions?.edgeOpacity).toBe(0.9);
  expect(capturedOptions?.background).toBe('#edf3fb');
});

test('renders overlay markers for highlighted nodes', async () => {
  mockCreateIKAMGraph.mockImplementation((_container: HTMLElement, _data: GraphData, _options: GraphOptions) => {
    return {
      update: vi.fn(),
      setOptions: vi.fn(),
      focusNode: vi.fn(),
      fitToNodes: vi.fn(),
      fitToGroup: vi.fn(),
      fitToData: vi.fn(),
      destroy: vi.fn(),
    };
  });

  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: async () => ({
      query: 'revenue',
      results: [
        { node_id: 'node-1', group_ids: ['group-1'], confidence: 0.9 },
        { node_id: 'node-2', group_ids: ['group-1'], confidence: 0.8 },
      ],
      groups: [{ id: 'group-1', label: 'Revenue', size: 2 }],
      evidence_paths: [],
      explanations: [
        {
          node_id: 'node-1',
          summary: 'Matched query evidence with weighted signals.',
          reasons: { text_match_tokens: ['revenue'], relation_matches: [], graph_degree: 2 },
        },
      ],
      scores: [
        { node_id: 'node-1', semantic: 0.8, graph: 0.7, evidence: 0.6, confidence: 0.9 },
        { node_id: 'node-2', semantic: 0.7, graph: 0.6, evidence: 0.5, confidence: 0.8 },
      ],
    }),
  } as Response);

  renderWorkspace();
  const input = screen.getByPlaceholderText('Search semantic group');
  fireEvent.change(input, { target: { value: 'revenue' } });
  fireEvent.submit(input.closest('form')!);

  await waitFor(() => expect(screen.getAllByTestId('graph-marker')).toHaveLength(2));
  expect(screen.getAllByTitle(/Matched query evidence/i).length).toBeGreaterThan(0);
});

test('opens inspector when clicking search marker', async () => {
  mockCreateIKAMGraph.mockImplementation((_container: HTMLElement, _data: GraphData, _options: GraphOptions) => {
    return {
      update: vi.fn(),
      setOptions: vi.fn(),
      focusNode: vi.fn(),
      fitToNodes: vi.fn(),
      fitToGroup: vi.fn(),
      fitToData: vi.fn(),
      destroy: vi.fn(),
    };
  });

  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: async () => ({
      query: 'revenue',
      results: [{ node_id: 'node-1', group_ids: ['group-1'], confidence: 0.9 }],
      groups: [{ id: 'group-1', label: 'Revenue', size: 1 }],
      evidence_paths: [],
      explanations: [
        {
          node_id: 'node-1',
          summary: 'Matched query evidence with weighted signals.',
          reasons: { text_match_tokens: ['revenue'], relation_matches: [], graph_degree: 2 },
        },
      ],
      scores: [{ node_id: 'node-1', semantic: 0.8, graph: 0.7, evidence: 0.6, confidence: 0.9 }],
    }),
  } as Response);

  renderWorkspace();
  const input = screen.getByPlaceholderText('Search semantic group');
  fireEvent.change(input, { target: { value: 'revenue' } });
  fireEvent.submit(input.closest('form')!);

  const marker = await screen.findByRole('button', { name: /Revenue Node/i });
  fireEvent.click(marker);

  await waitFor(() => expect(screen.getByTestId('graph-inspector')).toBeInTheDocument());
  expect(screen.getByText('Why Chosen')).toBeInTheDocument();
});

test('shows full explanation details in inspector', async () => {
  let capturedOptions: GraphOptions | undefined;
  mockCreateIKAMGraph.mockImplementation((_container: HTMLElement, _data: GraphData, options: GraphOptions) => {
    capturedOptions = options;
    return {
      update: vi.fn(),
      setOptions: vi.fn(),
      focusNode: vi.fn(),
      fitToNodes: vi.fn(),
      fitToGroup: vi.fn(),
      fitToData: vi.fn(),
      destroy: vi.fn(),
    };
  });

  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: async () => ({
      query: 'revenue',
      results: [{ node_id: 'node-1', group_ids: ['group-1'], confidence: 0.9 }],
      groups: [{ id: 'group-1', label: 'Revenue', size: 1 }],
      evidence_paths: [],
      explanations: [
        {
          node_id: 'node-1',
          summary: 'Matched query evidence with weighted signals.',
          reasons: {
            text_match_tokens: ['revenue'],
            relation_matches: ['evaluated'],
            graph_degree: 12,
            weights: { text: 0.4, relation: 0.35, graph: 0.25 },
          },
        },
      ],
      scores: [{ node_id: 'node-1', semantic: 0.8, graph: 0.7, evidence: 0.6, confidence: 0.9 }],
    }),
  } as Response);

  renderWorkspace();
  const input = screen.getByPlaceholderText('Search semantic group');
  fireEvent.change(input, { target: { value: 'revenue' } });
  fireEvent.submit(input.closest('form')!);

  await waitFor(() => expect(capturedOptions).toBeDefined());
  act(() => {
    capturedOptions?.onSelectionChange?.({ selectedNodeId: 'node-1', selectedEdgeId: undefined, selectedNodeIds: [] });
  });

  await waitFor(() => expect(screen.getByText('Why Chosen')).toBeInTheDocument());
  expect(screen.getByText(/Matched query evidence/i)).toBeInTheDocument();
  const explanation = screen.getByText('Why Chosen').closest('.inspector-explanation') as HTMLElement;
  expect(explanation).toBeTruthy();
  expect(explanation).toHaveTextContent(/revenue/i);
  expect(explanation).toHaveTextContent(/evaluated/i);
  expect(explanation).toHaveTextContent(/degree/i);
  expect(explanation).toHaveTextContent(/0.4/);
  expect(screen.getByTestId('inspector-json')).toHaveClass('inspector-json-compact');
  expect(screen.getByRole('button', { name: /copy evidence/i })).toBeInTheDocument();
});

test('copies explanation details to clipboard', async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.assign(navigator, { clipboard: { writeText } });
  let capturedOptions: GraphOptions | undefined;
  mockCreateIKAMGraph.mockImplementation((_container: HTMLElement, _data: GraphData, options: GraphOptions) => {
    capturedOptions = options;
    return {
      update: vi.fn(),
      setOptions: vi.fn(),
      focusNode: vi.fn(),
      fitToNodes: vi.fn(),
      fitToGroup: vi.fn(),
      fitToData: vi.fn(),
      destroy: vi.fn(),
    };
  });

  mockFetch.mockResolvedValueOnce({
    ok: true,
    json: async () => ({
      query: 'revenue',
      results: [{ node_id: 'node-1', group_ids: ['group-1'], confidence: 0.9 }],
      groups: [{ id: 'group-1', label: 'Revenue', size: 1 }],
      evidence_paths: [],
      explanations: [
        {
          node_id: 'node-1',
          summary: 'Matched query evidence with weighted signals.',
          reasons: {
            text_match_tokens: ['revenue'],
            relation_matches: ['evaluated'],
            graph_degree: 12,
            weights: { text: 0.4, relation: 0.35, graph: 0.25 },
          },
        },
      ],
      scores: [{ node_id: 'node-1', semantic: 0.8, graph: 0.7, evidence: 0.6, confidence: 0.9 }],
    }),
  } as Response);

  renderWorkspace();
  const input = screen.getByPlaceholderText('Search semantic group');
  fireEvent.change(input, { target: { value: 'revenue' } });
  fireEvent.submit(input.closest('form')!);

  await waitFor(() => expect(capturedOptions).toBeDefined());
  act(() => {
    capturedOptions?.onSelectionChange?.({ selectedNodeId: 'node-1', selectedEdgeId: undefined, selectedNodeIds: [] });
  });

  const button = await screen.findByRole('button', { name: /copy evidence/i });
  fireEvent.click(button);
  expect(writeText).toHaveBeenCalled();
});

test('exposes graph search result type', () => {
  expect(GRAPH_SEARCH_RESULT_TYPE).toBe('GraphSearchResult');

  const sample: GraphSearchResult = {
    query: 'growth drivers',
    results: [{ node_id: 'node-1', group_ids: ['group-1'], confidence: 0.9 }],
    groups: [{ id: 'group-1', label: 'Revenue', size: 1, centroid_node_id: 'node-1' }],
    evidence_paths: [{ node_id: 'node-1', path: [{ node_id: 'node-1' }] }],
    explanations: [{ node_id: 'node-1', summary: 'Evidence summary.' }],
    scores: [{ node_id: 'node-1', semantic: 0.8, graph: 0.7, evidence: 0.6, confidence: 0.9 }],
  };

  expect(sample.query).toBe('growth drivers');
});

test('shows render artifact button and render chain for artifact selection', async () => {
  let capturedOptions: GraphOptions | undefined;
  mockCreateIKAMGraph.mockImplementation((_container: HTMLElement, _data: GraphData, options: GraphOptions) => {
    capturedOptions = options;
    return {
      update: vi.fn(),
      setOptions: vi.fn(),
      focusNode: vi.fn(),
      fitToNodes: vi.fn(),
      fitToGroup: vi.fn(),
      fitToData: vi.fn(),
      destroy: vi.fn(),
    };
  });

  const artifactGraphData: GraphData = {
    nodes: [
      {
        id: 'artifact-1',
        type: 'artifact',
        label: 'Revenue Plan 2026',
        meta: { origin: 'map' },
      },
      {
        id: 'node-1',
        type: 'text',
        label: 'Projected revenue 2026',
        meta: { origin: 'map' },
      },
    ],
    edges: [
      {
        id: 'edge-1',
        source: 'artifact-1',
        target: 'node-1',
        kind: 'artifact-root',
        meta: { origin: 'map' },
      },
    ],
  };

  render(
    <GraphWorkspace
      activeRun={activeRun}
      summary={{ nodes: 2, edges: 1, semantic_entities: 1, semantic_relations: 1 }}
      loadingSummary={false}
      summaryError={null}
      decisions={[]}
      loadingDecisions={false}
      decisionError={null}
      graphData={artifactGraphData}
      loadingGraphData={false}
      graphDataError={null}
      semanticEntities={[]}
      semanticRelations={[]}
      onSaveReview={async () => {}}
    />
  );

  await waitFor(() => expect(capturedOptions).toBeDefined());
  act(() => {
    capturedOptions?.onSelectionChange?.({ selectedNodeId: 'artifact-1', selectedEdgeId: undefined, selectedNodeIds: [] });
  });

  expect(await screen.findByRole('button', { name: /render artifact/i })).toBeInTheDocument();
  expect(screen.getByText('Render Chain')).toBeInTheDocument();
  const renderChainPanel = screen.getByTestId('render-chain');
  expect(renderChainPanel).toHaveTextContent(/Revenue Plan 2026/i);
  expect(renderChainPanel).toHaveTextContent(/Projected revenue 2026/i);
});

test('render artifact highlights render chain nodes and edges', async () => {
  let capturedOptions: GraphOptions | undefined;
  const updateSpy = vi.fn();
  const fitToNodesSpy = vi.fn();
  const focusNodeSpy = vi.fn();
  mockCreateIKAMGraph.mockImplementation((_container: HTMLElement, _data: GraphData, options: GraphOptions) => {
    capturedOptions = options;
    return {
      update: updateSpy,
      setOptions: vi.fn(),
      focusNode: focusNodeSpy,
      fitToNodes: fitToNodesSpy,
      fitToGroup: vi.fn(),
      fitToData: vi.fn(),
      destroy: vi.fn(),
    };
  });

  const artifactGraphData: GraphData = {
    nodes: [
      { id: 'artifact-1', type: 'artifact', label: 'Revenue Plan 2026', meta: { origin: 'map' } },
      { id: 'node-1', type: 'text', label: 'Projected revenue 2026', meta: { origin: 'map' } },
      { id: 'node-2', type: 'binary', label: 'Workbook', meta: { origin: 'map' } },
    ],
    edges: [
      { id: 'edge-1', source: 'artifact-1', target: 'node-1', kind: 'artifact-root', meta: { origin: 'map' } },
      { id: 'edge-2', source: 'artifact-1', target: 'node-2', kind: 'artifact-root', meta: { origin: 'map' } },
    ],
  };

  render(
    <GraphWorkspace
      activeRun={activeRun}
      summary={{ nodes: 3, edges: 2, semantic_entities: 1, semantic_relations: 1 }}
      loadingSummary={false}
      summaryError={null}
      decisions={[]}
      loadingDecisions={false}
      decisionError={null}
      graphData={artifactGraphData}
      loadingGraphData={false}
      graphDataError={null}
      semanticEntities={[]}
      semanticRelations={[]}
      onSaveReview={async () => {}}
    />
  );

  await waitFor(() => expect(capturedOptions).toBeDefined());
  act(() => {
    capturedOptions?.onSelectionChange?.({ selectedNodeId: 'artifact-1', selectedEdgeId: undefined, selectedNodeIds: [] });
  });

  const renderButton = await screen.findByRole('button', { name: /render artifact/i });
  fireEvent.click(renderButton);

  await waitFor(() => expect(updateSpy).toHaveBeenCalled());
  expect(fitToNodesSpy).toHaveBeenCalledWith(['artifact-1', 'node-1', 'node-2']);
  expect(focusNodeSpy).toHaveBeenCalledWith('artifact-1');
});

test('render chain sets meta.pulse on render-chain edges for animation', async () => {
  let capturedOptions: GraphOptions | undefined;
  const updateSpy = vi.fn();
  mockCreateIKAMGraph.mockImplementation((_container: HTMLElement, _data: GraphData, options: GraphOptions) => {
    capturedOptions = options;
    return {
      update: updateSpy,
      setOptions: vi.fn(),
      focusNode: vi.fn(),
      fitToNodes: vi.fn(),
      fitToGroup: vi.fn(),
      fitToData: vi.fn(),
      destroy: vi.fn(),
    };
  });

  const artifactGraphData: GraphData = {
    nodes: [
      { id: 'artifact-1', type: 'artifact', label: 'Revenue Plan 2026', meta: { origin: 'map' } },
      { id: 'node-1', type: 'text', label: 'Projected revenue 2026', meta: { origin: 'map' } },
      { id: 'node-other', type: 'text', label: 'Unrelated', meta: { origin: 'other' } },
    ],
    edges: [
      { id: 'edge-chain', source: 'artifact-1', target: 'node-1', kind: 'artifact-root', meta: { origin: 'map' } },
      { id: 'edge-other', source: 'node-other', target: 'node-1', kind: 'semantic', meta: { origin: 'other' } },
    ],
  };

  render(
    <GraphWorkspace
      activeRun={activeRun}
      summary={{ nodes: 3, edges: 2, semantic_entities: 1, semantic_relations: 1 }}
      loadingSummary={false}
      summaryError={null}
      decisions={[]}
      loadingDecisions={false}
      decisionError={null}
      graphData={artifactGraphData}
      loadingGraphData={false}
      graphDataError={null}
      semanticEntities={[]}
      semanticRelations={[]}
      onSaveReview={async () => {}}
    />
  );

  await waitFor(() => expect(capturedOptions).toBeDefined());
  act(() => {
    capturedOptions?.onSelectionChange?.({ selectedNodeId: 'artifact-1', selectedEdgeId: undefined, selectedNodeIds: [] });
  });

  await waitFor(() => expect(updateSpy).toHaveBeenCalled());

  const updatedGraph = updateSpy.mock.calls[updateSpy.mock.calls.length - 1][0] as GraphData;
  const chainEdge = updatedGraph.edges.find((e) => e.id === 'edge-chain');
  const otherEdge = updatedGraph.edges.find((e) => e.id === 'edge-other');

  expect(chainEdge?.meta?.pulse).toBe(true);
  expect(otherEdge?.meta?.pulse).toBeFalsy();
});

test('shows download artifact button for artifact-type nodes', async () => {
  let capturedOptions: GraphOptions | undefined;
  mockCreateIKAMGraph.mockImplementation((_container: HTMLElement, _data: GraphData, options: GraphOptions) => {
    capturedOptions = options;
    return {
      update: vi.fn(),
      setOptions: vi.fn(),
      focusNode: vi.fn(),
      fitToNodes: vi.fn(),
      fitToGroup: vi.fn(),
      fitToData: vi.fn(),
      destroy: vi.fn(),
    };
  });

  const artifactGraphData: GraphData = {
    nodes: [
      {
        id: 'artifact-1',
        type: 'artifact',
        label: 'Revenue Plan 2026',
        meta: { origin: 'map' },
      },
      {
        id: 'node-1',
        type: 'text',
        label: 'Projected revenue 2026',
        meta: { origin: 'map' },
      },
    ],
    edges: [
      {
        id: 'edge-1',
        source: 'artifact-1',
        target: 'node-1',
        kind: 'artifact-root',
        meta: { origin: 'map' },
      },
    ],
  };

  render(
    <GraphWorkspace
      activeRun={activeRun}
      summary={{ nodes: 2, edges: 1, semantic_entities: 1, semantic_relations: 1 }}
      loadingSummary={false}
      summaryError={null}
      decisions={[]}
      loadingDecisions={false}
      decisionError={null}
      graphData={artifactGraphData}
      loadingGraphData={false}
      graphDataError={null}
      semanticEntities={[]}
      semanticRelations={[]}
      onSaveReview={async () => {}}
    />
  );

  await waitFor(() => expect(capturedOptions).toBeDefined());
  act(() => {
    capturedOptions?.onSelectionChange?.({ selectedNodeId: 'artifact-1', selectedEdgeId: undefined, selectedNodeIds: [] });
  });

  expect(await screen.findByRole('button', { name: /render artifact/i })).toBeInTheDocument();
  expect(await screen.findByRole('button', { name: /download artifact/i })).toBeInTheDocument();
});

test('clicking download artifact button triggers artifact download', async () => {
  const createObjectURLSpy = vi.fn().mockReturnValue('blob:mock-url');
  const revokeObjectURLSpy = vi.fn();
  vi.stubGlobal('URL', { ...URL, createObjectURL: createObjectURLSpy, revokeObjectURL: revokeObjectURLSpy });

  let capturedOptions: GraphOptions | undefined;
  mockCreateIKAMGraph.mockImplementation((_container: HTMLElement, _data: GraphData, options: GraphOptions) => {
    capturedOptions = options;
    return {
      update: vi.fn(),
      setOptions: vi.fn(),
      focusNode: vi.fn(),
      fitToNodes: vi.fn(),
      fitToGroup: vi.fn(),
      fitToData: vi.fn(),
      destroy: vi.fn(),
    };
  });

  const artifactGraphData: GraphData = {
    nodes: [
      {
        id: 'artifact-1',
        type: 'artifact',
        label: 'Revenue Plan 2026',
        meta: { origin: 'map' },
      },
      {
        id: 'node-1',
        type: 'text',
        label: 'Projected revenue 2026',
        meta: { origin: 'map' },
      },
    ],
    edges: [
      {
        id: 'edge-1',
        source: 'artifact-1',
        target: 'node-1',
        kind: 'artifact-root',
        meta: { origin: 'map' },
      },
    ],
  };

  mockFetch.mockResolvedValueOnce({
    ok: true,
    blob: async () => new Blob(['file content'], { type: 'application/octet-stream' }),
    headers: new Headers({ 'content-disposition': 'attachment; filename="Revenue Plan 2026"' }),
  } as Response);

  render(
    <GraphWorkspace
      activeRun={activeRun}
      summary={{ nodes: 2, edges: 1, semantic_entities: 1, semantic_relations: 1 }}
      loadingSummary={false}
      summaryError={null}
      decisions={[]}
      loadingDecisions={false}
      decisionError={null}
      graphData={artifactGraphData}
      loadingGraphData={false}
      graphDataError={null}
      semanticEntities={[]}
      semanticRelations={[]}
      onSaveReview={async () => {}}
    />
  );

  await waitFor(() => expect(capturedOptions).toBeDefined());
  act(() => {
    capturedOptions?.onSelectionChange?.({ selectedNodeId: 'artifact-1', selectedEdgeId: undefined, selectedNodeIds: [] });
  });

  const downloadButton = await screen.findByRole('button', { name: /download artifact/i });
  fireEvent.click(downloadButton);

  await waitFor(() => {
    expect(mockFetch).toHaveBeenCalledWith(
      expect.stringContaining('/artifacts/artifact-1/download'),
      expect.any(Object)
    );
  });
  expect(anchorClickSpy).toHaveBeenCalledTimes(1);
});
