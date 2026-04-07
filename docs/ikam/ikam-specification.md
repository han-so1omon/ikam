# Internal Knowledge Artifact Model (IKAM) Specification

_Version 1.0.0 | October 2025_

## Overview

The **Internal Knowledge Artifact Model (IKAM)** is Narraciones' canonical format for structured content and presentations. It provides a versioned, JSON-based representation that separates semantic content from layout, enabling multi-target export (PPTX, Google Slides, PDF, HTML, Notion, Markdown) while maintaining diffability, composability, and programmatic control.

**Note:** This document covers IKAM Document and IKAM Slide Deck. For spreadsheet functionality, see [IKAM Sheet Specification](./ikam-sheet-specification.md).

IKAM consists of two complementary layers:
1. **Document Model** — semantic content as a typed block tree
2. **Slide Layout Model** — canvas-style layout for presentations

---

## Design Principles

1. **Separation of Concerns** — content (semantic meaning) vs. layout (spatial arrangement)
2. **Versioned & Diffable** — JSON with semver schemas; Git-friendly for collaboration
3. **Composable** — blocks reference each other; slides reference document blocks
4. **Standard-Aligned** — use CommonMark, Vega-Lite, KaTeX where applicable
5. **Export-Agnostic** — one canonical source, many renderers (PPTX, HTML, etc.)
6. **Data-First** — tables and charts store structured data, not just visuals

---

## Document Model

### Root Document Schema

```typescript
interface Document {
  id: string;                    // UUID or stable identifier
  version: string;               // IKAM schema version (semver, e.g., "1.0.0")
  type: "document";
  meta: DocumentMeta;
  blocks: Block[];
}

interface DocumentMeta {
  title: string;
  description?: string;
  authors?: string[];
  tags?: string[];
  createdAt: string;             // ISO 8601 timestamp
  updatedAt: string;
  projectId?: string;            // Reference to parent project
  locale?: string;               // e.g., "en-US", "es-MX"
}
```

### Block Types

Blocks are discriminated unions. Each block has:
- `type` — discriminator field
- `id` — unique within document
- `content` — varies by type
- `children?` — for hierarchical structures
- `meta?` — optional metadata (comments, annotations)

#### Paragraph

```typescript
interface ParagraphBlock {
  type: "paragraph";
  id: string;
  content: InlineContent[];
  alignment?: "left" | "center" | "right" | "justify";
}
```

#### Heading

```typescript
interface HeadingBlock {
  type: "heading";
  id: string;
  level: 1 | 2 | 3 | 4 | 5 | 6;
  content: InlineContent[];
}
```

#### List

```typescript
interface ListBlock {
  type: "list";
  id: string;
  ordered: boolean;
  items: ListItem[];
}

interface ListItem {
  id: string;
  content: InlineContent[];
  children?: ListItem[];        // Nested lists
}
```

#### Table

```typescript
interface TableBlock {
  type: "table";
  id: string;
  schema: TableSchema;
  rows: Record<string, any>[];
  caption?: InlineContent[];
  displayOptions?: {
    headerRow?: boolean;
    stripeRows?: boolean;
    borders?: "none" | "horizontal" | "all";
    columnWidths?: number[];    // Proportional widths
  };
}

interface TableSchema {
  columns: TableColumn[];
}

interface TableColumn {
  key: string;                  // Field name in row objects
  label: string;                // Display label
  type: "string" | "number" | "boolean" | "date" | "currency";
  format?: string;              // e.g., "0.00%" for percentages
  alignment?: "left" | "center" | "right";
}
```

#### Chart

```typescript
interface ChartBlock {
  type: "chart";
  id: string;
  spec: VegaLiteSpec;           // Full Vega-Lite JSON spec
  dataRef?: string;             // Optional reference to TableBlock id
  caption?: InlineContent[];
  renderOptions?: {
    width?: number;
    height?: number;
    renderer?: "canvas" | "svg";
  };
}
```

#### Figure (Image/Illustration)

```typescript
interface FigureBlock {
  type: "figure";
  id: string;
  assetId: string;              // Reference to MinIO asset
  alt: string;
  caption?: InlineContent[];
  displayOptions?: {
    width?: number | "auto";
    alignment?: "left" | "center" | "right";
  };
}
```

#### Callout

```typescript
interface CalloutBlock {
  type: "callout";
  id: string;
  variant: "info" | "warning" | "success" | "error" | "note";
  icon?: string;                // Emoji or icon name
  content: Block[];             // Nested blocks
}
```

#### Equation

```typescript
interface EquationBlock {
  type: "equation";
  id: string;
  latex: string;                // KaTeX-compatible TeX
  displayMode: boolean;         // true = block, false = inline
}
```

#### Code

```typescript
interface CodeBlock {
  type: "code";
  id: string;
  language: string;             // e.g., "python", "javascript"
  code: string;
  caption?: InlineContent[];
  showLineNumbers?: boolean;
}
```

#### Divider

```typescript
interface DividerBlock {
  type: "divider";
  id: string;
  style?: "solid" | "dashed" | "dotted";
}
```

#### Embed

```typescript
interface EmbedBlock {
  type: "embed";
  id: string;
  embedType: "url" | "iframe" | "video" | "audio";
  url: string;
  title?: string;
  displayOptions?: {
    width?: number;
    height?: number;
    autoplay?: boolean;
  };
}
```

#### Reference (Cross-reference)

```typescript
interface ReferenceBlock {
  type: "reference";
  id: string;
  targetId: string;             // Block id being referenced
  displayText?: string;
}
```

### Inline Content & Marks

Inline content is represented as an array of text spans with optional marks (styling).

```typescript
type InlineContent = TextSpan[];

interface TextSpan {
  text: string;
  marks?: Mark[];
}

type Mark =
  | { type: "bold" }
  | { type: "italic" }
  | { type: "underline" }
  | { type: "strikethrough" }
  | { type: "code" }
  | { type: "link"; href: string; title?: string }
  | { type: "superscript" }
  | { type: "subscript" }
  | { type: "highlight"; color?: string }
  | { type: "color"; value: string };  // Text color
```

**Example:**

```json
[
  { "text": "Revenue grew " },
  { "text": "25%", "marks": [{ "type": "bold" }] },
  { "text": " YoY. See " },
  { "text": "analysis", "marks": [{ "type": "link", "href": "#block-abc123" }] },
  { "text": " for details." }
]
```

---

## Slide Layout Model

### Root Deck Schema

```typescript
interface SlideDeck {
  id: string;
  version: string;              // IKAM schema version
  type: "slide-deck";
  meta: DeckMeta;
  theme: Theme;
  slides: Slide[];
}

interface DeckMeta {
  title: string;
  description?: string;
  authors?: string[];
  createdAt: string;
  updatedAt: string;
  projectId?: string;
}
```

### Theme

```typescript
interface Theme {
  id: string;
  name: string;
  tokens: DesignTokens;
  masters: SlideMaster[];
}

interface DesignTokens {
  colors: Record<string, string>;      // e.g., { "primary": "#2E5D4E", "accent": "#C89968" }
  fonts: {
    heading: FontSpec;
    body: FontSpec;
    mono: FontSpec;
  };
  spacing: Record<string, number>;     // e.g., { "xs": 4, "sm": 8, "md": 16, "lg": 32 }
  borderRadius: Record<string, number>;
  shadows: Record<string, string>;
}

interface FontSpec {
  family: string;
  weight: number;
  lineHeight: number;
}

interface SlideMaster {
  id: string;
  name: string;                        // e.g., "title", "content", "section-divider"
  background?: BackgroundSpec;
  placeholders?: Placeholder[];        // Pre-positioned zones
}

interface BackgroundSpec {
  type: "solid" | "gradient" | "image";
  color?: string;
  gradientStops?: Array<{ color: string; position: number }>;
  imageAssetId?: string;
}

interface Placeholder {
  id: string;
  type: "title" | "body" | "image" | "chart" | "table";
  frame: Frame;
  styleRef?: string;
}
```

### Slide

```typescript
interface Slide {
  id: string;
  masterId?: string;                   // Reference to SlideMaster
  background?: BackgroundSpec;
  elements: Element[];
  notes?: string;                      // Speaker notes (plain text or markdown)
  transition?: TransitionSpec;
}

interface TransitionSpec {
  type: "none" | "fade" | "slide" | "wipe";
  duration?: number;                   // milliseconds
}
```

### Element Types

Elements are discriminated unions representing objects placed on slides.

#### Base Element

```typescript
interface BaseElement {
  id: string;
  frame: Frame;
  zIndex?: number;
  locked?: boolean;
  hidden?: boolean;
  styleRef?: string;                   // Reference to style in theme
}

interface Frame {
  x: number;                           // Position from left (0-1 normalized or pixels)
  y: number;                           // Position from top
  width: number;
  height: number;
  anchor?: "top-left" | "center" | "bottom-right";  // Anchor point
}
```

#### Text Element

```typescript
interface TextElement extends BaseElement {
  type: "text";
  contentRef?: string;                 // Optional reference to Document block id
  content?: InlineContent[];           // Inline content if not referencing
  style?: TextStyle;
}

interface TextStyle {
  fontFamily?: string;
  fontSize?: number;
  fontWeight?: number;
  color?: string;
  alignment?: "left" | "center" | "right" | "justify";
  verticalAlignment?: "top" | "middle" | "bottom";
  padding?: { top: number; right: number; bottom: number; left: number };
}
```

#### Table Element

```typescript
interface TableElement extends BaseElement {
  type: "table";
  contentRef: string;                  // Reference to TableBlock id in Document
  style?: TableStyle;
}

interface TableStyle {
  headerBackground?: string;
  rowBackground?: string;
  alternateRowBackground?: string;
  borderColor?: string;
  fontSize?: number;
}
```

#### Chart Element

```typescript
interface ChartElement extends BaseElement {
  type: "chart";
  contentRef: string;                  // Reference to ChartBlock id
  renderAssetId?: string;              // Pre-rendered PNG/SVG asset id
  style?: ChartStyle;
}

interface ChartStyle {
  background?: string;
  padding?: number;
}
```

#### Image Element

```typescript
interface ImageElement extends BaseElement {
  type: "image";
  assetId: string;                     // Reference to asset in MinIO
  alt?: string;
  fit?: "cover" | "contain" | "fill" | "scale-down";
  opacity?: number;
}
```

#### Shape Element

```typescript
interface ShapeElement extends BaseElement {
  type: "shape";
  shapeType: "rectangle" | "ellipse" | "triangle" | "line" | "arrow";
  fill?: string;
  stroke?: string;
  strokeWidth?: number;
  cornerRadius?: number;               // For rectangles
}
```

#### Data Binding (Advanced)

Elements can bind to dynamic data via JSONPath-like selectors:

```typescript
interface DataBinding {
  select: string;                      // e.g., "blocks[?type=='table' && id=='revenue-summary']"
  transform?: string;                  // Optional transformation expression
}
```

---

## Versioning & Migration

### Schema Versioning

- IKAM uses **semver** for schema versions (e.g., `1.0.0`, `1.1.0`, `2.0.0`).
- Breaking changes (incompatible structure) increment major version.
- Backward-compatible additions increment minor version.
- Patches for clarifications/fixes increment patch version.

### Migration Strategy

- Store documents and decks with their schema version.
- Maintain migration functions in `packages/common/narraciones_common/migrations/`.
- On read, detect version mismatch and apply migrations sequentially.
- Log migrations for audit trail.

**Example migration:**

```python
def migrate_1_0_to_1_1(doc: dict) -> dict:
    """Add 'locale' field to meta, default to 'en-US'."""
    if 'meta' not in doc:
        doc['meta'] = {}
    doc['meta'].setdefault('locale', 'en-US')
    doc['version'] = '1.1.0'
    return doc
```

---

## Storage

### Database Schema (PostgreSQL)

```sql
-- Documents table
CREATE TABLE documents (
  id UUID PRIMARY KEY,
  project_id UUID REFERENCES projects(id),
  version TEXT NOT NULL,                    -- IKAM schema version
  title TEXT NOT NULL,
  content JSONB NOT NULL,                   -- Full Document object
  created_at TIMESTAMPTZ DEFAULT NOW(),
  updated_at TIMESTAMPTZ DEFAULT NOW(),
  created_by TEXT,
  tags TEXT[]
);

CREATE INDEX idx_documents_project ON documents(project_id);
CREATE INDEX idx_documents_tags ON documents USING GIN(tags);

-- Slide decks table
CREATE TABLE slide_decks (
  id UUID PRIMARY KEY,
  project_id UUID REFERENCES projects(id),
  version TEXT NOT NULL,
  title TEXT NOT NULL,
  content JSONB NOT NULL,                   -- Full SlideDeck object
  created_at TIMESTAMPTZ DEFAULT NOW(),
  updated_at TIMESTAMPTZ DEFAULT NOW(),
  created_by TEXT
);

-- Document versions (history)
CREATE TABLE document_versions (
  id SERIAL PRIMARY KEY,
  document_id UUID REFERENCES documents(id) ON DELETE CASCADE,
  version_number INTEGER NOT NULL,
  content JSONB NOT NULL,
  created_at TIMESTAMPTZ DEFAULT NOW(),
  created_by TEXT,
  change_summary TEXT
);

CREATE INDEX idx_document_versions_doc ON document_versions(document_id, version_number DESC);

-- Assets (references to MinIO)
CREATE TABLE assets (
  id UUID PRIMARY KEY,
  project_id UUID REFERENCES projects(id),
  asset_type TEXT NOT NULL,                 -- 'image', 'chart-render', 'video', etc.
  mime_type TEXT NOT NULL,
  storage_key TEXT NOT NULL,                -- MinIO object key
  size_bytes BIGINT,
  created_at TIMESTAMPTZ DEFAULT NOW(),
  metadata JSONB
);
```

### Asset Storage (MinIO)

- **Bucket:** `narraciones-assets`
- **Key pattern:** `projects/{project_id}/assets/{asset_id}.{ext}`
- **Metadata:** Store MIME type, original filename, dimensions (for images), etc.
- **Lifecycle:** Implement orphan cleanup job to remove unreferenced assets.

---

## Rendering Pipelines

### Export Flow

1. **Input:** Document or SlideDeck (IKAM JSON)
2. **Validation:** Pydantic models validate structure
3. **Resolution:** Resolve references (blocks, assets, data bindings)
4. **Transformation:** Apply theme/tokens; resolve inline styles
5. **Rendering:** Target-specific renderer (PPTX, HTML, PDF, etc.)
6. **Output:** Binary file or URL to rendered asset

### Renderers

| Target | Library/Tool | Notes |
|--------|--------------|-------|
| HTML | Jinja2 + custom templates | Responsive; can embed Vega-Lite charts as interactive |
| PPTX | `python-pptx` | Map SLIDE elements to shapes/text boxes; embed images/charts |
| Google Slides | Google Slides API | Batch update requests; pre-render charts to images |
| PDF | Playwright or weasyprint | Print HTML or deck |
| Markdown | Custom exporter | Flatten Document blocks to CommonMark; skip layout |
| Notion | Notion API | Map blocks to Notion block types (best-effort) |

### Chart Rendering

- Use **Vega-Lite** for chart specs (stored in `ChartBlock.spec`).
- Pre-render to PNG/SVG via:
  - Node.js: `vega-lite` + `vega` + `canvas` or `sharp`
  - Python: `altair` + `altair_saver` (uses Node internally) or `vl-convert`
- Store rendered assets in MinIO; reference by `renderAssetId` in `ChartElement`.

---

## Validation & Type Safety

### Pydantic Models (Python)

Define schemas in `packages/ikam/src/ikam/`:

```
ikam/
  __init__.py
  document.py       # Document, Block types
  slide.py          # SlideDeck, Element types
  theme.py          # Theme, DesignTokens
  validators.py     # Custom validators
```

**Example:**

```python
from pydantic import BaseModel, Field
from typing import Literal, Union, List, Optional
from datetime import datetime

class DocumentMeta(BaseModel):
    title: str
    description: Optional[str] = None
    authors: Optional[List[str]] = None
    tags: Optional[List[str]] = None
    created_at: datetime
    updated_at: datetime
    project_id: Optional[str] = None
    locale: Optional[str] = "en-US"

class ParagraphBlock(BaseModel):
    type: Literal["paragraph"]
    id: str
    content: List[dict]  # InlineContent
    alignment: Optional[Literal["left", "center", "right", "justify"]] = "left"

# ... other block types

Block = Union[ParagraphBlock, HeadingBlock, TableBlock, ChartBlock, ...]

class Document(BaseModel):
    id: str
    version: str = "1.0.0"
    type: Literal["document"]
    meta: DocumentMeta
    blocks: List[Block]
```

### JSON Schema (for frontend/TS)

Generate JSON Schema from Pydantic models:

```bash
python packages/ikam/tools/generate_types.py
```

---

## Diff & Collaboration

### Block-Level Diffing

- Each block has a stable `id`.
- Store granular history per block in `document_versions`.
- Compute diffs using JSON Patch (RFC 6902) or custom block diff logic.

### Merge Strategy

- **Optimistic concurrency:** Use `updated_at` timestamp + version number.
- **Server-side merge:** On conflict, apply operational transform or CRDT-style merge.
- **Manual resolution:** UI presents conflicting changes for user decision.

### Presence & Real-Time Sync (Future)

- Use WebSocket or SSE for live cursors and collaborative editing.
- Sync block-level changes via operational transforms (OT) or CRDTs (e.g., Yjs).

---

## Examples

### Minimal Document

```json
{
  "id": "doc-001",
  "version": "1.0.0",
  "type": "document",
  "meta": {
    "title": "Q4 Revenue Analysis",
    "authors": ["analyst@example.com"],
    "created_at": "2025-10-27T10:00:00Z",
    "updated_at": "2025-10-27T10:00:00Z",
    "project_id": "proj-abc"
  },
  "blocks": [
    {
      "type": "heading",
      "id": "h1",
      "level": 1,
      "content": [{ "text": "Q4 Revenue Analysis" }]
    },
    {
      "type": "paragraph",
      "id": "p1",
      "content": [
        { "text": "Revenue grew " },
        { "text": "25%", "marks": [{ "type": "bold" }] },
        { "text": " year-over-year." }
      ]
    },
    {
      "type": "table",
      "id": "tbl1",
      "schema": {
        "columns": [
          { "key": "metric", "label": "Metric", "type": "string" },
          { "key": "value", "label": "Value", "type": "currency", "format": "$0,0" }
        ]
      },
      "rows": [
        { "metric": "ARR", "value": 1000000 },
        { "metric": "MRR", "value": 83333 }
      ]
    }
  ]
}
```

### Minimal Slide Deck

```json
{
  "id": "deck-001",
  "version": "1.0.0",
  "type": "slide-deck",
  "meta": {
    "title": "Investor Pitch",
    "created_at": "2025-10-27T10:00:00Z",
    "updated_at": "2025-10-27T10:00:00Z",
    "project_id": "proj-abc"
  },
  "theme": {
    "id": "theme-default",
    "name": "Narraciones Brand",
    "tokens": {
      "colors": {
        "primary": "#2E5D4E",
        "accent": "#C89968",
        "background": "#FAFAF9",
        "text": "#1C1C1A"
      },
      "fonts": {
        "heading": { "family": "Urbanist", "weight": 700, "lineHeight": 1.2 },
        "body": { "family": "system-ui", "weight": 400, "lineHeight": 1.5 }
      },
      "spacing": { "sm": 8, "md": 16, "lg": 32 }
    },
    "masters": []
  },
  "slides": [
    {
      "id": "slide-1",
      "elements": [
        {
          "type": "text",
          "id": "txt1",
          "frame": { "x": 0.1, "y": 0.3, "width": 0.8, "height": 0.4 },
          "content": [
            { "text": "Investor Pitch", "marks": [{ "type": "bold" }] }
          ],
          "style": {
            "fontSize": 48,
            "alignment": "center",
            "color": "#2E5D4E"
          }
        }
      ]
    }
  ]
}
```

---

## Next Steps for Implementation

1. **Define Pydantic models** in `packages/ikam/src/ikam/`
2. **Add database migrations** for `documents`, `slide_decks`, `document_versions`, `assets` tables
3. **Build basic HTML renderer** for Document blocks
4. **Implement PPTX exporter** using `python-pptx` for SlideDeck
5. **Add Vega-Lite chart rendering** pipeline (Node worker or Python vl-convert)
6. **Create API endpoints**:
   - `POST /api/knowledge_base` — create/update document uploads
   - `GET /api/knowledge_base/{id}` — retrieve document source
   - `POST /api/knowledge_base/{id}/render` — submit render job (HTML, PDF, etc.)
   - `POST /api/slide-decks` — create/update deck
   - `POST /api/slide-decks/{id}/export` — submit export job (PPTX, Google Slides)
7. **Wire render jobs** into existing jobs/events Kafka pipeline
8. **Add contract tests** for each renderer (HTML, PPTX, Markdown)
9. **Build UI components** for block editing (rich text, table editor, chart builder)
10. **Implement version history** and diff UI

---

## References

- **CommonMark:** https://commonmark.org/
- **Vega-Lite:** https://vega.github.io/vega-lite/
- **KaTeX:** https://katex.org/
- **ProseMirror (schema inspiration):** https://prosemirror.net/
- **Notion API:** https://developers.notion.com/
- **python-pptx:** https://python-pptx.readthedocs.io/
- **JSON Patch:** https://jsonpatch.com/

---

_This specification is a living document. Update version number and changelog when schemas evolve._
