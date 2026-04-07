type Decision = {
  step_index: number;
  decision_type: string;
  created_at: string;
};

type DecisionTraceProps = {
  runId: string | null;
  graphId: string | null;
  decisions: Decision[];
  loading: boolean;
  error: string | null;
};

const DecisionTrace = ({ runId, graphId, decisions, loading, error }: DecisionTraceProps) => {
  return (
    <section className="panel panel-dark panel-inline">
      <h2>Decision Trace</h2>
      <div className="panel-grid">
        <div className="metric">
          <span className="metric-label">Run</span>
          <span className="metric-value">{runId ?? '--'}</span>
        </div>
        <div className="metric">
          <span className="metric-label">Graph</span>
          <span className="metric-value">{graphId ?? '--'}</span>
        </div>
        <div className="metric">
          <span className="metric-label">Decisions</span>
          <span className="metric-value">{decisions.length}</span>
        </div>
        <div className="metric">
          <span className="metric-label">AI Only</span>
          <span className="metric-value">Off</span>
        </div>
      </div>
      {error ? (
        <div className="panel-placeholder">Error: {error}</div>
      ) : loading ? (
        <div className="panel-placeholder">Loading decisions…</div>
      ) : decisions.length === 0 ? (
        <div className="panel-placeholder">Decision stream</div>
      ) : (
        <div className="decision-list">
          {decisions.map((decision) => (
            <div key={`${decision.step_index}-${decision.created_at}`} className="decision-row">
              <span>{decision.decision_type}</span>
              <span className="decision-meta">{decision.created_at}</span>
            </div>
          ))}
        </div>
      )}
    </section>
  );
};

export default DecisionTrace;
