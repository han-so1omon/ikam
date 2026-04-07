import type { ArtifactPreviewResponse } from '../../api/client';
import ContentViewer from './ContentViewer';

type Props = {
  left: ArtifactPreviewResponse;
  right: ArtifactPreviewResponse;
  layout?: 'auto' | 'horizontal' | 'vertical';
};

const textFromPreview = (preview: ArtifactPreviewResponse): string | null => {
  if (preview.kind !== 'text') return null;
  const text = preview.preview && typeof preview.preview === 'object' ? (preview.preview as Record<string, unknown>).text : null;
  return typeof text === 'string' ? text : null;
};

const buildSimpleDiff = (leftText: string, rightText: string): Array<{ kind: 'same' | 'left' | 'right'; text: string }> => {
  const leftLines = leftText.split('\n');
  const rightLines = rightText.split('\n');
  const max = Math.max(leftLines.length, rightLines.length);
  const rows: Array<{ kind: 'same' | 'left' | 'right'; text: string }> = [];
  for (let i = 0; i < max; i += 1) {
    const a = leftLines[i] ?? '';
    const b = rightLines[i] ?? '';
    if (a === b) {
      rows.push({ kind: 'same', text: a });
    } else {
      if (a) rows.push({ kind: 'left', text: a });
      if (b) rows.push({ kind: 'right', text: b });
    }
  }
  return rows;
};

const sizeLabel = (value: unknown): string => {
  if (typeof value !== 'number' || !Number.isFinite(value)) return 'n/a';
  return `${value} B`;
};

const ContentDiffViewer = ({ left, right, layout = 'auto' }: Props) => {
  const leftText = textFromPreview(left);
  const rightText = textFromPreview(right);
  const mixedTypes = left.kind !== right.kind;
  const gridClass = layout === 'vertical' ? 'content-diff-grid-vertical' : layout === 'horizontal' ? 'content-diff-grid-horizontal' : 'content-diff-grid-auto';

  return (
    <div className="content-diff-viewer" data-testid="content-diff-viewer">
      <div className="content-diff-meta" data-testid="content-diff-meta">
        <div>
          <strong>Left:</strong> {left.file_name} ({left.kind}, {left.mime_type}, {sizeLabel(left.metadata?.size_bytes)})
        </div>
        <div>
          <strong>Right:</strong> {right.file_name} ({right.kind}, {right.mime_type}, {sizeLabel(right.metadata?.size_bytes)})
        </div>
      </div>
      {leftText !== null && rightText !== null ? (
        <div className="content-diff-text">
          <h6>Text Diff</h6>
          <pre>
            {buildSimpleDiff(leftText, rightText).map((row, idx) => (
              <div key={`${row.kind}-${idx}`} className={`content-diff-row content-diff-row-${row.kind}`}>
                <span className="content-diff-gutter">{row.kind === 'same' ? ' ' : row.kind === 'left' ? '-' : '+'}</span>
                <span>{row.text || ' '}</span>
              </div>
            ))}
          </pre>
        </div>
      ) : null}
      {mixedTypes ? (
        <p className="content-diff-note">
          Comparison mode: raw views (different content kinds). Left is <strong>{left.kind}</strong>, right is <strong>{right.kind}</strong>.
        </p>
      ) : null}
      <div className={`content-diff-grid ${gridClass}`} data-testid="content-diff-grid">
        <div>
          <h6>Source</h6>
          <ContentViewer preview={left} />
        </div>
        <div>
          <h6>Target</h6>
          <ContentViewer preview={right} />
        </div>
      </div>
    </div>
  );
};

export default ContentDiffViewer;
