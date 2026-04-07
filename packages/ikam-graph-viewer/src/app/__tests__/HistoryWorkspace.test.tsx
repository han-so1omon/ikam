import { vi } from 'vitest';
import { fireEvent, render, screen, waitFor } from '@testing-library/react';

import HistoryWorkspace from '../components/HistoryWorkspace';

const { mockGetHistoryRefs, mockGetHistoryCommits, mockGetHistoryCommitDetail, mockGetHistorySemanticGraph } = vi.hoisted(() => ({
  mockGetHistoryRefs: vi.fn(),
  mockGetHistoryCommits: vi.fn(),
  mockGetHistoryCommitDetail: vi.fn(),
  mockGetHistorySemanticGraph: vi.fn(),
}));

vi.mock('../api/client', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('../api/client');
  return {
    ...actual,
    getHistoryRefs: mockGetHistoryRefs,
    getHistoryCommits: mockGetHistoryCommits,
    getHistoryCommitDetail: mockGetHistoryCommitDetail,
    getHistorySemanticGraph: mockGetHistorySemanticGraph,
  };
});

beforeEach(() => {
  mockGetHistoryRefs.mockReset();
  mockGetHistoryCommits.mockReset();
  mockGetHistoryCommitDetail.mockReset();
  mockGetHistorySemanticGraph.mockReset();
});

test('loads refs, commits, and selected commit semantic graph', async () => {
  mockGetHistoryRefs.mockResolvedValue({ refs: [{ ref: 'refs/heads/main', commit_id: 'commit-1' }] });
  mockGetHistoryCommits.mockResolvedValue({ commits: [{ id: 'commit-1', profile: 'modelado/commit-entry@1', content: { ref: 'refs/heads/main', parents: [] } }] });
  mockGetHistoryCommitDetail.mockResolvedValue({ commit: { id: 'commit-1', profile: 'modelado/commit-entry@1', content: { ref: 'refs/heads/main', commit_policy: 'semantic_relations_only', parents: [] } } });
  mockGetHistorySemanticGraph.mockResolvedValue({ commit_id: 'commit-1', nodes: [{ id: 'prop-1', kind: 'proposition' }], edges: [] });

  render(<HistoryWorkspace runId="run-history" />);

  await waitFor(() => expect(screen.getByText('refs/heads/main')).toBeInTheDocument());
  expect(screen.getByText('commit-1')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button', { name: /commit-1/i }));
  await waitFor(() => expect(screen.getByText('modelado/commit-entry@1')).toBeInTheDocument());
  expect(screen.getByText('prop-1')).toBeInTheDocument();
});
