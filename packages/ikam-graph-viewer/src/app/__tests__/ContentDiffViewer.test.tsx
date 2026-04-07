import { render, screen } from '@testing-library/react';

import ContentDiffViewer from '../components/content-viewer/ContentDiffViewer';

test('renders mixed-type previews in raw comparison mode', () => {
  render(
    <ContentDiffViewer
      left={{
        kind: 'table',
        mime_type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
        file_name: 'left.xlsx',
        metadata: { size_bytes: 1234 },
        preview: { sheets: [{ sheet_name: 'Sheet1', rows: [['A', '1']] }] },
      }}
      right={{
        kind: 'slides',
        mime_type: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
        file_name: 'right.pptx',
        metadata: { size_bytes: 987 },
        preview: { slides: [{ title: 'Plan', lines: ['Line one'] }] },
      }}
    />
  );

  expect(screen.getByText(/Comparison mode: raw views/i)).toBeInTheDocument();
  const meta = screen.getByTestId('content-diff-meta');
  expect(meta.textContent).toContain('left.xlsx');
  expect(meta.textContent).toContain('right.pptx');
  expect(screen.getByTestId('content-diff-grid')).toBeInTheDocument();
});

test('supports explicit vertical layout mode', () => {
  render(
    <ContentDiffViewer
      layout="vertical"
      left={{
        kind: 'text',
        mime_type: 'text/plain',
        file_name: 'a.txt',
        metadata: {},
        preview: { text: 'hello\nworld' },
      }}
      right={{
        kind: 'text',
        mime_type: 'text/plain',
        file_name: 'b.txt',
        metadata: {},
        preview: { text: 'hello\nteam' },
      }}
    />
  );

  const grid = screen.getByTestId('content-diff-grid');
  expect(grid.className).toContain('content-diff-grid-vertical');
  expect(screen.getByText('Text Diff')).toBeInTheDocument();
});
