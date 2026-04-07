import type { MergeResult, RunEntry } from '../api/client';

type MergeWorkspaceProps = {
  runs: RunEntry[];
  selectedGraphIds: string[];
  onToggleGraphId: (graphId: string) => void;
  apply: boolean;
  onApplyChange: (value: boolean) => void;
  onRunMerge: () => void;
  loading: boolean;
  error: string | null;
  result: MergeResult | null;
};

const MergeWorkspace = ({
  runs,
  selectedGraphIds,
  onToggleGraphId,
  apply,
  onApplyChange,
  onRunMerge,
  loading,
  error,
  result,
}: MergeWorkspaceProps) => {
  const graphIds = Array.from(new Set(runs.map((run) => run.graph_id ?? run.project_id)));
  return (
    <section className="panel panel-dark">
      <h2>Merge Workspace</h2>
      <p className="panel-subtitle">Source: /benchmarks/merge?graph_ids=...&apply=...</p>
      <div className="panel-actions">
        <label className="model-select">
          <input name="merge-apply" type="checkbox" checked={apply} onChange={(event) => onApplyChange(event.target.checked)} />
          <span>Apply updates</span>
        </label>
        <button type="button" disabled={loading || selectedGraphIds.length === 0} onClick={onRunMerge}>
          Run Merge
        </button>
      </div>
      {!graphIds.length ? (
        <div className="panel-placeholder">No graphs available from runs</div>
      ) : (
        <div className="case-selector" role="group" aria-label="Graph selector">
          {graphIds.map((graphId) => (
            <label key={graphId} className="case-option">
              <input
                name={`merge-graph-${graphId}`}
                type="checkbox"
                checked={selectedGraphIds.includes(graphId)}
                onChange={() => onToggleGraphId(graphId)}
                disabled={loading}
              />
              <span>{graphId}</span>
            </label>
          ))}
        </div>
      )}
      {error ? <div className="panel-placeholder">Error: {error}</div> : null}
      {!result ? (
        <div className="panel-placeholder">No merge run yet</div>
      ) : (
        <div className="panel-grid">
          <div className="metric">
            <span className="metric-label">Graphs</span>
            <span className="metric-value">{result.graph_ids.length}</span>
          </div>
          <div className="metric">
            <span className="metric-label">Proposed edges</span>
            <span className="metric-value">{result.proposed_edges.length}</span>
          </div>
          <div className="metric">
            <span className="metric-label">Proposed fragments</span>
            <span className="metric-value">{result.proposed_relational_fragments.length}</span>
          </div>
          <div className="metric">
            <span className="metric-label">Apply result</span>
            <span className="metric-value">
              {result.apply_result
                ? `${result.apply_result.edge_updates}/${result.apply_result.relational_fragment_updates}`
                : 'Not applied'}
            </span>
          </div>
        </div>
      )}
    </section>
  );
};

export default MergeWorkspace;
