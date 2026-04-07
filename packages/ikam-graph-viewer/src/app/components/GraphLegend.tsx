import type { GraphData } from '../../types';
import { buildGraphStats } from '../../index';
import { semanticLegendEntries, stateLegendEntries } from '../../theme';

type GraphLegendProps = {
  graphData: GraphData;
};

const GraphLegend = ({ graphData }: GraphLegendProps) => {
  const stats = buildGraphStats(graphData);
  const selfEdgeCount = graphData.edges.filter((edge) => edge.source === edge.target).length;
  const allEdgesAreSelfLoops = graphData.edges.length > 0 && selfEdgeCount === graphData.edges.length;
  return (
    <section className="panel panel-dark panel-inline" data-testid="graph-legend">
      <h2>Graph Legend</h2>
      <div className="panel-grid">
        <div className="metric">
          <span className="metric-label">Node Kinds</span>
          <span className="metric-value">{stats.nodeKinds.length}</span>
        </div>
        <div className="metric">
          <span className="metric-label">Edge Kinds</span>
          <span className="metric-value">{stats.edgeKinds.length}</span>
        </div>
        <div className="metric">
          <span className="metric-label">Self-Loop Edges</span>
          <span className="metric-value">{selfEdgeCount}</span>
        </div>
      </div>
      {allEdgesAreSelfLoops ? (
        <p className="legend-note" data-testid="graph-self-loop-note">
          This graph currently uses self-loop edges; loop rings around nodes indicate edge presence.
        </p>
      ) : null}
      <div className="legend-list">
        {stats.nodeKinds.map((kind) => (
          <div key={`node-${kind.kind}`} className="legend-row">
            <span>{kind.kind}</span>
            <span className="decision-meta">{kind.count} nodes</span>
          </div>
        ))}
        {stats.edgeKinds.map((kind) => (
          <div key={`edge-${kind.kind}`} className="legend-row">
            <span>{kind.kind}</span>
            <span className="decision-meta">{kind.count} edges</span>
          </div>
        ))}
      </div>

      <h3 className="legend-section-title">Color Key</h3>
      <div className="legend-list" data-testid="graph-color-key">
        {semanticLegendEntries.map((entry) => (
          <div key={`semantic-${entry.kind}`} className="legend-row legend-color-row">
            <span className="legend-main">
              <span className="legend-swatch" style={{ backgroundColor: entry.color }} aria-hidden />
              <span className="legend-main-text">{entry.label}</span>
            </span>
            <span className="decision-meta">{entry.description}</span>
          </div>
        ))}
      </div>

      <h3 className="legend-section-title">State Key</h3>
      <div className="legend-list" data-testid="graph-state-key">
        {stateLegendEntries.map((entry) => (
          <div key={`state-${entry.key}`} className="legend-row legend-color-row">
            <span className="legend-main">
              <span className="legend-swatch" style={{ backgroundColor: entry.color }} aria-hidden />
              <span className="legend-main-text">{entry.label}</span>
            </span>
            <span className="decision-meta">{entry.description}</span>
          </div>
        ))}
      </div>
    </section>
  );
};

export default GraphLegend;
