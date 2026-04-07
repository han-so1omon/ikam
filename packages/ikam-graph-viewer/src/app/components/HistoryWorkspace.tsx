import { useEffect, useMemo, useState } from 'react';

import {
  getHistoryCommitDetail,
  getHistoryCommits,
  getHistoryRefs,
  getHistorySemanticGraph,
  type HistoryCommitEntry,
  type HistoryRefEntry,
  type HistorySemanticGraphResponse,
} from '../api/client';

type HistoryWorkspaceProps = {
  runId: string | null;
};

const HistoryWorkspace = ({ runId }: HistoryWorkspaceProps) => {
  const [refs, setRefs] = useState<HistoryRefEntry[]>([]);
  const [commits, setCommits] = useState<HistoryCommitEntry[]>([]);
  const [selectedCommitId, setSelectedCommitId] = useState<string | null>(null);
  const [selectedCommit, setSelectedCommit] = useState<HistoryCommitEntry | null>(null);
  const [semanticGraph, setSemanticGraph] = useState<HistorySemanticGraphResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const loadHistory = async () => {
      if (!runId) {
        setRefs([]);
        setCommits([]);
        setSelectedCommitId(null);
        setSelectedCommit(null);
        setSemanticGraph(null);
        setLoading(false);
        setError(null);
        return;
      }
      setLoading(true);
      setError(null);
      try {
        const [refsResponse, commitsResponse] = await Promise.all([
          getHistoryRefs(runId),
          getHistoryCommits({ runId }),
        ]);
        if (cancelled) return;
        const nextRefs = refsResponse.refs ?? [];
        const nextCommits = commitsResponse.commits ?? [];
        setRefs(nextRefs);
        setCommits(nextCommits);
        const defaultCommitId = nextRefs[0]?.commit_id || nextCommits[0]?.id || null;
        setSelectedCommitId(defaultCommitId);
      } catch (loadError) {
        if (cancelled) return;
        setError(loadError instanceof Error ? loadError.message : 'Failed loading history');
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    };
    void loadHistory();
    return () => {
      cancelled = true;
    };
  }, [runId]);

  useEffect(() => {
    let cancelled = false;
    const loadCommit = async () => {
      if (!runId || !selectedCommitId) {
        setSelectedCommit(null);
        setSemanticGraph(null);
        return;
      }
      try {
        const [commitResponse, semanticResponse] = await Promise.all([
          getHistoryCommitDetail({ runId, commitId: selectedCommitId }),
          getHistorySemanticGraph({ runId, commitId: selectedCommitId }),
        ]);
        if (cancelled) return;
        setSelectedCommit(commitResponse.commit);
        setSemanticGraph(semanticResponse);
      } catch (loadError) {
        if (cancelled) return;
        setError(loadError instanceof Error ? loadError.message : 'Failed loading commit detail');
      }
    };
    void loadCommit();
    return () => {
      cancelled = true;
    };
  }, [runId, selectedCommitId]);

  const commitProfile = useMemo(() => selectedCommit?.profile ?? null, [selectedCommit]);
  const semanticNodes = Array.isArray(semanticGraph?.nodes) ? semanticGraph.nodes : [];

  return (
    <section className="panel panel-muted" data-testid="history-workspace">
      <h2>History Workspace</h2>
      <p className="panel-subtitle">Source: /history/refs, /history/commits, /history/commits/:commit_id/semantic-graph</p>
      <p className="decision-meta">Run: {runId ?? '--'}</p>

      {loading ? <div className="panel-placeholder">Loading history...</div> : null}
      {error ? <div className="panel-placeholder">Error: {error}</div> : null}

      <div className="evaluation-debug-grid">
        <section>
          <h5>Refs</h5>
          {refs.length === 0 ? <p className="panel-subtitle">No refs available.</p> : null}
          <ul>
            {refs.map((ref) => (
              <li key={`${ref.ref}:${ref.commit_id}`}>
                <strong>{ref.ref}</strong> {'->'} {ref.commit_id}
              </li>
            ))}
          </ul>
        </section>

        <section>
          <h5>Commits</h5>
          {commits.length === 0 ? <p className="panel-subtitle">No commits available.</p> : null}
          <div className="panel-actions">
            {commits.map((commit) => (
              <button
                key={commit.id}
                type="button"
                onClick={() => setSelectedCommitId(commit.id)}
                className={selectedCommitId === commit.id ? 'tab-active' : ''}
              >
                {commit.id}
              </button>
            ))}
          </div>
        </section>

        <section>
          <h5>Commit Detail</h5>
          {selectedCommit ? (
            <>
              <p>{`id: ${selectedCommit.id}`}</p>
              <p><strong>profile:</strong> {commitProfile ?? 'n/a'}</p>
              <pre className="evaluation-report">{JSON.stringify(selectedCommit.content ?? {}, null, 2)}</pre>
            </>
          ) : (
            <p className="panel-subtitle">Select a commit to view details.</p>
          )}
        </section>

        <section>
          <h5>Semantic Graph</h5>
          {!semanticGraph ? <p className="panel-subtitle">No semantic graph loaded.</p> : null}
          {semanticGraph ? (
            <ul>
              {semanticNodes.map((node) => (
                <li key={node.id}>
                  <span>{node.id}</span>
                  <span> ({node.kind})</span>
                </li>
              ))}
            </ul>
          ) : null}
        </section>
      </div>
    </section>
  );
};

export default HistoryWorkspace;
