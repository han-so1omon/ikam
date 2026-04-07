import { render, screen } from '@testing-library/react';

import ContentViewer from '../components/content-viewer/ContentViewer';

test('renders table previews as readable sheet table', () => {
  render(
    <ContentViewer
      preview={{
        kind: 'table',
        mime_type: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
        file_name: 'inventory.xlsx',
        metadata: {},
        preview: {
          sheets: [
            {
              sheet_name: 'Sheet1',
              rows: [
                ['SKU', 'Qty'],
                ['A-1', '42'],
              ],
            },
          ],
        },
      }}
    />
  );

  expect(screen.getByText('Sheet1')).toBeInTheDocument();
  expect(screen.getByText('SKU')).toBeInTheDocument();
  expect(screen.getByText('A-1')).toBeInTheDocument();
});

test('renders slide previews as titled sections with bullets', () => {
  render(
    <ContentViewer
      preview={{
        kind: 'slides',
        mime_type: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
        file_name: 'deck.pptx',
        metadata: {},
        preview: {
          slides: [
            {
              title: 'Go-To-Market',
              lines: ['Segment focus', 'Pricing update'],
            },
          ],
        },
      }}
    />
  );

  expect(screen.getByText('Go-To-Market')).toBeInTheDocument();
  expect(screen.getByText('Segment focus')).toBeInTheDocument();
  expect(screen.getByText('Pricing update')).toBeInTheDocument();
  expect(screen.getByText('Slide 1 / 1')).toBeInTheDocument();
});

test('renders json previews as structured key-value tree', () => {
  render(
    <ContentViewer
      preview={{
        kind: 'json',
        mime_type: 'application/json',
        file_name: 'summary.json',
        metadata: {},
        preview: {
          parsed: {
            company: 'Bramble',
            metrics: { revenue: 1200000, growth: 0.28 },
            flags: ['verified', 'seed-stage'],
          },
        },
      }}
    />
  );

  expect(screen.getAllByText('{').length).toBeGreaterThan(0);
  expect(screen.getByText(/company/)).toBeInTheDocument();
  expect(screen.getByText('"Bramble"')).toBeInTheDocument();
  expect(screen.getByText(/metrics/)).toBeInTheDocument();
  expect(screen.getAllByRole('button').length).toBeGreaterThan(0);
});

test('renders pdf-page text fragments as structured document section preview', () => {
  render(
    <ContentViewer
      preview={{
        kind: 'text',
        mime_type: 'application/ikam-pdf-page+json',
        file_name: 'gifting.pdf#page-1',
        metadata: {},
        preview: {
          text: 'Who we are\nA neighborhood specialty shop in Oakland.\nBundles\n(cid:127) Mini ($35)\n(cid:127) Classic ($55)',
        },
      }}
    />
  );

  expect(screen.getByText('Who we are')).toBeInTheDocument();
  expect(screen.getAllByRole('list').length).toBeGreaterThan(0);
  expect(screen.getByText('Mini ($35)')).toBeInTheDocument();
});

test('renders json-like text fragments with collapsible JSON view', () => {
  render(
    <ContentViewer
      preview={{
        kind: 'text',
        mime_type: 'text/ikam-paragraph',
        file_name: 'fragment-json',
        metadata: {},
        preview: {
          text: '{"company":"Bramble","metrics":{"revenue":1200000}}',
        },
      }}
    />
  );

  expect(screen.getAllByRole('button').length).toBeGreaterThan(0);
  expect(screen.getByText(/company/)).toBeInTheDocument();
});

test('renders markdown previews as rich GFM content', () => {
  render(
    <ContentViewer
      preview={{
        kind: 'text',
        mime_type: 'text/markdown',
        file_name: 'one-pager.md',
        metadata: {},
        preview: {
          text: '# Bramble\n\n- item one\n- item two\n\n**bold**',
        },
      }}
    />
  );

  expect(screen.getByRole('heading', { name: 'Bramble' })).toBeInTheDocument();
  expect(screen.getByRole('list')).toBeInTheDocument();
  expect(screen.getByText('bold')).toBeInTheDocument();
});

test('renders markdown-like text fragments as rich GFM content', () => {
  render(
    <ContentViewer
      preview={{
        kind: 'text',
        mime_type: 'text/ikam-paragraph',
        file_name: 'fragment.txt',
        metadata: {},
        preview: {
          text: '## Signals\n\n- Fast checks\n- Deterministic replay',
        },
      }}
    />
  );

  expect(screen.getByRole('heading', { name: 'Signals' })).toBeInTheDocument();
  expect(screen.getByRole('list')).toBeInTheDocument();
  expect(screen.getByText('Fast checks')).toBeInTheDocument();
});

test('renders table-region text fragments as structured table preview', () => {
  render(
    <ContentViewer
      preview={{
        kind: 'text',
        mime_type: 'application/ikam-table-region+json',
        file_name: 'fragment-table',
        metadata: {},
        preview: {
          text: 'Row 1: SKU | Qty\nRow 2: A-1 | 42',
        },
      }}
    />
  );

  expect(screen.getByText('Table Fragment')).toBeInTheDocument();
  expect(screen.getByRole('table')).toBeInTheDocument();
  expect(screen.getByText('SKU')).toBeInTheDocument();
  expect(screen.getByText('A-1')).toBeInTheDocument();
});

test('renders slide-shape text fragments as structured slide preview', () => {
  render(
    <ContentViewer
      preview={{
        kind: 'text',
        mime_type: 'application/ikam-slide-shape+json',
        file_name: 'fragment-slide',
        metadata: {},
        preview: {
          text: 'Title: Expansion Plan\n• Segment focus\n• Pricing update\nNotes: Keep sensitivity assumptions explicit.',
        },
      }}
    />
  );

  expect(screen.getByText('Expansion Plan')).toBeInTheDocument();
  expect(screen.getByRole('list')).toBeInTheDocument();
  expect(screen.getByText('Segment focus')).toBeInTheDocument();
  expect(screen.getByText(/Keep sensitivity assumptions explicit/)).toBeInTheDocument();
});

test('renders relation slot bindings for relational IR fragments', () => {
  render(
    <ContentViewer
      preview={{
        kind: 'json',
        mime_type: 'application/vnd.ikam.claim-ir+json',
        file_name: 'ir-claim-1',
        metadata: {},
        preview: {
          parsed: {
            relation_type: 'market_share_change',
            slot_bindings: {
              segment: 'SMB',
              delta_pct: 14,
              period: 'Q1-2025',
            },
          },
        },
      }}
    />
  );

  expect(screen.getByText('Relation Parameters')).toBeInTheDocument();
  expect(screen.getAllByText(/segment/i).length).toBeGreaterThan(0);
  expect(screen.getAllByText(/SMB/).length).toBeGreaterThan(0);
  expect(screen.getAllByText(/delta_pct/i).length).toBeGreaterThan(0);
});

test('renders relation parameters for subject-predicate-object IR fragments', () => {
  render(
    <ContentViewer
      preview={{
        kind: 'json',
        mime_type: 'application/vnd.ikam.claim-ir+json',
        file_name: 'ir-claim-2',
        metadata: {},
        preview: {
          parsed: {
            subject: { entity: 'revenue', scope: 'Q1' },
            predicate: 'increased_by',
            object: { value: 14, unit: 'pct' },
          },
        },
      }}
    />
  );

  expect(screen.getByText('Relation Parameters')).toBeInTheDocument();
  expect(screen.getAllByText(/increased_by/i).length).toBeGreaterThan(0);
  expect(screen.getAllByText(/subject/i).length).toBeGreaterThan(0);
  expect(screen.getAllByText(/object/i).length).toBeGreaterThan(0);
});
