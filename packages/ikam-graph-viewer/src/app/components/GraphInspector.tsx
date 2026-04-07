import { useMemo } from 'react';

import type { SelectionDetails } from '../../index';
import type { GraphSearchResult } from '../api/client';

type GraphInspectorProps = {
  selection: SelectionDetails;
  explanation?: GraphSearchResult['explanations'][number] | null;
  renderChain?: {
    artifactId: string;
    artifactLabel: string;
    nodeIds: string[];
    edgeIds: string[];
    steps: Array<{ from: string; to: string; kind: string }>;
  } | null;
  onRenderArtifact?: () => void;
  onDownloadArtifact?: () => void;
};

const formatJson = (value: unknown) => JSON.stringify(value, null, 2);

const GraphInspector = ({ selection, explanation, renderChain, onRenderArtifact, onDownloadArtifact }: GraphInspectorProps) => {
  const hasSelection = selection.selectedNode || selection.selectedEdge || selection.selectedNodeIds.length > 0;
  const reasons = explanation?.reasons ?? {};
  const explanationPayload = useMemo(() => {
    if (!explanation) return '';
    return JSON.stringify(explanation, null, 2);
  }, [explanation]);
  return (
    <section className="panel panel-dark panel-inline" data-testid="graph-inspector">
      <h2>Inspector</h2>
      {!hasSelection ? (
        <div className="panel-placeholder">Select a node or edge to inspect meaning and provenance</div>
      ) : (
        <div className="inspector-grid">
          {selection.selectedNode ? (
            <>
              <div className="inspector-section">
                <div className="inspector-section-title">Selection</div>
                <div className="inspector-section-grid">
                  <div className="metric">
                    <span className="metric-label">Selected Node</span>
                    <span className="metric-value">{selection.selectedNode.label}</span>
                  </div>
                  <div className="metric">
                    <span className="metric-label">Node Type</span>
                    <span className="metric-value">{selection.selectedNode.type}</span>
                  </div>
                  <div className="metric">
                    <span className="metric-label">Origin</span>
                    <span className="metric-value">{selection.selectedNode.provenance.origin}</span>
                  </div>
                </div>
                {selection.selectedNode.type === 'artifact' ? (
                  <div className="inspector-actions">
                    <button
                      type="button"
                      className="inspector-render-button"
                      onClick={() => {
                        onRenderArtifact?.();
                      }}
                    >
                      Render Artifact
                    </button>
                    <button
                      type="button"
                      className="inspector-download-button"
                      onClick={() => {
                        onDownloadArtifact?.();
                      }}
                    >
                      Download Artifact
                    </button>
                  </div>
                ) : null}
              </div>
              {renderChain ? (
                <div className="inspector-section" data-testid="render-chain">
                  <div className="inspector-section-title">Render Chain</div>
                  <div className="render-chain-summary">{renderChain.artifactLabel}</div>
                  <div className="render-chain-list">
                    {renderChain.steps.map((step, index) => (
                      <div className="render-chain-item" key={`${step.from}-${step.to}-${index}`}>
                        <span>{step.from}</span>
                        <span className="render-chain-arrow">-&gt;</span>
                        <span>{step.to}</span>
                        <span className="render-chain-kind">{step.kind}</span>
                      </div>
                    ))}
                  </div>
                </div>
              ) : null}
              {explanation ? (
                <div className="inspector-explanation">
                  <div className="inspector-explanation-header">
                    <div>
                      <div className="inspector-explanation-title">Why Chosen</div>
                      <p className="inspector-explanation-summary">{explanation.summary}</p>
                    </div>
                    <button
                      type="button"
                      className="inspector-copy"
                      onClick={() => {
                        if (!explanationPayload) return;
                        navigator.clipboard?.writeText(explanationPayload);
                      }}
                    >
                      Copy Evidence
                    </button>
                  </div>
                  <div className="inspector-explanation-grid">
                    <div>
                      <div className="inspector-explanation-label">Tokens</div>
                      <div className="inspector-explanation-tags">
                        {(reasons.text_match_tokens ?? []).length ? (
                          reasons.text_match_tokens?.map((token) => (
                            <span key={token} className="inspector-tag">
                              {token}
                            </span>
                          ))
                        ) : (
                          <span className="inspector-tag inspector-tag-muted">None</span>
                        )}
                      </div>
                    </div>
                    <div>
                      <div className="inspector-explanation-label">Relations</div>
                      <div className="inspector-explanation-tags">
                        {(reasons.relation_matches ?? []).length ? (
                          reasons.relation_matches?.map((rel) => (
                            <span key={rel} className="inspector-tag">
                              {rel}
                            </span>
                          ))
                        ) : (
                          <span className="inspector-tag inspector-tag-muted">None</span>
                        )}
                      </div>
                    </div>
                    <div>
                      <div className="inspector-explanation-label">Graph Degree</div>
                      <div className="inspector-explanation-value">{reasons.graph_degree ?? '--'}</div>
                    </div>
                    <div>
                      <div className="inspector-explanation-label">Weights</div>
                      <div className="inspector-explanation-value">
                        {(reasons.weights &&
                          `T ${reasons.weights.text} · R ${reasons.weights.relation} · G ${reasons.weights.graph}`) ||
                          '--'}
                      </div>
                    </div>
                  </div>
                </div>
              ) : null}
              <div className="inspector-section">
                <div className="inspector-section-title">Raw Metadata</div>
                <pre
                  className={`inspector-json${explanation ? ' inspector-json-compact' : ''}`}
                  data-testid="inspector-json"
                >
                  {formatJson(selection.selectedNode.meta)}
                </pre>
              </div>
            </>
          ) : null}

          {selection.selectedEdge ? (
            <>
              <div className="metric">
                <span className="metric-label">Selected Edge</span>
                <span className="metric-value">{selection.selectedEdge.id ?? `${selection.selectedEdge.source} -> ${selection.selectedEdge.target}`}</span>
              </div>
              <div className="metric">
                <span className="metric-label">Edge Kind</span>
                <span className="metric-value">{selection.selectedEdge.kind}</span>
              </div>
              <div className="metric">
                <span className="metric-label">Origin</span>
                <span className="metric-value">{selection.selectedEdge.provenance.origin}</span>
              </div>
              <div className="inspector-section">
                <div className="inspector-section-title">Relation Evidence</div>
                <div className="inspector-section-grid">
                  <div className="metric">
                    <span className="metric-label">Relation ID</span>
                    <span className="metric-value">{String((selection.selectedEdge.meta as any)?.relation_id ?? '--')}</span>
                  </div>
                  <div className="metric">
                    <span className="metric-label">Status</span>
                    <span className="metric-value">{String((selection.selectedEdge.meta as any)?.relation_status ?? '--')}</span>
                  </div>
                  <div className="metric">
                    <span className="metric-label">Evidence</span>
                    <span className="metric-value">
                      {Array.isArray((selection.selectedEdge.meta as any)?.evidence)
                        ? (selection.selectedEdge.meta as any).evidence.join(', ')
                        : '--'}
                    </span>
                  </div>
                  <div className="metric">
                    <span className="metric-label">Rationale</span>
                    <span className="metric-value">{String((selection.selectedEdge.meta as any)?.rationale ?? '--')}</span>
                  </div>
                </div>
              </div>
              <pre className="inspector-json">{formatJson(selection.selectedEdge.meta)}</pre>
            </>
          ) : null}

          {selection.selectedNodeIds.length > 0 ? (
            <div className="metric">
              <span className="metric-label">Multi Select</span>
              <span className="metric-value">{selection.selectedNodeIds.length} nodes</span>
            </div>
          ) : null}
        </div>
      )}
    </section>
  );
};

export default GraphInspector;
