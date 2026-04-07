import { useMemo, useState } from 'react';

import type { AnswerQualitySummary, EnrichmentItem, EnrichmentReceipt, EnrichmentRun } from '../api/client';

type ReviewPanelProps = {
  runId: string;
  graphId: string;
  answerQuality?: AnswerQualitySummary;
  enrichmentRuns: EnrichmentRun[];
  enrichmentItems: EnrichmentItem[];
  enrichmentReceipts: EnrichmentReceipt[];
  onSaveReview: (runId: string, review: { query_id: string; relevance: number; fidelity: number; clarity: number; note: string }) => Promise<void>;
  onApproveEnrichment: (graphId: string, enrichmentId: string) => Promise<void>;
  onRejectEnrichment: (graphId: string, enrichmentId: string) => Promise<void>;
  onCommitStage: (graphId: string) => Promise<void>;
};

type PanelTab = 'review' | 'enrichment' | 'stage';

const ReviewPanel = ({
  runId,
  graphId,
  answerQuality,
  enrichmentRuns = [],
  enrichmentItems = [],
  enrichmentReceipts = [],
  onSaveReview,
  onApproveEnrichment,
  onRejectEnrichment,
  onCommitStage,
}: ReviewPanelProps) => {
  const [activeTab, setActiveTab] = useState<PanelTab>('review');
  const queryScores = answerQuality?.query_scores ?? [];
  const defaultQueryId = queryScores[0]?.query_id ?? '';
  const [queryId, setQueryId] = useState(defaultQueryId);
  const [relevance, setRelevance] = useState('0.7');
  const [fidelity, setFidelity] = useState('0.7');
  const [clarity, setClarity] = useState('0.7');
  const [note, setNote] = useState('');
  const [saving, setSaving] = useState(false);
  const [mutatingId, setMutatingId] = useState<string | null>(null);
  const selected = useMemo(() => queryScores.find((query) => query.query_id === queryId) ?? queryScores[0], [queryId, queryScores]);
  const queued = enrichmentItems.filter((item) => item.status === 'queued');

  if (!runId) {
    return null;
  }

  return (
    <section className="review-panel glass-panel" data-testid="review-panel">
      <header className="review-panel-header">
        <h3>Review Workspace</h3>
        <span className="review-mode-badge">{answerQuality?.review_mode ?? 'oracle-defaulted'}</span>
      </header>

      <div className="review-tabs" role="tablist" aria-label="Review tabs">
        <button type="button" role="tab" aria-selected={activeTab === 'review'} onClick={() => setActiveTab('review')}>
          Review
        </button>
        <button type="button" role="tab" aria-selected={activeTab === 'enrichment'} onClick={() => setActiveTab('enrichment')}>
          Enrichment
        </button>
        <button type="button" role="tab" aria-selected={activeTab === 'stage'} onClick={() => setActiveTab('stage')}>
          Stage
        </button>
      </div>

      {activeTab === 'review' ? (
        selected ? (
          <>
            <label>
              Query
              <select value={queryId} onChange={(event) => setQueryId(event.target.value)}>
                {queryScores.map((query) => (
                  <option key={query.query_id} value={query.query_id}>
                    {query.query_id}
                  </option>
                ))}
              </select>
            </label>

            <label>
              Relevance
              <input value={relevance} onChange={(event) => setRelevance(event.target.value)} type="number" min="0" max="1" step="0.01" />
            </label>

            <label>
              Fidelity
              <input value={fidelity} onChange={(event) => setFidelity(event.target.value)} type="number" min="0" max="1" step="0.01" />
            </label>

            <label>
              Clarity
              <input value={clarity} onChange={(event) => setClarity(event.target.value)} type="number" min="0" max="1" step="0.01" />
            </label>

            <label>
              Note
              <textarea value={note} onChange={(event) => setNote(event.target.value)} rows={3} />
            </label>

            <button
              type="button"
              disabled={saving}
              onClick={async () => {
                setSaving(true);
                try {
                  await onSaveReview(runId, {
                    query_id: selected.query_id,
                    relevance: Number(relevance),
                    fidelity: Number(fidelity),
                    clarity: Number(clarity),
                    note,
                  });
                } finally {
                  setSaving(false);
                }
              }}
            >
              Save Review
            </button>
          </>
        ) : (
          <div className="panel-placeholder">No review queries available for this run</div>
        )
      ) : null}

      {activeTab === 'enrichment' ? (
        <div className="review-table-wrap" data-testid="enrichment-tab">
          <table className="review-table">
            <thead>
              <tr>
                <th>Run</th>
                <th>Mode</th>
                <th>Status</th>
                <th>Relations</th>
                <th>Unresolved</th>
                <th>Rationale</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              {enrichmentRuns.map((item) => (
                <tr key={item.enrichment_id}>
                  <td>{item.sequence}</td>
                  <td>{item.lane_mode}</td>
                  <td>{item.status}</td>
                  <td>{item.relation_count}</td>
                  <td>{item.unresolved_count}</td>
                  <td>
                    {enrichmentItems
                      .filter((candidate) => candidate.enrichment_id === item.enrichment_id)
                      .map((candidate) => candidate.rationale)
                      .filter(Boolean)
                      .slice(0, 1)
                      .join('') || '--'}
                  </td>
                  <td>
                    <button
                      type="button"
                      disabled={mutatingId === item.enrichment_id}
                      onClick={async () => {
                        setMutatingId(item.enrichment_id);
                        try {
                          await onApproveEnrichment(graphId, item.enrichment_id);
                        } finally {
                          setMutatingId(null);
                        }
                      }}
                    >
                      Approve
                    </button>
                    <button
                      type="button"
                      disabled={mutatingId === item.enrichment_id}
                      onClick={async () => {
                        setMutatingId(item.enrichment_id);
                        try {
                          await onRejectEnrichment(graphId, item.enrichment_id);
                        } finally {
                          setMutatingId(null);
                        }
                      }}
                    >
                      Reject
                    </button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}

      {activeTab === 'stage' ? (
        <div className="review-table-wrap" data-testid="stage-tab">
          <button
            type="button"
            disabled={queued.length === 0 || saving}
            onClick={async () => {
              setSaving(true);
              try {
                await onCommitStage(graphId);
              } finally {
                setSaving(false);
              }
            }}
          >
            Commit Queue ({queued.length})
          </button>
          <table className="review-table">
            <thead>
              <tr>
                <th>Relation</th>
                <th>Kind</th>
                <th>Source</th>
                <th>Target</th>
                <th>Status</th>
              </tr>
            </thead>
            <tbody>
              {queued.map((item) => (
                <tr key={item.relation_id}>
                  <td>{item.relation_id}</td>
                  <td>{item.relation_kind}</td>
                  <td>{item.source}</td>
                  <td>{item.target}</td>
                  <td>{item.status}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <div className="review-receipts" data-testid="stage-receipts">
            {enrichmentReceipts.map((receipt) => (
              <div key={receipt.receipt_id} className="review-receipt-item">
                {receipt.receipt_id}: committed {receipt.committed} ({receipt.committed_edges} edges / {receipt.committed_relations} relations)
              </div>
            ))}
          </div>
        </div>
      ) : null}
    </section>
  );
};

export default ReviewPanel;
