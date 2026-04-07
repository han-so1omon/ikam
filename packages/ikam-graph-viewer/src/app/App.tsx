import { useEffect, useMemo, useState } from 'react';

import {
  API_BASE_URL,
  type CaseMeta,
  type Decision,
  type EnrichmentItem,
  type EnrichmentReceipt,
  type EnrichmentRun,
  type GraphEdgeResponse,
  type GraphNodeResponse,
  type GraphSummary,
  type MergeResult,
  type RunEntry,
  type SemanticEntity,
  type SemanticRelation,
  type WikiDocument,
  approveEnrichment,
  commitEnrichmentQueue,
  generateWiki,
  getEnrichmentItems,
  getEnrichmentReceipts,
  getEnrichmentRuns,
  getWiki,
  getCases,
  getDecisions,
  getGraphEdges,
  getGraphNodes,
  getGraphSummary,
  listRuns,
  rejectEnrichment,
  runBenchmarks,
  runMerge,
  saveManualReview,
} from './api/client';
import type { GraphData } from '../types';
import GraphWorkspace from './components/GraphWorkspace';
import HistoryWorkspace from './components/HistoryWorkspace';
import MergeWorkspace from './components/MergeWorkspace';
import RunsWorkspace from './components/RunsWorkspace';
import RegistryWorkspace from './components/RegistryWorkspace';
import WikiWorkspace from './components/WikiWorkspace';

type TabKey = 'runs' | 'graph' | 'merge' | 'history' | 'wiki' | 'registry';

const EMPTY_SUMMARY: GraphSummary = {
  nodes: 0,
  edges: 0,
  semantic_entities: 0,
  semantic_relations: 0,
};

const EMPTY_GRAPH_DATA: GraphData = {
  nodes: [],
  edges: [],
};

const DEFAULT_CASE_ID = 's-local-retail-v01';

const mapGraphData = (nodes: GraphNodeResponse[], edges: GraphEdgeResponse[]): GraphData => {
  const safeNodes = Array.isArray(nodes) ? nodes : [];
  const safeEdges = Array.isArray(edges) ? edges : [];
  const mappedNodes = safeNodes.map((node, index) => ({
    id: String(node.id ?? `node-${index}`),
    type: String(node.kind ?? node.type ?? 'fragment'),
    label: node.label,
    level: typeof node.level === 'number' ? node.level : undefined,
    meta: node.meta,
  }));
  const nodeIds = new Set(mappedNodes.map((node) => node.id));
  const mappedEdges = safeEdges
    .filter((edge) => nodeIds.has(String(edge.source)) && nodeIds.has(String(edge.target)))
    .map((edge, index) => ({
      id: edge.id ?? `edge-${index}`,
      source: String(edge.source),
      target: String(edge.target),
      kind: edge.kind,
      meta: edge.meta,
    }));
  return { nodes: mappedNodes, edges: mappedEdges };
};

const dedupeRunsById = (items: RunEntry[]): RunEntry[] => {
  const seen = new Set<string>();
  const deduped: RunEntry[] = [];
  for (const item of items) {
    if (seen.has(item.run_id)) {
      continue;
    }
    seen.add(item.run_id);
    deduped.push(item);
  }
  return deduped;
};

const App = () => {
  const [activeTab, setActiveTab] = useState<TabKey>('runs');
  const [isSidebarCollapsed, setIsSidebarCollapsed] = useState(true);
  const [cases, setCases] = useState<CaseMeta[]>([]);
  const [loadingCases, setLoadingCases] = useState(false);
  const [caseError, setCaseError] = useState<string | null>(null);

  const [runs, setRuns] = useState<RunEntry[]>([]);
  const [activeRunId, setActiveRunId] = useState<string | null>(null);
  const [selectedCaseIds, setSelectedCaseIds] = useState<string[]>([]);
  const [resetBeforeRun, setResetBeforeRun] = useState(true);
  const [runningCases, setRunningCases] = useState(false);
  const [runError, setRunError] = useState<string | null>(null);

  const [graphSummary, setGraphSummary] = useState<GraphSummary>(EMPTY_SUMMARY);
  const [loadingGraphSummary, setLoadingGraphSummary] = useState(false);
  const [summaryError, setSummaryError] = useState<string | null>(null);
  const [graphData, setGraphData] = useState<GraphData>(EMPTY_GRAPH_DATA);
  const [loadingGraphData, setLoadingGraphData] = useState(false);
  const [graphDataError, setGraphDataError] = useState<string | null>(null);
  const [enrichmentRuns, setEnrichmentRuns] = useState<EnrichmentRun[]>([]);
  const [enrichmentItems, setEnrichmentItems] = useState<EnrichmentItem[]>([]);
  const [enrichmentReceipts, setEnrichmentReceipts] = useState<EnrichmentReceipt[]>([]);

  const [decisions, setDecisions] = useState<Decision[]>([]);
  const [loadingDecisions, setLoadingDecisions] = useState(false);
  const [decisionError, setDecisionError] = useState<string | null>(null);

  const [selectedMergeGraphIds, setSelectedMergeGraphIds] = useState<string[]>([]);
  const [applyMerge, setApplyMerge] = useState(false);
  const [runningMerge, setRunningMerge] = useState(false);
  const [mergeError, setMergeError] = useState<string | null>(null);
  const [mergeResult, setMergeResult] = useState<MergeResult | null>(null);
  const [wiki, setWiki] = useState<WikiDocument | null>(null);
  const [loadingWiki, setLoadingWiki] = useState(false);
  const [wikiError, setWikiError] = useState<string | null>(null);

  const activeRun = useMemo(
    () => runs.find((run) => run.run_id === activeRunId) ?? null,
    [runs, activeRunId]
  );
  const activeGraphId = activeRun ? activeRun.graph_id ?? activeRun.project_id : null;
  const semanticEntities: SemanticEntity[] = activeRun?.semantic?.entities ?? [];
  const semanticRelations: SemanticRelation[] = activeRun?.semantic?.relations ?? [];

  const loadCases = async () => {
    setLoadingCases(true);
    setCaseError(null);
    try {
      const loadedCases = await getCases();
      setCases(loadedCases);
      setSelectedCaseIds((current) => {
        if (current.length > 0) {
          return current;
        }
        return loadedCases.some((item) => item.case_id === DEFAULT_CASE_ID) ? [DEFAULT_CASE_ID] : current;
      });
    } catch (error) {
      setCaseError(error instanceof Error ? error.message : 'Failed loading cases');
    } finally {
      setLoadingCases(false);
    }
  };

  const loadRuns = async () => {
    try {
      const listedRuns = dedupeRunsById(await listRuns());
      setRuns(listedRuns);
      if (!activeRunId && listedRuns[0]) {
        setActiveRunId(listedRuns[0].run_id);
      }
    } catch {
      // Keep prior run data visible if this refresh fails.
    }
  };

  const onToggleCase = (caseId: string) => {
    setSelectedCaseIds((prev) =>
      prev.includes(caseId) ? prev.filter((item) => item !== caseId) : [...prev, caseId]
    );
  };

  const onRunCases = async (pipelineId?: string) => {
    setRunningCases(true);
    setRunError(null);
    try {
      const newRuns = await runBenchmarks({
        caseIds: selectedCaseIds,
        reset: resetBeforeRun,
        includeEvaluation: false,
        pipelineId,
      });
      setRuns((prev) => dedupeRunsById([...newRuns, ...prev]));
      if (newRuns[0]) {
        setActiveRunId(newRuns[0].run_id);
        setActiveTab('runs');
      }
      await loadRuns();
    } catch (error) {
      setRunError(error instanceof Error ? error.message : 'Failed running cases');
    } finally {
      setRunningCases(false);
    }
  };

  const onToggleMergeGraphId = (graphId: string) => {
    setSelectedMergeGraphIds((prev) =>
      prev.includes(graphId) ? prev.filter((item) => item !== graphId) : [...prev, graphId]
    );
  };

  const onRunMerge = async () => {
    setRunningMerge(true);
    setMergeError(null);
    try {
      setMergeResult(await runMerge({ graphIds: selectedMergeGraphIds, apply: applyMerge }));
    } catch (error) {
      setMergeError(error instanceof Error ? error.message : 'Failed merge');
    } finally {
      setRunningMerge(false);
    }
  };

  const onGenerateWiki = async () => {
    if (!activeGraphId) return;
    setLoadingWiki(true);
    setWikiError(null);
    try {
      const generated = await generateWiki(activeGraphId);
      setWiki(generated);
      setActiveTab('wiki');
    } catch (error) {
      setWikiError(error instanceof Error ? error.message : 'Failed wiki generation');
    } finally {
      setLoadingWiki(false);
    }
  };

  const onSaveReview = async (
    runId: string,
    review: { query_id: string; relevance: number; fidelity: number; clarity: number; note: string }
  ) => {
    const answerQuality = await saveManualReview(runId, review);
    setRuns((current) =>
      current.map((run) => {
        if (run.run_id !== runId) {
          return run;
        }
        return {
          ...run,
          answer_quality: answerQuality,
        };
      })
    );
  };

  const refreshEnrichmentState = async (graphId: string) => {
    const [runsPayload, itemsPayload, receiptsPayload] = await Promise.all([
      getEnrichmentRuns(graphId),
      getEnrichmentItems(graphId),
      getEnrichmentReceipts(graphId),
    ]);
    setEnrichmentRuns(runsPayload);
    setEnrichmentItems(itemsPayload);
    setEnrichmentReceipts(receiptsPayload);
  };

  const onApproveEnrichment = async (graphId: string, enrichmentId: string) => {
    await approveEnrichment(graphId, enrichmentId);
    await refreshEnrichmentState(graphId);
    const [nodes, edges] = await Promise.all([getGraphNodes(graphId), getGraphEdges(graphId)]);
    setGraphData(mapGraphData(nodes, edges));
  };

  const onRejectEnrichment = async (graphId: string, enrichmentId: string) => {
    await rejectEnrichment(graphId, enrichmentId);
    await refreshEnrichmentState(graphId);
  };

  const onCommitStage = async (graphId: string) => {
    await commitEnrichmentQueue(graphId);
    await refreshEnrichmentState(graphId);
    const [nodes, edges, summary] = await Promise.all([
      getGraphNodes(graphId),
      getGraphEdges(graphId),
      getGraphSummary(graphId),
    ]);
    setGraphData(mapGraphData(nodes, edges));
    setGraphSummary(summary);
  };

  useEffect(() => {
    void loadCases();
    void loadRuns();
  }, []);

  useEffect(() => {
    if (!activeGraphId) {
      setGraphSummary(EMPTY_SUMMARY);
      setDecisions([]);
      setGraphData(EMPTY_GRAPH_DATA);
      setEnrichmentRuns([]);
      setEnrichmentItems([]);
      setEnrichmentReceipts([]);
      return;
    }

    const fetchGraphContext = async () => {
      setLoadingGraphSummary(true);
      setLoadingGraphData(true);
      setLoadingDecisions(true);
      setSummaryError(null);
      setDecisionError(null);
      setGraphDataError(null);
      try {
        setGraphSummary(await getGraphSummary(activeGraphId));
      } catch (error) {
        setSummaryError(error instanceof Error ? error.message : 'Failed graph summary');
      } finally {
        setLoadingGraphSummary(false);
      }

      try {
        const [nodes, edges] = await Promise.all([getGraphNodes(activeGraphId), getGraphEdges(activeGraphId)]);
        setGraphData(mapGraphData(nodes, edges));
      } catch (error) {
        setGraphDataError(error instanceof Error ? error.message : 'Failed graph map');
        setGraphData(EMPTY_GRAPH_DATA);
      } finally {
        setLoadingGraphData(false);
      }

      try {
        await refreshEnrichmentState(activeGraphId);
      } catch {
        setEnrichmentRuns([]);
        setEnrichmentItems([]);
        setEnrichmentReceipts([]);
      }

      if (!activeRunId) {
        setLoadingDecisions(false);
        setDecisions([]);
        return;
      }

      try {
        setDecisions(await getDecisions(activeRunId));
      } catch (error) {
        setDecisionError(error instanceof Error ? error.message : 'Failed decision trace');
      } finally {
        setLoadingDecisions(false);
      }
    };

    void fetchGraphContext();
  }, [activeGraphId, activeRunId]);

  useEffect(() => {
    if (!activeGraphId) {
      setWiki(null);
      return;
    }
    const loadWiki = async () => {
      try {
        setWiki(await getWiki(activeGraphId));
      } catch {
        // optional fetch; ignore to avoid disrupting workspace load
      }
    };
    void loadWiki();
  }, [activeGraphId]);

  return (
    <div className="app-shell app-shell-full">
      <aside
        className={`app-sidebar glass-nav-shell ${isSidebarCollapsed ? 'app-sidebar-collapsed' : ''}`.trim()}
        aria-label="Perf report workspaces"
        data-testid="workspace-sidebar"
        data-collapsed={isSidebarCollapsed ? 'true' : 'false'}
      >
        <div className="sidebar-brand-row">
          {!isSidebarCollapsed ? (
            <div className="sidebar-brand">
              <span className="sidebar-kicker">IKAM</span>
              <span className="sidebar-title">Perf</span>
              <h1 className="sidebar-heading">IKAM Performance Report</h1>
            </div>
          ) : null}
          <button
            type="button"
            className="sidebar-collapse-toggle"
            aria-label={isSidebarCollapsed ? 'Expand workspace nav' : 'Collapse workspace nav'}
            aria-expanded={!isSidebarCollapsed}
            aria-controls="workspace-sidebar-nav"
            onClick={() => setIsSidebarCollapsed((current) => !current)}
          >
            <span aria-hidden>{isSidebarCollapsed ? '»' : '«'}</span>
          </button>
        </div>
        <nav className="sidebar-nav" id="workspace-sidebar-nav">
          <button type="button" aria-label="Runs" title="Runs" aria-current={activeTab === 'runs' ? 'page' : undefined} className={activeTab === 'runs' ? 'tab-active' : ''} onClick={() => setActiveTab('runs')}>
            <span className="nav-icon" aria-hidden>▦</span>
            {!isSidebarCollapsed ? <span className="nav-label">Runs</span> : null}
          </button>
          <button type="button" aria-label="Graph" title="Graph" aria-current={activeTab === 'graph' ? 'page' : undefined} className={activeTab === 'graph' ? 'tab-active' : ''} onClick={() => setActiveTab('graph')}>
            <span className="nav-icon" aria-hidden>◎</span>
            {!isSidebarCollapsed ? <span className="nav-label">Graph</span> : null}
          </button>
          <button type="button" aria-label="Merge" title="Merge" aria-current={activeTab === 'merge' ? 'page' : undefined} className={activeTab === 'merge' ? 'tab-active' : ''} onClick={() => setActiveTab('merge')}>
            <span className="nav-icon" aria-hidden>⇄</span>
            {!isSidebarCollapsed ? <span className="nav-label">Merge</span> : null}
          </button>
          <button type="button" aria-label="History" title="History" aria-current={activeTab === 'history' ? 'page' : undefined} className={activeTab === 'history' ? 'tab-active' : ''} onClick={() => setActiveTab('history')}>
            <span className="nav-icon" aria-hidden>◷</span>
            {!isSidebarCollapsed ? <span className="nav-label">History</span> : null}
          </button>
          <button type="button" aria-label="Wiki" title="Wiki" aria-current={activeTab === 'wiki' ? 'page' : undefined} className={activeTab === 'wiki' ? 'tab-active' : ''} onClick={() => setActiveTab('wiki')}>
            <span className="nav-icon" aria-hidden>✎</span>
            {!isSidebarCollapsed ? <span className="nav-label">Wiki</span> : null}
          </button>

          <button type="button" aria-label="Registry" title="Registry" aria-current={activeTab === 'registry' ? 'page' : undefined} className={activeTab === 'registry' ? 'tab-active' : ''} onClick={() => setActiveTab('registry')}>
            <span className="nav-icon" aria-hidden>📋</span>
            {!isSidebarCollapsed ? <span className="nav-label">Registry</span> : null}
          </button>
        </nav>
        <div className="sidebar-meta" aria-hidden={isSidebarCollapsed}>
          <span>Profile: ikam-perf-report</span>
          <span>Source: {API_BASE_URL}</span>
        </div>
      </aside>

      <main className="app-main app-main-single">
        {activeTab === 'runs' ? (
          <RunsWorkspace
            cases={cases}
            loadingCases={loadingCases}
            caseError={caseError}
            selectedCaseIds={selectedCaseIds}
            onToggleCase={onToggleCase}
            reset={resetBeforeRun}
            onResetChange={setResetBeforeRun}
            running={runningCases}
            runError={runError}
            onRunCases={onRunCases}
            runs={runs}
            activeRunId={activeRunId}
            onSelectRun={setActiveRunId}
          />
        ) : null}

        {activeTab === 'graph' ? (
          <GraphWorkspace
            activeRun={activeRun}
            summary={graphSummary}
            loadingSummary={loadingGraphSummary}
            summaryError={summaryError}
            decisions={decisions}
            loadingDecisions={loadingDecisions}
            decisionError={decisionError}
            graphData={graphData}
            loadingGraphData={loadingGraphData}
            graphDataError={graphDataError}
            semanticEntities={semanticEntities}
            semanticRelations={semanticRelations}
            onSaveReview={onSaveReview}
            enrichmentRuns={enrichmentRuns}
            enrichmentItems={enrichmentItems}
            enrichmentReceipts={enrichmentReceipts}
            onApproveEnrichment={onApproveEnrichment}
            onRejectEnrichment={onRejectEnrichment}
            onCommitStage={onCommitStage}
          />
        ) : null}

        {activeTab === 'merge' ? (
          <MergeWorkspace
            runs={runs}
            selectedGraphIds={selectedMergeGraphIds}
            onToggleGraphId={onToggleMergeGraphId}
            apply={applyMerge}
            onApplyChange={setApplyMerge}
            onRunMerge={onRunMerge}
            loading={runningMerge}
            error={mergeError}
            result={mergeResult}
          />
        ) : null}

        {activeTab === 'history' ? (
          <HistoryWorkspace runId={activeRunId} />
        ) : null}

        {activeTab === 'wiki' ? (
          <WikiWorkspace
            activeGraphId={activeGraphId}
            wiki={wiki}
            loading={loadingWiki}
            error={wikiError}
            onGenerate={onGenerateWiki}
          />
        ) : null}

        {activeTab === 'registry' ? (
          <RegistryWorkspace />
        ) : null}
      </main>
    </div>
  );
};

export default App;
