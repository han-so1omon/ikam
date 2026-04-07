import type { SemanticEntity, SemanticRelation } from '../api/client';

type SemanticExplorerProps = {
  entities: SemanticEntity[];
  relations: SemanticRelation[];
};

const SemanticExplorer = ({ entities, relations }: SemanticExplorerProps) => {
  return (
    <section className="panel panel-dark panel-inline" data-testid="semantic-explorer">
      <h2>Semantic Explorer</h2>
      <div className="panel-grid">
        <div className="metric">
          <span className="metric-label">Entities</span>
          <span className="metric-value">{entities.length}</span>
        </div>
        <div className="metric">
          <span className="metric-label">Relations</span>
          <span className="metric-value">{relations.length}</span>
        </div>
      </div>
      {!entities.length && !relations.length ? (
        <div className="panel-placeholder">No semantic entities or relations for this run</div>
      ) : (
        <div className="semantic-columns">
          <div>
            <p className="panel-subtitle">Entities</p>
            {entities.slice(0, 20).map((entity) => (
              <div key={entity.id} className="legend-row">
                <span>{entity.label ?? entity.id}</span>
                <span className="decision-meta">{entity.kind ?? 'entity'}</span>
              </div>
            ))}
          </div>
          <div>
            <p className="panel-subtitle">Relations</p>
            {relations.slice(0, 20).map((relation) => (
              <div key={relation.id} className="legend-row">
                <span>{relation.id}</span>
                <span className="decision-meta">{relation.kind ?? 'relation'}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </section>
  );
};

export default SemanticExplorer;
