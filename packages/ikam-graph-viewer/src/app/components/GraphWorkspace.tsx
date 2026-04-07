import { useEffect, useMemo, useRef, useState } from 'react';

import { createIKAMGraph, deriveSelectionDetails, type GraphData, type GraphHandle, type SelectionDetails } from '../../index';
import { semanticLegendEntries } from '../../theme';
import type {
  Decision,
  EnrichmentItem,
  EnrichmentReceipt,
  EnrichmentRun,
  GraphSearchResult,
  GraphSummary,
  RunEntry,
  SemanticEntity,
  SemanticRelation,
} from '../api/client';
import { API_BASE_URL, downloadArtifact } from '../api/client';
import DecisionTrace from './DecisionTrace';
import GraphInspector from './GraphInspector';
import GraphLegend from './GraphLegend';
import ReportKpiStrip from './ReportKpiStrip';
import ReviewPanel from './ReviewPanel';
import SemanticExplorer from './SemanticExplorer';

type GraphWorkspaceProps = {
  activeRun: RunEntry | null;
  summary: GraphSummary;
  loadingSummary: boolean;
  summaryError: string | null;
  decisions: Decision[];
  loadingDecisions: boolean;
  decisionError: string | null;
  graphData: GraphData;
  loadingGraphData: boolean;
  graphDataError: string | null;
  semanticEntities: SemanticEntity[];
  semanticRelations: SemanticRelation[];
  onSaveReview: (runId: string, review: { query_id: string; relevance: number; fidelity: number; clarity: number; note: string }) => Promise<void>;
  enrichmentRuns: EnrichmentRun[];
  enrichmentItems: EnrichmentItem[];
  enrichmentReceipts: EnrichmentReceipt[];
  onApproveEnrichment: (graphId: string, enrichmentId: string) => Promise<void>;
  onRejectEnrichment: (graphId: string, enrichmentId: string) => Promise<void>;
  onCommitStage: (graphId: string) => Promise<void>;
};

const GraphWorkspace = ({
  activeRun,
  summary,
  loadingSummary,
  summaryError,
  decisions,
  loadingDecisions,
  decisionError,
  graphData,
  loadingGraphData,
  graphDataError,
  semanticEntities,
  semanticRelations,
  onSaveReview,
  enrichmentRuns,
  enrichmentItems,
  enrichmentReceipts,
  onApproveEnrichment,
  onRejectEnrichment,
  onCommitStage,
}: GraphWorkspaceProps) => {
  const quickLegendEntries = semanticLegendEntries.slice(0, 4);
  const graphViewportRef = useRef<HTMLDivElement | null>(null);
  const graphHandleRef = useRef<GraphHandle | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | undefined>(undefined);
  const [selectedEdgeId, setSelectedEdgeId] = useState<string | undefined>(undefined);
  const [selectedNodeIds, setSelectedNodeIds] = useState<string[]>([]);
  const [viewport, setViewport] = useState<{ width: number; height: number; cameraDistance?: number } | null>(null);
  const [hoveredNodeId, setHoveredNodeId] = useState<string | null>(null);
  const [hoveredEdgeId, setHoveredEdgeId] = useState<string | null>(null);
  const [hoverPosition, setHoverPosition] = useState<{ x: number; y: number } | null>(null);
  const [hoverVisible, setHoverVisible] = useState(false);
  const [groupQuery, setGroupQuery] = useState('');
  const [submittedQuery, setSubmittedQuery] = useState('');
  const [activeGroupId, setActiveGroupId] = useState<string | null>(null);
  const [discoveringGroups, setDiscoveringGroups] = useState(false);
  const [parseSummary, setParseSummary] = useState<string | null>(null);
  const [searchResult, setSearchResult] = useState<GraphSearchResult | null>(null);
  const hoverIntentRef = useRef(false);
  const mapShellRef = useRef<HTMLDivElement | null>(null);
  const lastPointerRef = useRef<{ x: number; y: number } | null>(null);

  const selectionDetails: SelectionDetails = useMemo(
    () => deriveSelectionDetails(graphData, { selectedNodeId, selectedEdgeId, selectedNodeIds }),
    [graphData, selectedNodeId, selectedEdgeId, selectedNodeIds]
  );

  const hoverDetails: SelectionDetails = useMemo(
    () => deriveSelectionDetails(graphData, {
      selectedNodeId: hoveredNodeId ?? undefined,
      selectedEdgeId: hoveredEdgeId ?? undefined,
      selectedNodeIds: [],
    }),
    [graphData, hoveredNodeId, hoveredEdgeId]
  );

  useEffect(() => {
    if (!activeRun || !graphViewportRef.current || graphData.nodes.length === 0) {
      return;
    }
    graphHandleRef.current?.destroy();
    graphHandleRef.current = createIKAMGraph(graphViewportRef.current, graphData, {
      background: '#edf3fb',
      edgeOpacity: 0.9,
      nodeSize: 12,
      orbitControls: true,
      showGroups: false,
      activeGroupId: null,
      onPointerMove: (coords) => {
        lastPointerRef.current = coords;
      },
      onNodeClick: (node) => {
        setSelectedNodeId(node.id);
        setSelectedEdgeId(undefined);
        clearHover();
      },
      onNodeHover: (node) => {
        if (node) {
          hoverIntentRef.current = true;
          if (mapShellRef.current && lastPointerRef.current) {
            const rect = mapShellRef.current.getBoundingClientRect();
            setHoverPosition({
              x: Math.min(Math.max(lastPointerRef.current.x, 8), rect.width - 8),
              y: Math.min(Math.max(lastPointerRef.current.y, 8), rect.height - 8),
            });
          }
          setHoveredNodeId(node.id);
          setHoveredEdgeId(null);
          setHoverVisible(true);
        } else {
          hoverIntentRef.current = false;
        }
      },
      onEdgeHover: (edge) => {
        if (edge) {
          hoverIntentRef.current = true;
          if (mapShellRef.current && lastPointerRef.current) {
            const rect = mapShellRef.current.getBoundingClientRect();
            setHoverPosition({
              x: Math.min(Math.max(lastPointerRef.current.x, 8), rect.width - 8),
              y: Math.min(Math.max(lastPointerRef.current.y, 8), rect.height - 8),
            });
          }
          const fallbackId = `${edge.source}::${edge.target}::${edge.kind ?? 'edge'}`;
          setHoveredEdgeId(edge.id ?? fallbackId);
          setHoveredNodeId(null);
          setHoverVisible(true);
        } else {
          hoverIntentRef.current = false;
        }
      },
      onEdgeClick: (edge) => {
        setSelectedEdgeId(edge.id);
        setSelectedNodeId(undefined);
        clearHover();
      },
      onSelectionChange: (selection) => {
        setSelectedNodeId(selection.selectedNodeId);
        setSelectedEdgeId(selection.selectedEdgeId);
        setSelectedNodeIds(selection.selectedNodeIds);
      },
      onViewportChange: (nextViewport) => {
        setViewport(nextViewport);
      },
    });

    return () => {
      graphHandleRef.current?.destroy();
      graphHandleRef.current = null;
    };
  }, [activeRun, graphData]);

  useEffect(() => {
    graphHandleRef.current?.setOptions({ activeGroupId });
  }, [activeGroupId]);

  const clearHover = () => {
    hoverIntentRef.current = false;
    setHoverVisible(false);
    setHoveredNodeId(null);
    setHoveredEdgeId(null);
  };

  const graphId = activeRun ? activeRun.graph_id ?? activeRun.project_id : null;
  const hoverActive =
    hoverVisible &&
    (hoverDetails.selectedNode || hoverDetails.selectedEdge) &&
    !selectionDetails.selectedNode &&
    !selectionDetails.selectedEdge;

  const cachedGroupSuggestions = useMemo(() => {
    const nodes = graphData.nodes ?? [];
    const suggestions: Array<{ id: string; label: string; count: number }> = [];
    const counts = new Map<string, { label: string; count: number }>();
    for (const node of nodes) {
      const meta = (node as any)?.meta ?? {};
      const groupId = typeof meta.semantic_entity_id === 'string' ? meta.semantic_entity_id : 'ungrouped';
      const label = typeof meta.semantic_entity_label === 'string' ? meta.semantic_entity_label : groupId;
      const entry = counts.get(groupId) ?? { label, count: 0 };
      entry.count += 1;
      counts.set(groupId, entry);
    }
    for (const [id, info] of counts.entries()) {
      const label = info.label || id;
      suggestions.push({ id, label, count: info.count });
    }
    return suggestions.sort((a, b) => b.count - a.count).slice(0, 8);
  }, [graphData]);

  const searchMarkers = useMemo(() => {
    if (!searchResult?.results?.length) return [];
    const nodeLabels = new Map(graphData.nodes.map((node) => [node.id, node.label ?? node.id]));
    const explanations = new Map(searchResult.explanations?.map((item) => [item.node_id, item]) ?? []);
    return searchResult.results.slice(0, 10).map((result, index) => ({
      id: result.node_id,
      label: nodeLabels.get(result.node_id) ?? result.node_id,
      index,
      summary: explanations.get(result.node_id)?.summary ?? '',
    }));
  }, [graphData.nodes, searchResult]);

  const activeExplanation = useMemo(() => {
    if (!searchResult?.explanations?.length || !selectionDetails.selectedNode) {
      return null;
    }
    return searchResult.explanations.find((item) => item.node_id === selectionDetails.selectedNode?.id) ?? null;
  }, [searchResult, selectionDetails.selectedNode]);

  const renderChain = useMemo(() => {
    const selectedNode = selectionDetails.selectedNode;
    if (!selectedNode || selectedNode.type !== 'artifact') return null;

    const labelByNodeId = new Map(graphData.nodes.map((node) => [node.id, node.label ?? node.id]));
    const chainEdges = graphData.edges.filter(
      (edge) => edge.source === selectedNode.id && (edge.kind === 'artifact-root' || edge.kind === 'composition')
    );
    const nodeIds = Array.from(new Set([selectedNode.id, ...chainEdges.map((edge) => edge.target)]));

    return {
      artifactId: selectedNode.id,
      artifactLabel: selectedNode.label,
      nodeIds,
      edgeIds: chainEdges.map((edge) => edge.id ?? `${edge.source}->${edge.target}:${edge.kind ?? 'edge'}`),
      steps: chainEdges.map((edge) => ({
        from: labelByNodeId.get(edge.source) ?? edge.source,
        to: labelByNodeId.get(edge.target) ?? edge.target,
        kind: edge.kind ?? 'edge',
      })),
    };
  }, [graphData.edges, graphData.nodes, selectionDetails.selectedNode]);

  const highlightRenderChain = () => {
    if (!renderChain) return;
    const highlightedNodes = new Set(renderChain.nodeIds);
    const highlightedEdges = new Set(renderChain.edgeIds);
    const nextGraph = {
      ...graphData,
      nodes: graphData.nodes.map((node) => ({
        ...node,
        meta: {
          ...(node as any).meta,
          highlighted: highlightedNodes.has(node.id),
          dimmed: !highlightedNodes.has(node.id),
        },
      })),
      edges: graphData.edges.map((edge) => {
        const edgeId = edge.id ?? `${edge.source}->${edge.target}:${edge.kind ?? 'edge'}`;
        return {
          ...edge,
          meta: {
            ...(edge as any).meta,
            highlighted: highlightedEdges.has(edgeId),
            dimmed: !highlightedEdges.has(edgeId),
            pulse: highlightedEdges.has(edgeId),
          },
        };
      }),
    };
    graphHandleRef.current?.update(nextGraph);
    graphHandleRef.current?.fitToNodes(renderChain.nodeIds);
    graphHandleRef.current?.focusNode(renderChain.artifactId);
  };

  const handleDownloadArtifact = () => {
    const selectedNode = selectionDetails.selectedNode;
    if (!selectedNode || selectedNode.type !== 'artifact') return;
    downloadArtifact(selectedNode.id);
  };

  useEffect(() => {
    if (!renderChain) return;
    highlightRenderChain();
  }, [renderChain]);

  const [groupSuggestions, setGroupSuggestions] = useState(cachedGroupSuggestions);

  useEffect(() => {
    if (!submittedQuery.trim()) {
      setDiscoveringGroups(false);
      setGroupSuggestions(cachedGroupSuggestions);
      setParseSummary(null);
      setSearchResult(null);
    }
  }, [cachedGroupSuggestions, submittedQuery]);

  useEffect(() => {
    if (!submittedQuery.trim()) {
      return;
    }
    const controller = new AbortController();
    const query = submittedQuery.trim();
    const normalized = query.toLowerCase();
    const inferred = normalized.includes('derive') || normalized.includes('provenance') ? 'Derivation intent detected' : 'Semantic intent detected';
    const payload = {
      query,
      graph_id: graphId,
      scope_id: activeRun?.run_id,
      intent: 'search',
    };
    setParseSummary(inferred);
    setDiscoveringGroups(true);

    fetch(`${API_BASE_URL}/graph/search`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
      signal: controller.signal,
    })
      .then(async (response) => {
        if (!response.ok) {
          throw new Error(`Graph search failed (${response.status})`);
        }
        return (await response.json()) as GraphSearchResult;
      })
      .then((body) => {
        setSearchResult(body);
        const next = (body.groups ?? []).map((group) => ({
          id: group.id,
          label: group.label || group.id,
          count: group.size ?? 0,
        }));
        setGroupSuggestions(next);
        const firstResult = body.results?.[0]?.node_id;
        if (firstResult) {
          const resultNodeIds = body.results.map((result) => result.node_id);
          const highlighted = new Set(body.results.map((result) => result.node_id));
          const nextGraph = {
            ...graphData,
            nodes: graphData.nodes.map((node) => ({
              ...node,
              meta: {
                ...(node as any).meta,
                highlighted: highlighted.has(node.id),
                dimmed: !highlighted.has(node.id),
              },
            })),
          };
          graphHandleRef.current?.update(nextGraph);
          graphHandleRef.current?.fitToNodes(resultNodeIds);
          graphHandleRef.current?.focusNode(firstResult);
        }
      })
      .catch((error) => {
        if (error?.name === 'AbortError') {
          return;
        }
        setGroupSuggestions([]);
      })
      .finally(() => {
        setDiscoveringGroups(false);
      });

    return () => {
      controller.abort();
    };
  }, [activeRun?.run_id, graphId, submittedQuery]);
  return (
    <section className="panel panel-muted graph-fullscreen">
      {!activeRun ? (
        <div className="panel-placeholder">Select a run in Runs workspace to inspect graph context</div>
      ) : (
        <>
          {!loadingGraphData && graphData.nodes.length === 0 ? (
            <div className="panel-placeholder">No graph map data available for this run</div>
          ) : null}
          <div className="graph-surface">
            <div className="graph-map-shell">
                <div
                  className="graph-map-area graph-map-area-soft-glass"
                  ref={mapShellRef}
                  data-testid="graph-map-shell"
                onClick={(event) => {
                  if (event.target === event.currentTarget) {
                    clearHover();
                  }
                }}
              >
                <div
                  className="graph-map"
                  ref={graphViewportRef}
                  aria-label="IKAM graph map"
                  data-testid="graph-map"
                  onClick={clearHover}
                />
                {searchMarkers.length ? (
                  <div className="graph-markers" aria-label="Search highlights">
                    {searchMarkers.map((marker) => (
                      <button
                        key={marker.id}
                        className="graph-marker"
                        data-testid="graph-marker"
                        type="button"
                        aria-label={marker.label}
                        title={marker.summary}
                        style={{ top: 24 + marker.index * 26, right: 24 }}
                        onClick={() => {
                          setSelectedNodeId(marker.id);
                          setSelectedEdgeId(undefined);
                          setSelectedNodeIds([]);
                          graphHandleRef.current?.focusNode(marker.id);
                        }}
                      >
                        <span className="graph-marker-dot" />
                        <span className="graph-marker-label">{marker.label}</span>
                      </button>
                    ))}
                  </div>
                ) : null}
                <div className="graph-overlay glass-overlay-region">
                  <ReportKpiStrip answerQuality={activeRun?.answer_quality} summary={summary} />
                  <div className="graph-topbar glass-panel">
                    <div className="graph-topbar-title">Graph Workspace</div>
                    <form
                      className="graph-search"
                      onSubmit={(event) => {
                        event.preventDefault();
                        const next = groupQuery.trim();
                        if (!next) {
                          setSubmittedQuery('');
                          setActiveGroupId(null);
                          graphHandleRef.current?.setOptions({ showGroups: false, activeGroupId: null });
                          return;
                        }
                        setSubmittedQuery(next);
                      }}
                    >
                      <input
                        type="search"
                        placeholder="Search semantic group"
                        value={groupQuery}
                        onChange={(event) => {
                          setGroupQuery(event.target.value);
                        }}
                      />
                      {groupQuery.trim() ? (
                        <div className="graph-search-results" data-testid="graph-group-search">
                          {submittedQuery !== groupQuery.trim() ? (
                            <div className="graph-search-status" aria-live="polite">
                              Press Enter to search
                            </div>
                          ) : null}
                          {discoveringGroups ? (
                            <div className="graph-search-status" aria-live="polite">
                              Parsing… Discovering…
                            </div>
                          ) : null}
                          {parseSummary ? <div className="graph-search-status">{parseSummary}</div> : null}
                          {submittedQuery && groupSuggestions.length === 0 ? (
                            <button type="button" className="graph-search-empty">
                              No matches
                            </button>
                          ) : null}
                          {submittedQuery
                            ? groupSuggestions.map((suggestion) => (
                                <button
                                  key={suggestion.id}
                                  type="button"
                                  onClick={() => {
                                    const groupId = `semantic-entity:${suggestion.id}`;
                                    setActiveGroupId(groupId);
                                    graphHandleRef.current?.setOptions({ showGroups: true, activeGroupId: groupId });
                                    graphHandleRef.current?.fitToGroup(groupId);
                                    setGroupQuery(`${suggestion.label}`);
                                  }}
                                >
                                  <span>{suggestion.label}</span>
                                  <span className="graph-search-count">{suggestion.count}</span>
                                </button>
                              ))
                            : null}
                        </div>
                      ) : null}
                    </form>
                    <div className="graph-topbar-actions">
                      <button type="button" onClick={() => graphHandleRef.current?.fitToData()}>
                        Fit Graph
                      </button>
                    </div>
                  </div>
                  <div className="graph-quick-legend glass-panel" data-testid="graph-quick-legend">
                    {quickLegendEntries.map((entry) => (
                      <div key={entry.kind} className="graph-quick-legend-item" title={entry.description}>
                        <span className="graph-quick-legend-dot" style={{ backgroundColor: entry.color }} aria-hidden />
                        <span>{entry.label}</span>
                      </div>
                    ))}
                  </div>
                  <div className="graph-status">
                    {summaryError ? <div className="panel-placeholder">Error: {summaryError}</div> : null}
                    {loadingSummary ? <div className="panel-placeholder">Loading graph summary...</div> : null}
                    {graphDataError ? <div className="panel-placeholder">Error: {graphDataError}</div> : null}
                    {loadingGraphData ? <div className="panel-placeholder">Loading graph map...</div> : null}
                  </div>
                  <div className="graph-context-panels" data-testid="graph-context-panels">
                    <GraphLegend graphData={graphData} />
                    <SemanticExplorer entities={semanticEntities} relations={semanticRelations} />
                  </div>
                </div>
                {selectionDetails.selectedNode || selectionDetails.selectedEdge ? (
                    <aside className="graph-drawer glass-panel" data-testid="graph-drawer">
                  <GraphInspector
                    selection={selectionDetails}
                    explanation={activeExplanation}
                    renderChain={renderChain}
                    onRenderArtifact={highlightRenderChain}
                    onDownloadArtifact={handleDownloadArtifact}
                  />
                  </aside>
                ) : null}
                {activeRun && graphId ? (
                  <ReviewPanel
                    runId={activeRun.run_id}
                    graphId={graphId}
                    answerQuality={activeRun.answer_quality}
                    onSaveReview={onSaveReview}
                    enrichmentRuns={enrichmentRuns}
                    enrichmentItems={enrichmentItems}
                    enrichmentReceipts={enrichmentReceipts}
                    onApproveEnrichment={onApproveEnrichment}
                    onRejectEnrichment={onRejectEnrichment}
                    onCommitStage={onCommitStage}
                  />
                ) : null}
                {hoverActive && hoverPosition ? (
                  <div
                    className="graph-hover"
                    data-testid="hover-preview"
                    style={{ left: hoverPosition.x + 16, top: hoverPosition.y + 16 }}
                  >
                    {hoverDetails.selectedNode ? (
                      <>
                        <div className="hover-title">{hoverDetails.selectedNode.label}</div>
                        <div className="hover-row">Type · {hoverDetails.selectedNode.type}</div>
                        <div className="hover-row">Origin · {hoverDetails.selectedNode.provenance.origin}</div>
                        <div className="hover-meta">
                          Entities: {hoverDetails.selectedNode.semanticLinks.entityIds.join(', ') || '--'}
                        </div>
                        <div className="hover-meta">
                          Relations: {hoverDetails.selectedNode.semanticLinks.relationIds.join(', ') || '--'}
                        </div>
                      </>
                    ) : null}
                    {hoverDetails.selectedEdge ? (
                      <>
                        <div className="hover-title">{hoverDetails.selectedEdge.kind}</div>
                        <div className="hover-row">Origin · {hoverDetails.selectedEdge.provenance.origin}</div>
                        <div className="hover-meta">
                          Entities: {hoverDetails.selectedEdge.semanticLinks.entityIds.join(', ') || '--'}
                        </div>
                        <div className="hover-meta">
                          Relations: {hoverDetails.selectedEdge.semanticLinks.relationIds.join(', ') || '--'}
                        </div>
                        <div className="hover-meta">
                          Edge · {hoverDetails.selectedEdge.source} → {hoverDetails.selectedEdge.target}
                        </div>
                      </>
                    ) : null}
                  </div>
                ) : null}
              </div>
            </div>
          </div>
        </>
      )}
    </section>
  );
};

export default GraphWorkspace;
