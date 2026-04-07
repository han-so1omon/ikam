import type { WikiDocument } from '../api/client';

type WikiWorkspaceProps = {
  activeGraphId: string | null;
  wiki: WikiDocument | null;
  loading: boolean;
  error: string | null;
  onGenerate: () => void;
};

const WikiWorkspace = ({ activeGraphId, wiki, loading, error, onGenerate }: WikiWorkspaceProps) => {
  return (
    <section className="panel panel-muted" data-testid="wiki-workspace">
      <h2>Wiki Workspace</h2>
      <p className="panel-subtitle">Source: /graph/wiki + /graph/wiki/generate</p>
      <div className="panel-actions">
        <button type="button" disabled={!activeGraphId || loading} onClick={onGenerate}>
          Generate Wiki
        </button>
        <span className="decision-meta">Graph: {activeGraphId ?? '--'}</span>
      </div>
      {error ? <div className="panel-placeholder">Error: {error}</div> : null}
      {loading ? <div className="panel-placeholder">Generating wiki...</div> : null}
      {!loading && !wiki ? <div className="panel-placeholder">No wiki generated yet</div> : null}
      {wiki ? (
        <div className="wiki-sections">
          {wiki.sections.map((section) => (
            <article key={section.section_id} className="wiki-card">
              <h3>{section.title}</h3>
              <p>{section.generated_markdown}</p>
              <p className="decision-meta">
                model: {section.generation_provenance.model_id} / harness: {section.generation_provenance.harness_id}
              </p>
            </article>
          ))}
          <article className="wiki-card wiki-breakdown" data-testid="ikam-breakdown">
            <h3>{wiki.ikam_breakdown.title}</h3>
            <p>{wiki.ikam_breakdown.generated_markdown}</p>
            <p className="decision-meta">
              model: {wiki.ikam_breakdown.generation_provenance.model_id} / harness: {wiki.ikam_breakdown.generation_provenance.harness_id}
            </p>
          </article>
        </div>
      ) : null}
    </section>
  );
};

export default WikiWorkspace;
