import type { ArtifactPreviewResponse } from '../../api/client';
import ReactMarkdown from 'react-markdown';
import { JsonView, defaultStyles } from 'react-json-view-lite';
import remarkGfm from 'remark-gfm';
import 'react-json-view-lite/dist/index.css';

type Props = {
  preview: ArtifactPreviewResponse | null;
};

const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === 'object' ? (value as Record<string, unknown>) : {};

const asString = (value: unknown): string => (typeof value === 'string' ? value : '');

const asStringArray = (value: unknown): string[] =>
  Array.isArray(value) ? value.filter((item): item is string => typeof item === 'string') : [];

const looksLikeMarkdown = (text: string): boolean => {
  if (!text.trim()) return false;
  return /(^|\n)#{1,6}\s+\S|(^|\n)[*-]\s+\S|(^|\n)\d+\.\s+\S|\[[^\]]+\]\([^\)]+\)|(^|\n)\|.+\|.+\||`[^`]+`|\*\*[^*]+\*\*/m.test(text);
};

const tryParseJsonText = (text: string): unknown | null => {
  const trimmed = text.trim();
  if (!trimmed) return null;
  if (!(trimmed.startsWith('{') || trimmed.startsWith('['))) return null;
  try {
    return JSON.parse(trimmed);
  } catch {
    return null;
  }
};

const parseTableRegionText = (text: string): string[][] =>
  text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => {
      const rowText = line.replace(/^Row\s+\d+\s*:\s*/i, '');
      return rowText.split('|').map((cell) => cell.trim());
    })
    .filter((row) => row.length > 0 && row.some((cell) => cell.length > 0));

const parseSlideShapeText = (text: string): { title: string; lines: string[]; notes: string } => {
  const parsed = { title: '', lines: [] as string[], notes: '' };
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line) continue;
    if (line.startsWith('Title:')) {
      parsed.title = line.replace(/^Title:\s*/, '').trim();
      continue;
    }
    if (line.startsWith('Notes:')) {
      parsed.notes = line.replace(/^Notes:\s*/, '').trim();
      continue;
    }
    parsed.lines.push(line.replace(/^•\s*/, '').trim());
  }
  return parsed;
};

type SlideData = { title: string; lines: string[]; notes?: string };

const normalizePdfFragmentText = (text: string): string => text.replace(/\(cid:127\)/g, '•').replace(/\u2022/g, '•');

const parsePdfFragmentText = (text: string): SlideData[] => {
  const lines = normalizePdfFragmentText(text)
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.length === 0) return [];

  const sections: SlideData[] = [];
  let current: SlideData | null = null;
  const isHeading = (line: string): boolean => !line.startsWith('•') && line.split(' ').length <= 6 && /^[A-Z]/.test(line);

  for (const line of lines) {
    if (!current) {
      current = { title: isHeading(line) ? line : 'PDF Section', lines: isHeading(line) ? [] : [line] };
      continue;
    }
    if (isHeading(line) && current.lines.length > 0) {
      sections.push(current);
      current = { title: line, lines: [] };
      continue;
    }
    current.lines.push(line.replace(/^•\s*/, ''));
  }
  if (current) sections.push(current);
  return sections;
};

const extractRelationSlots = (value: unknown): { relationType: string; entries: Array<{ key: string; value: string }> } | null => {
  if (!value || typeof value !== 'object') return null;
  const record = value as Record<string, unknown>;
  const relationType = typeof record.relation_type === 'string'
    ? record.relation_type
    : (typeof record.predicate === 'string' ? record.predicate : 'semantic_relation');

  const explicitBindings = record.slot_bindings;
  let bindings: Record<string, unknown> | null = null;
  if (explicitBindings && typeof explicitBindings === 'object' && !Array.isArray(explicitBindings)) {
    bindings = explicitBindings as Record<string, unknown>;
  } else if ('subject' in record || 'object' in record) {
    bindings = {
      subject: record.subject,
      object: record.object,
    };
  }

  if (!bindings) return null;

  const entries = Object.entries(bindings).map(([key, rawValue]) => ({
    key,
    value: typeof rawValue === 'string' ? rawValue : JSON.stringify(rawValue),
  }));
  if (entries.length === 0) return null;
  return { relationType, entries };
};

const SlideDeck = ({ slides }: { slides: SlideData[] }) => (
  <div className="slide-deck">
    {slides.length === 0 ? <p>n/a</p> : null}
    {slides.map((slide, index) => (
      <section key={`${slide.title}-${index}`} className="slide-card">
        <div className="slide-canvas">
          <header className="slide-title">{slide.title || `Slide ${index + 1}`}</header>
          <div className="slide-body">
            {slide.lines.length > 0 ? (
              <ul>
                {slide.lines.slice(0, 24).map((line, lineIndex) => (
                  <li key={`${slide.title}-line-${lineIndex}`}>{line}</li>
                ))}
              </ul>
            ) : (
              <p>(Empty slide)</p>
            )}
          </div>
        </div>
        {slide.notes ? <p className="slide-notes"><strong>Notes:</strong> {slide.notes}</p> : null}
        <p className="slide-index">Slide {index + 1} / {slides.length}</p>
      </section>
    ))}
  </div>
);

const JsonPreview = ({ value }: { value: unknown }) => (
  <div className="fragment-preview json-preview">
    <JsonView
      data={value}
      shouldInitiallyExpand={(level) => level < 1}
      style={defaultStyles}
      clickToExpandNode
    />
  </div>
);

const ContentViewer = ({ preview }: Props) => {
  if (!preview) {
    return <p><strong>content_preview:</strong> n/a</p>;
  }

  const payload = asRecord(preview.preview);

  if (preview.kind === 'text') {
    const text = typeof payload.text === 'string' ? payload.text : '';
    if (preview.mime_type === 'application/ikam-table-region+json') {
      const rows = parseTableRegionText(text);
      return (
        <div>
          <strong>content_preview:</strong>
          <div className="fragment-preview">
            <section className="content-viewer-section">
              <h6>Table Fragment</h6>
              {rows.length === 0 ? (
                <p>n/a</p>
              ) : (
                <table className="content-viewer-table">
                  <tbody>
                    {rows.slice(0, 20).map((row, rowIndex) => (
                      <tr key={`table-fragment-row-${rowIndex}`}>
                        {row.slice(0, 12).map((cell, cellIndex) => (
                          <td key={`table-fragment-row-${rowIndex}-cell-${cellIndex}`}>{cell || '\u00a0'}</td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </section>
          </div>
        </div>
      );
    }
    if (preview.mime_type === 'application/ikam-slide-shape+json') {
      const slide = parseSlideShapeText(text);
      return (
        <div>
          <strong>content_preview:</strong>
          <div className="fragment-preview">
            <SlideDeck slides={[{ title: slide.title || 'Slide Fragment', lines: slide.lines, notes: slide.notes }]} />
          </div>
        </div>
      );
    }
    if (preview.mime_type === 'application/ikam-pdf-page+json') {
      const sections = parsePdfFragmentText(text);
      return (
        <div>
          <strong>content_preview:</strong>
          <div className="fragment-preview">
            <SlideDeck slides={sections.length > 0 ? sections : [{ title: 'PDF Section', lines: [normalizePdfFragmentText(text)] }]} />
          </div>
        </div>
      );
    }
    const parsedJson = tryParseJsonText(text);
    if (parsedJson !== null) {
      const relationSlots = preview.mime_type === 'application/vnd.ikam.claim-ir+json' ? extractRelationSlots(parsedJson) : null;
      return (
        <div>
          <strong>content_preview:</strong>
          {relationSlots ? (
            <section className="content-viewer-section">
              <h6>Relation Parameters</h6>
              <p><strong>relation_type:</strong> {relationSlots.relationType}</p>
              <ul>
                {relationSlots.entries.map((entry) => (
                  <li key={`slot-${entry.key}`}><strong>{entry.key}:</strong> {entry.value}</li>
                ))}
              </ul>
            </section>
          ) : null}
          <JsonPreview value={parsedJson} />
        </div>
      );
    }
    if (preview.mime_type === 'text/markdown' || looksLikeMarkdown(text)) {
      return (
        <div>
          <strong>content_preview:</strong>
          <div className="fragment-preview markdown-preview">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{text || 'n/a'}</ReactMarkdown>
          </div>
        </div>
      );
    }
    return (
      <div>
        <strong>content_preview:</strong>
        <pre className="fragment-preview">{text || 'n/a'}</pre>
      </div>
    );
  }

  if (preview.kind === 'json') {
    const parsed = payload.parsed ?? {};
    const relationSlots = preview.mime_type === 'application/vnd.ikam.claim-ir+json' ? extractRelationSlots(parsed) : null;
    return (
      <div>
        <strong>content_preview:</strong>
        {relationSlots ? (
          <section className="content-viewer-section">
            <h6>Relation Parameters</h6>
            <p><strong>relation_type:</strong> {relationSlots.relationType}</p>
            <ul>
              {relationSlots.entries.map((entry) => (
                <li key={`slot-${entry.key}`}><strong>{entry.key}:</strong> {entry.value}</li>
              ))}
            </ul>
          </section>
        ) : null}
        <JsonPreview value={parsed} />
      </div>
    );
  }

  if (preview.kind === 'pdf') {
    const bytes = typeof payload.bytes_b64 === 'string' ? payload.bytes_b64 : '';
    const src = bytes ? `data:${preview.mime_type};base64,${bytes}` : '';
    if (!src) return <p><strong>content_preview:</strong> n/a</p>;
    return (
      <div>
        <strong>content_preview:</strong>
        <iframe title={preview.file_name} className="fragment-preview-frame" src={src} />
      </div>
    );
  }

  if (preview.kind === 'image') {
    const dataUrl = typeof payload.data_url === 'string' ? payload.data_url : '';
    if (!dataUrl) return <p><strong>content_preview:</strong> n/a</p>;
    return (
      <div>
        <strong>content_preview:</strong>
        <div className="fragment-preview-image-wrap">
          <img src={dataUrl} alt={preview.file_name} className="fragment-preview-image" />
        </div>
      </div>
    );
  }

  if (preview.kind === 'table') {
    const sheets = Array.isArray(payload.sheets) ? payload.sheets : [];
    return (
      <div>
        <strong>content_preview:</strong>
        <div className="fragment-preview">
          {sheets.length === 0 ? <p>n/a</p> : null}
          {sheets.map((sheet, index) => {
            const sheetRecord = asRecord(sheet);
            const sheetName = asString(sheetRecord.sheet_name) || `Sheet ${index + 1}`;
            const rows = Array.isArray(sheetRecord.rows) ? sheetRecord.rows : [];
            return (
              <section key={`${sheetName}-${index}`} className="content-viewer-section">
                <h6>{sheetName}</h6>
                <table className="content-viewer-table">
                  <tbody>
                    {rows.slice(0, 16).map((row, rowIndex) => {
                      const cells = Array.isArray(row) ? row : [];
                      return (
                        <tr key={`${sheetName}-row-${rowIndex}`}>
                          {cells.slice(0, 12).map((cell, cellIndex) => (
                            <td key={`${sheetName}-row-${rowIndex}-cell-${cellIndex}`}>{asString(cell) || '\u00a0'}</td>
                          ))}
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </section>
            );
          })}
        </div>
      </div>
    );
  }

  if (preview.kind === 'slides') {
    const slides = Array.isArray(payload.slides) ? payload.slides : [];
    const parsedSlides: SlideData[] = slides.map((slide, index) => {
      const slideRecord = asRecord(slide);
      return {
        title: asString(slideRecord.title) || `Slide ${index + 1}`,
        lines: asStringArray(slideRecord.lines),
      };
    });
    return (
      <div>
        <strong>content_preview:</strong>
        <div className="fragment-preview">
          <SlideDeck slides={parsedSlides} />
        </div>
      </div>
    );
  }

  if (preview.kind === 'doc') {
    const paragraphs = Array.isArray(payload.paragraphs) ? payload.paragraphs : [];
    return (
      <div>
        <strong>content_preview:</strong>
        <div className="fragment-preview">
          {paragraphs.length === 0 ? <p>n/a</p> : null}
          {paragraphs.slice(0, 120).map((paragraph, index) => (
            <p key={`paragraph-${index}`}>{asString(paragraph)}</p>
          ))}
        </div>
      </div>
    );
  }

  return (
    <div>
      <strong>content_preview:</strong>
      <pre className="fragment-preview">{JSON.stringify(payload, null, 2)}</pre>
    </div>
  );
};

export default ContentViewer;
