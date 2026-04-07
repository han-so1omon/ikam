import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import ReviewPanel from '../components/ReviewPanel';

test('shows oracle-defaulted badge and query list', () => {
  render(
    <ReviewPanel
      runId="run-1"
      graphId="graph-1"
      answerQuality={{
        aqs: 0.81,
        review_mode: 'oracle-defaulted',
        review_coverage: 0,
        query_scores: [
          {
            query_id: 'q-1',
            oracle_score: 0.81,
            reviewer_score: 0.81,
            aqs: 0.81,
            review_mode: 'oracle-defaulted',
            oracle: { coverage: 0.8, grounded_precision: 0.82 },
            review: { relevance: 0.81, fidelity: 0.81, clarity: 0.81, note: '' },
          },
        ],
      }}
      enrichmentRuns={[]}
      enrichmentItems={[]}
      enrichmentReceipts={[]}
      onSaveReview={async () => {}}
      onApproveEnrichment={async () => {}}
      onRejectEnrichment={async () => {}}
      onCommitStage={async () => {}}
    />
  );

  expect(screen.getByText('oracle-defaulted')).toBeInTheDocument();
  expect(screen.getByRole('option', { name: 'q-1' })).toBeInTheDocument();
});

test('submits manual review rubric and note', async () => {
  const onSaveReview = vi.fn().mockResolvedValue(undefined);
  render(
    <ReviewPanel
      runId="run-1"
      graphId="graph-1"
      answerQuality={{
        aqs: 0.81,
        review_mode: 'oracle-defaulted',
        review_coverage: 0,
        query_scores: [
          {
            query_id: 'q-1',
            oracle_score: 0.81,
            reviewer_score: 0.81,
            aqs: 0.81,
            review_mode: 'oracle-defaulted',
            oracle: { coverage: 0.8, grounded_precision: 0.82 },
            review: { relevance: 0.81, fidelity: 0.81, clarity: 0.81, note: '' },
          },
        ],
      }}
      enrichmentRuns={[]}
      enrichmentItems={[]}
      enrichmentReceipts={[]}
      onSaveReview={onSaveReview}
      onApproveEnrichment={async () => {}}
      onRejectEnrichment={async () => {}}
      onCommitStage={async () => {}}
    />
  );

  fireEvent.change(screen.getByLabelText('Relevance'), { target: { value: '0.7' } });
  fireEvent.change(screen.getByLabelText('Fidelity'), { target: { value: '0.6' } });
  fireEvent.change(screen.getByLabelText('Clarity'), { target: { value: '0.8' } });
  fireEvent.change(screen.getByLabelText('Note'), { target: { value: 'Good evidence path' } });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Save Review' }));
  });

  await waitFor(() =>
    expect(onSaveReview).toHaveBeenCalledWith('run-1', {
      query_id: 'q-1',
      relevance: 0.7,
      fidelity: 0.6,
      clarity: 0.8,
      note: 'Good evidence path',
    })
  );
});

test('shows enrichment and stage tabs with queued items', async () => {
  const onCommitStage = vi.fn().mockResolvedValue(undefined);
  render(
    <ReviewPanel
      runId="run-1"
      graphId="graph-1"
      answerQuality={{ aqs: 0.8, review_mode: 'oracle-defaulted', review_coverage: 0, query_scores: [] }}
      enrichmentRuns={[
        {
          enrichment_id: 'e-1',
          run_id: 'run-1',
          graph_id: 'graph-1',
          sequence: 1,
          lane_mode: 'explore-graph',
          status: 'staged',
          relation_count: 1,
          unresolved_count: 0,
        },
      ]}
      enrichmentItems={[
        {
          enrichment_id: 'e-1',
          run_id: 'run-1',
          graph_id: 'graph-1',
          relation_id: 'r-1',
          relation_kind: 'semantic_link',
          source: 'n1',
          target: 'n2',
          evidence: ['reasoning:demo'],
          status: 'queued',
          sequence: 1,
          lane_mode: 'explore-graph',
          unresolved: false,
        },
      ]}
      enrichmentReceipts={[]}
      onSaveReview={async () => {}}
      onApproveEnrichment={async () => {}}
      onRejectEnrichment={async () => {}}
      onCommitStage={onCommitStage}
    />
  );

  fireEvent.click(screen.getByRole('tab', { name: 'Enrichment' }));
  expect(screen.getByTestId('enrichment-tab')).toBeInTheDocument();

  fireEvent.click(screen.getByRole('tab', { name: 'Stage' }));
  expect(screen.getByTestId('stage-tab')).toBeInTheDocument();
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: 'Commit Queue (1)' }));
  });
  await waitFor(() => expect(onCommitStage).toHaveBeenCalledWith('graph-1'));
});
