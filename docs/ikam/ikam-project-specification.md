# IKAM Project Specification

_Version 1.0.0 | October 2025_

## Overview

The **IKAM Project** is Narraciones' canonical representation of a complete venture project within the Internal Knowledge Artifact Model (IKAM) ecosystem. It serves as the orchestration layer that ties together all knowledge artifacts, models, and media into a cohesive, queryable project graph.

IKAM Project is **one of the IKAM specifications** (alongside IKAM Document, IKAM Slide Deck, and IKAM Sheet) and provides:
1. **Project Graph** — relationships between artifacts, models, and media
2. **Artifact Registry** — typed catalog of all project knowledge
3. **Derivation Chains** — explicit lineage from source to derived artifacts
4. **Multi-Model Orchestration** — coordinates economic, story, and custom models
5. **Chat-Driven Editing** — natural language interface to project operations

**⚠️ Semantic Evaluation Principle:**
Artifact registration and typing in the IKAM Project rely on **semantic interpretation as a mandatory core feature**. Artifact types (IKAM Documents, Slide Decks, Sheets, etc.) are inferred from content structure, metadata, and semantic analysis—not from hardcoded type checks or strict schema predicates. All artifact classification must use semantic matching (content inspection, embedding similarity, LLM-based classification). Missing semantic infrastructure is a fatal error. See root `AGENTS.md` and `docs/testing/SEQUENCING_FRAMEWORK_PERSONAS_SPEC.md`.

---

## Design Principles

1. **Graph-First** — projects are graphs of artifacts with typed edges
2. **Explicit Lineage** — derived artifacts reference their sources with transformation metadata
3. **Typed Artifacts** — every artifact has a well-defined schema (IKAM, custom, or external)
4. **Polyglot Storage** — economic models, stories, documents, and media coexist
5. **Chat-Native** — natural language operations are first-class citizens
6. **Version-Aware** — all artifacts are versioned; project snapshots capture the full graph

---

## Core Schema

### Project Root

```typescript
interface Project {
  id: string;                          // UUID
  version: string;                     // IKAM Project schema version (semver, e.g., "1.0.0")
  type: "project";
  meta: ProjectMeta;
  artifacts: ArtifactRegistry;
  models: ModelRegistry;
  media: MediaRegistry;
  derivations: DerivationGraph;
  snapshots: SnapshotRegistry;
}

interface ProjectMeta {
  name: string;
  description?: string;
  industry?: string;                   // e.g., "SaaS", "Marketplace", "Hardware"
  stage?: string;                      // e.g., "Idea", "Seed", "Series A"
  team: TeamMember[];
  tags?: string[];
  createdAt: string;                   // ISO 8601
  updatedAt: string;
  settings?: ProjectSettings;
}

interface TeamMember {
  userId: string;
  role: "owner" | "editor" | "viewer";
  joinedAt: string;
}

interface ProjectSettings {
  defaultCurrency?: string;            // ISO 4217 code (e.g., "USD")
  defaultLocale?: string;              // e.g., "en-US"
  fiscalYearStart?: string;            // Month-day (e.g., "01-01", "04-01")
  theme?: ThemeReference;              // Reference to IKAM theme
}
```

---

## Artifact Registry

The artifact registry catalogs all knowledge artifacts in the project. Artifacts can be:
- **IKAM Documents** — semantic documents (reports, analyses, memos)
- **IKAM Slide Decks** — presentations (pitch decks, investor updates)
- **IKAM Sheets** — spreadsheets with formulas, charts, and analytics
- **Economic Models** — financial projections and unit economics
- **Story Models** — narrative structures and slide plans
- **External Documents** — uploaded PDFs, spreadsheets, research papers
- **Media Assets** — images, videos, logos, brand assets

```typescript
interface ArtifactRegistry {
  documents: Record<string, DocumentArtifact>;
  slideDecks: Record<string, SlideDeckArtifact>;
  sheets: Record<string, SheetArtifact>;
  economicModels: Record<string, EconomicModelArtifact>;
  storyModels: Record<string, StoryModelArtifact>;
  externalDocuments: Record<string, ExternalDocumentArtifact>;
}

interface BaseArtifact {
  id: string;                          // UUID
  type: string;                        // Discriminator
  name: string;
  description?: string;
  status: "draft" | "review" | "final" | "archived";
  createdAt: string;
  updatedAt: string;
  createdBy: string;                   // userId
  tags?: string[];
  versionHistory: VersionRef[];
}

interface DocumentArtifact extends BaseArtifact {
  type: "document";
  contentRef: IKAMDocumentRef;         // Reference to IKAM document storage
  format: "ikam";
}

interface SlideDeckArtifact extends BaseArtifact {
  type: "slideDeck";
  contentRef: IKAMSlideDeckRef;        // Reference to IKAM slide deck storage
  format: "ikam";
  theme?: ThemeReference;
}

interface SheetArtifact extends BaseArtifact {
  type: "sheet";
  contentRef: IKAMSheetRef;            // Reference to IKAM sheet (workbook) storage
  format: "ikam";
  sheets: string[];                    // Array of sheet names within workbook
}

interface EconomicModelArtifact extends BaseArtifact {
  type: "economicModel";
  contentRef: EconomicModelRef;        // Reference to economic model storage
  inputs: EconomicInputSchema;         // Schema for model inputs
  outputs: EconomicOutputSchema;       // Schema for calculated outputs
}

interface StoryModelArtifact extends BaseArtifact {
  type: "storyModel";
  contentRef: StoryModelRef;           // Reference to story model storage
  slides: SlideSchema[];               // Slide structure metadata
}

interface ExternalDocumentArtifact extends BaseArtifact {
  type: "externalDocument";
  contentRef: ExternalStorageRef;      // MinIO or S3 reference
  mimeType: string;                    // e.g., "application/pdf", "image/png"
  fileSize: number;                    // Bytes
  originalFilename: string;
}

interface VersionRef {
  versionId: string;
  timestamp: string;
  label?: string;
  createdBy: string;
}
```

---

## Model Registry

Models are specialized artifacts that expose computational interfaces (inputs → transformations → outputs).

```typescript
interface ModelRegistry {
  economic: Record<string, EconomicModelRef>;
  story: Record<string, StoryModelRef>;
  custom: Record<string, CustomModelRef>;
}

interface EconomicModelRef {
  artifactId: string;                  // Links to ArtifactRegistry
  engine: "univer" | "excel" | "custom";
  inputs: Record<string, any>;         // Current input values
  outputs: Record<string, any>;        // Calculated outputs (cached)
  formulaGraph?: FormulaGraph;         // Optional dependency graph
}

interface StoryModelRef {
  artifactId: string;
  engine: "slides" | "notion" | "custom";
  slides: SlideRef[];
  narrative: NarrativeStructure;
}

interface CustomModelRef {
  artifactId: string;
  engine: string;                      // Plugin identifier
  schema: any;                         // Plugin-defined schema
}
```

---

## Media Registry

Media assets are images, videos, and files used across artifacts.

```typescript
interface MediaRegistry {
  images: Record<string, ImageAsset>;
  videos: Record<string, VideoAsset>;
  files: Record<string, FileAsset>;
}

interface ImageAsset {
  id: string;
  name: string;
  storageRef: string;                  // MinIO path or URL
  mimeType: string;                    // e.g., "image/png"
  width?: number;
  height?: number;
  fileSize: number;
  uploadedAt: string;
  uploadedBy: string;
  tags?: string[];
  altText?: string;                    // Accessibility
  usedIn: ArtifactUsage[];             // Reverse index
}

interface VideoAsset {
  id: string;
  name: string;
  storageRef: string;
  mimeType: string;                    // e.g., "video/mp4"
  duration?: number;                   // Seconds
  fileSize: number;
  uploadedAt: string;
  uploadedBy: string;
  tags?: string[];
  usedIn: ArtifactUsage[];
}

interface FileAsset {
  id: string;
  name: string;
  storageRef: string;
  mimeType: string;
  fileSize: number;
  uploadedAt: string;
  uploadedBy: string;
  tags?: string[];
  usedIn: ArtifactUsage[];
}

interface ArtifactUsage {
  artifactId: string;
  artifactType: string;
  blockId?: string;                    // For IKAM documents
  slideId?: string;                    // For slide decks
}
```

---

## Derivation Graph

The derivation graph tracks how artifacts are derived from other artifacts. This enables:
- **Impact Analysis** — "What breaks if I change the economic model?"
- **Regeneration** — "Re-render all investor decks from updated story"
- **Provenance** — "Where did this chart come from?"

```typescript
interface DerivationGraph {
  nodes: Record<string, DerivationNode>;
  edges: DerivationEdge[];
}

interface DerivationNode {
  artifactId: string;
  artifactType: string;
  isSource: boolean;                   // True for manually created artifacts
  isDerived: boolean;                  // True for auto-generated artifacts
}

interface DerivationEdge {
  id: string;
  sourceId: string;                    // Artifact ID
  targetId: string;                    // Derived artifact ID
  derivationType: DerivationType;
  transformation?: TransformationSpec;
  createdAt: string;
}

type DerivationType =
  | "embed"                            // Chart embedded in slide
  | "export"                           // PDF exported from deck
  | "generate"                         // Story generated from economic model
  | "transform"                        // Custom transformation
  | "reference";                       // Simple reference

interface TransformationSpec {
  engine: string;                      // e.g., "vega-lite", "python-pptx", "gpt-4"
  version: string;
  parameters?: Record<string, any>;
  script?: string;                     // Optional transformation code
}
```

---

## Snapshot Registry

Project snapshots capture the entire project state at a point in time.

```typescript
interface SnapshotRegistry {
  snapshots: Record<string, ProjectSnapshot>;
}

interface ProjectSnapshot {
  id: string;
  label?: string;
  timestamp: string;
  createdBy: string;
  projectState: {
    meta: ProjectMeta;
    artifactVersions: Record<string, string>;  // artifactId → versionId
    modelVersions: Record<string, string>;     // modelId → versionId
    mediaVersions: Record<string, string>;     // mediaId → versionId
  };
  changes?: SnapshotDiff;              // Diff from previous snapshot
}

interface SnapshotDiff {
  artifactsAdded: string[];
  artifactsModified: string[];
  artifactsDeleted: string[];
  modelsModified: string[];
  mediaAdded: string[];
  mediaDeleted: string[];
}
```

---

## Chat-Based Operations

IPM defines a structured instruction schema for chat-based editing. All natural language instructions are parsed into typed operations.

### Instruction Schema

```typescript
interface ProjectInstruction {
  id: string;                          // UUID
  timestamp: string;
  userId: string;
  naturalLanguage: string;             // Original user input
  parsed: ParsedOperation;
  status: "pending" | "executing" | "complete" | "failed";
  result?: OperationResult;
  error?: string;
}

interface ParsedOperation {
  intent: OperationIntent;
  targets: OperationTarget[];
  parameters: Record<string, any>;
  confidence: number;                  // 0.0-1.0 (LLM confidence)
}

type OperationIntent =
  // Artifact operations
  | "create_artifact"
  | "update_artifact"
  | "delete_artifact"
  | "rename_artifact"
  | "duplicate_artifact"
  // Model operations
  | "update_economic_input"
  | "recalculate_model"
  | "generate_story"
  | "add_offering"
  | "remove_offering"
  // Media operations
  | "upload_media"
  | "replace_media"
  | "delete_media"
  // Derivation operations
  | "export_artifact"
  | "generate_derived"
  | "refresh_derived"
  // Project operations
  | "create_snapshot"
  | "restore_snapshot"
  | "update_project_meta";

interface OperationTarget {
  type: "artifact" | "model" | "media" | "project";
  id?: string;                         // For updates/deletes
  selector?: string;                   // For creation or bulk ops
}

interface OperationResult {
  success: boolean;
  affectedArtifacts: string[];
  affectedModels: string[];
  affectedMedia: string[];
  derivationsTriggered: string[];
  snapshotCreated?: string;
}
```

### Example Instructions

**Create Economic Model:**
```json
{
  "naturalLanguage": "Create a new SaaS economic model with $50 MRR and 25% churn",
  "parsed": {
    "intent": "create_artifact",
    "targets": [{ "type": "artifact", "selector": "economic_model" }],
    "parameters": {
      "template": "saas",
      "inputs": {
        "mrr": 50,
        "churnRate": 0.25
      }
    },
    "confidence": 0.92
  }
}
```

**Update Revenue Growth:**
```json
{
  "naturalLanguage": "Update revenue growth to 30% year-over-year",
  "parsed": {
    "intent": "update_economic_input",
    "targets": [{ "type": "model", "id": "econ-001" }],
    "parameters": {
      "input": "revenueGrowthRate",
      "value": 0.30
    },
    "confidence": 0.95
  }
}
```

**Generate Investor Deck:**
```json
{
  "naturalLanguage": "Generate a pitch deck from the economic model",
  "parsed": {
    "intent": "generate_derived",
    "targets": [
      { "type": "model", "id": "econ-001" },
      { "type": "artifact", "selector": "slideDeck" }
    ],
    "parameters": {
      "template": "investor_pitch",
      "derivationType": "generate"
    },
    "confidence": 0.88
  }
}
```

---

## Storage Schema (PostgreSQL)

### Projects Table

```sql
CREATE TABLE projects (
  id TEXT PRIMARY KEY,
  version TEXT NOT NULL,                          -- IKAM Project schema version
  meta JSONB NOT NULL,                            -- ProjectMeta
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_projects_updated_at ON projects(updated_at);
CREATE INDEX idx_projects_team ON projects USING gin((meta->'team'));
```

### Artifacts Table

```sql
CREATE TABLE artifacts (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  type TEXT NOT NULL,                             -- artifact type discriminator
  name TEXT NOT NULL,
  description TEXT,
  status TEXT NOT NULL DEFAULT 'draft',
  content_ref JSONB NOT NULL,                     -- Reference to content storage
  meta JSONB,                                     -- Type-specific metadata
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  created_by TEXT NOT NULL,
  tags TEXT[]
);

CREATE INDEX idx_artifacts_project ON artifacts(project_id);
CREATE INDEX idx_artifacts_type ON artifacts(type);
CREATE INDEX idx_artifacts_status ON artifacts(status);
CREATE INDEX idx_artifacts_tags ON artifacts USING gin(tags);
```

### Models Table

```sql
CREATE TABLE models (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  artifact_id TEXT REFERENCES artifacts(id) ON DELETE CASCADE,
  type TEXT NOT NULL,                             -- "economic", "story", "custom"
  engine TEXT NOT NULL,
  inputs JSONB NOT NULL DEFAULT '{}',
  outputs JSONB NOT NULL DEFAULT '{}',
  schema JSONB,                                   -- Input/output schemas
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_models_project ON models(project_id);
CREATE INDEX idx_models_artifact ON models(artifact_id);
CREATE INDEX idx_models_type ON models(type);
```

### Media Table

```sql
CREATE TABLE media (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  storage_ref TEXT NOT NULL,                      -- MinIO path
  mime_type TEXT NOT NULL,
  file_size INTEGER NOT NULL,
  meta JSONB,                                     -- Type-specific metadata (width, height, duration)
  uploaded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  uploaded_by TEXT NOT NULL,
  tags TEXT[],
  used_in JSONB DEFAULT '[]'                      -- Array of ArtifactUsage
);

CREATE INDEX idx_media_project ON media(project_id);
CREATE INDEX idx_media_mime_type ON media(mime_type);
CREATE INDEX idx_media_tags ON media USING gin(tags);
```

### Derivations Table

```sql
CREATE TABLE derivations (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  source_id TEXT NOT NULL,                        -- Artifact/model/media ID
  target_id TEXT NOT NULL,                        -- Derived artifact ID
  derivation_type TEXT NOT NULL,
  transformation JSONB,                           -- TransformationSpec
  created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_derivations_project ON derivations(project_id);
CREATE INDEX idx_derivations_source ON derivations(source_id);
CREATE INDEX idx_derivations_target ON derivations(target_id);
```

### Snapshots Table

```sql
CREATE TABLE project_snapshots (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  label TEXT,
  project_state JSONB NOT NULL,                   -- Full snapshot payload
  changes JSONB,                                  -- SnapshotDiff
  timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  created_by TEXT NOT NULL
);

CREATE INDEX idx_snapshots_project ON project_snapshots(project_id);
CREATE INDEX idx_snapshots_timestamp ON project_snapshots(timestamp);
```

### Instructions Table

```sql
CREATE TABLE project_instructions (
  id TEXT PRIMARY KEY,
  project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
  user_id TEXT NOT NULL,
  natural_language TEXT NOT NULL,
  parsed JSONB NOT NULL,                          -- ParsedOperation
  status TEXT NOT NULL DEFAULT 'pending',
  result JSONB,                                   -- OperationResult
  error TEXT,
  timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_instructions_project ON instructions(project_id);
CREATE INDEX idx_instructions_user ON instructions(user_id);
CREATE INDEX idx_instructions_status ON instructions(status);
CREATE INDEX idx_instructions_timestamp ON instructions(timestamp);
```

---

## Implementation Roadmap

### Phase 1: IKAM Project Foundation (Sprint 3)
- [ ] Define Pydantic models for IKAM Project core schema (`Project`, `ArtifactRegistry`, `ModelRegistry`, `MediaRegistry`)
- [ ] Add database migrations for IKAM Project tables
- [ ] Generate TypeScript types from Pydantic models
- [ ] Add contract tests for IKAM Project schema validation
- [ ] Implement basic artifact CRUD (create, read, update, delete)

**Exit Criteria:** IKAM Project structure validated; can create projects with artifacts and models.

### Phase 2: Derivation Graph (Sprint 4)
- [ ] Implement derivation graph storage and queries
- [ ] Add impact analysis API (`GET /api/projects/{id}/derivations/impact?artifactId={id}`)
- [ ] Create derivation tracking for economic → story generation
- [ ] Add regeneration API (`POST /api/projects/{id}/derivations/regenerate`)
- [ ] Build derivation visualization UI component

**Exit Criteria:** Derivation graph tracks relationships; impact analysis works; can regenerate derived artifacts.

### Phase 3: Chat-Based Editing (Sprint 5)
- [ ] Implement instruction parser (natural language → `ParsedOperation`)
- [ ] Add instruction executor (dispatch operations to model/artifact services)
- [ ] Create instruction history API (`GET /api/projects/{id}/instructions`)
- [ ] Build chat UI with instruction feedback loop
- [ ] Add SSE streaming for instruction progress

**Exit Criteria:** Users can submit natural language instructions; operations execute correctly; progress streamed.

### Phase 4: IKAM Document, Slide & Sheet Integration (Sprint 6)
- [ ] Migrate existing IKAM documents to IKAM Project artifact registry
- [ ] Add IKAM export derivations (document → PDF, slide deck → PPTX)
- [ ] Implement IKAM block embedding in economic models
- [ ] Create unified export API (`POST /api/projects/{id}/export`)
- [ ] Add version history for IKAM artifacts

**Exit Criteria:** IKAM documents integrated into IKAM Project; export pipelines functional; version history works.

### Phase 5: Advanced Features (Sprint 7)
- [ ] Implement project snapshots with diff visualization
- [ ] Add smart templates (AI-suggested templates based on project context)
- [ ] Create project search (full-text search across artifacts)
- [ ] Build project analytics dashboard (artifact usage, derivation health)
- [ ] Add collaboration features (presence, comments on artifacts)

**Exit Criteria:** Snapshots capture full project state; search works; analytics provide insights; collaboration functional.

---

## Success Metrics

- **Project Graph Complete:** All artifacts, models, and media tracked with relationships
- **Derivation Chains Functional:** Impact analysis accurate; regeneration works
- **Chat Interface Usable:** 80%+ instruction success rate; <2s response time
- **IKAM Integration Seamless:** All IKAM features accessible via IKAM Project APIs
- **Version History Reliable:** Snapshots capture state; diffs accurate; restore works
- **Performance Acceptable:** Project load <500ms; instruction parse <1s; regeneration <5s

---

## Appendix: IKAM Ecosystem

The **IKAM (Internal Knowledge Artifact Model)** ecosystem consists of four complementary specifications:

### IKAM Project
- **Scope:** Entire project with all artifacts, models, and media
- **Purpose:** Orchestration, lineage, chat interface, project graph
- **Storage:** PostgreSQL + MinIO
- **Consumers:** Frontend project views, chat interface, export pipelines

### IKAM Document  
- **Scope:** Semantic content as typed block tree (paragraphs, headings, lists, tables, charts)
- **Purpose:** Rich documents with multi-target export (HTML, PDF, Markdown, Notion)
- **Storage:** PostgreSQL JSONB
- **Consumers:** Document editor, HTML renderer, PDF exporter

### IKAM Slide Deck
- **Scope:** Canvas-style presentations with positioned elements
- **Purpose:** Pitch decks and investor presentations with PPTX/Google Slides export
- **Storage:** PostgreSQL JSONB
- **Consumers:** Slide editor, PPTX exporter, Google Slides API

### IKAM Sheet
- **Scope:** Spreadsheets with formulas, charts, and pivot tables
- **Purpose:** Financial modeling, data analysis, scenario planning with Excel/Google Sheets export
- **Storage:** PostgreSQL (sparse cell storage) + MinIO for embedded charts
- **Consumers:** Sheet editor, calculation engine, XLSX exporter

**Relationship:**  
IKAM Project provides the orchestration layer that ties together Documents, Slide Decks, and Sheets with economic models, story models, and media assets. All four specifications work together to form a complete knowledge management system for venture teams.

---

## References

- [IKAM Document/Slide Specification](./ikam-specification.md)
- [IKAM Sheet Specification](./ikam-sheet-specification.md)
- [Restructure Roadmap](./restructure-roadmap.md) Section 7: IKAM Implementation
