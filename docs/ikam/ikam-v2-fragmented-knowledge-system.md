# IKAM v2: Fragmented Knowledge System

**Version:** 2.0.0-draft  
**Status:** Design Exploration (November 2025)  
**Phase:** Phase 2 Integration Planning  

**2026-01 Update (Direction Change):** The original “L0 = core essence, L1 = supporting, L2 = detail” framing is too rigid for a semantic-first system. The current direction is **semantic-first, ad-hoc decomposition**: decomposition shape is driven by artifact structure + semantic intent, and “partial rendering” is policy-based (salience/relevance/view), not “render levels 0..2”.

Canonical policy doc: [SEMANTIC_FIRST_DECOMPOSITION.md](./SEMANTIC_FIRST_DECOMPOSITION.md)

---

## ⚠️ CRITICAL: Mathematical Foundations

IKAM v2's design is grounded in **rigorous mathematical proofs** that guarantee:

1. **Storage Gains Monotonicity** ([STORAGE_GAINS_EXAMPLE.md](STORAGE_GAINS_EXAMPLE.md))
   - Δ(N) = S_flat(N) − S_IKAM(N) increases monotonically with N outputs
   - Break-even at N* ≈ 2-3 outputs for typical scenarios
   - Example: 3 outputs → 32.7% storage reduction (5.34 KB saved)

2. **Fisher Information Dominance** ([FISHER_INFORMATION_GAINS.md](FISHER_INFORMATION_GAINS.md))
   - I_IKAM(θ) ≥ I_RAG(θ) + Δ_provenance(N)
   - Expected: ≥15% lower parameter estimation RMSE vs traditional RAG
   - Expected: ≥25% lower output variance (consistency)
   - Expected: ≥30% fewer feedback iterations to converge

3. **Lossless Reconstruction Guarantee**
   - reconstruct(decompose(A)) = A (byte-level equality)
   - All provenance relationships explicitly stored and observable
   - Deterministic algorithms only (no probabilistic compression)

**Implementation Requirements:**
- ✅ Round-trip tests with 100% pass rate (byte-level equality)
- ✅ Provenance completeness tests (all derivations traceable)
- ✅ CAS deduplication validation (identical content → same hash)
- ✅ Information metric regression tests (Fisher information non-decreasing)

**See Section 11 for formal guarantees and testing strategy.**

---

## Executive Summary

**How this doc fits with current implementation planning:**
- This document is a conceptual design exploration.
- The current operator-level implementation plan (grounding retrieval, relational validation, draft overlays, projection/replay) is captured in [IKAM Graph Algorithms Plan](./IKAM_GRAPH_ALGORITHMS_PLAN.md).
- IKAM uses a two-layer model: **storage fragments** (CAS bytes) + **domain fragments** (materialized typed objects). Artifacts may include both atomic content and **relational fragments** (connections + descriptions) as renderable objects.
- Interaction is semantic-first: user intent is interpreted to generate an operator plan (no hardcoded persona/artifact switches; missing semantic infrastructure is a fatal configuration error).

IKAM v2 introduces a **fragmented knowledge system** that decomposes artifacts into hierarchical fragments, enabling:

1. **Multi-level representation:** Core ideas at the lowest level, supplementary ideas at higher levels
2. **Deterministic reproduction:** Full render path ensures exact artifact reconstruction
3. **Non-deterministic representation:** Partial level rendering allows flexible abstraction
4. **Cross-format translation:** Seamless transformation between documents, slides, and sheets
5. **Salience-based storage:** Cache-like layering optimizes hot/warm/cold fragment access
6. **Provenance tracking:** Explicit relationships enable superior parameter estimation (see FISHER_INFORMATION_GAINS.md)

The system builds on IKAM v1's foundation (abstract data model, multiple serializations, rendering pipelines) and introduces three new core concepts:

- **Fragments:** Recursive artifact units representing ideas at different abstraction levels
- **Radicals:** Instructions for rendering fragments into concrete artifacts
- **Forja Package:** Decomposition/reconstruction engine for fragment operations

---

## 1. Conceptual Architecture

### 1.1 Fragments: Multi-Level Artifact Representation

Fragments are the atomic units of knowledge in IKAM v2. Each fragment:

- **Is itself an artifact** (recursive model): a fragment can be rendered, stored, versioned, and derived from
- **Represents an idea at a specific abstraction level:** Level 0 = core essence, Level N = full elaboration
- **Maintains deterministic render paths:** Full path from L0→L1→...→Ln guarantees reproducibility
- **Supports partial rendering:** Rendering only L0→L2 produces a valid but simplified artifact

**Hierarchical Structure:**

```
Artifact (e.g., Economic Model Document)
├── Fragment L0: Core Value Proposition
│   └── data: { "value": "marketplace connects X with Y" }
├── Fragment L1: Business Model Canvas
│   ├── parent: Fragment L0
│   └── data: { "segments": [...], "channels": [...], "revenue": [...] }
├── Fragment L2: Unit Economics Detail
│   ├── parent: Fragment L1
│   └── data: { "cac": {...}, "ltv": {...}, "margins": {...} }
└── Fragment L3: Full Financial Model
    ├── parent: Fragment L2
    └── data: { "revenue_forecast": [...], "cash_flow": [...], "balance_sheet": [...] }
```

**Properties:**

```typescript
interface Fragment {
  id: string;
  artifactId: string;           // Parent artifact this fragment belongs to
  level: number;                // Abstraction level (0 = core, higher = more detail)
  parentFragmentId?: string;    // Fragment at level-1 (null for L0)
  type: "text" | "data" | "visual" | "structural";
  content: FragmentContent;     // Type-specific content model
  radicalRefs: RadicalRef[];    // Instructions for rendering this fragment
  salience: number;             // 0.0-1.0 importance score for storage layering
  createdAt: string;
  updatedAt: string;
}

interface FragmentContent {
  // Base fields common to all fragment types
  summary: string;              // Human-readable summary of this fragment's idea
  
  // Type-specific fields (discriminated union in implementation)
  // text: { blocks: Block[] }
  // data: { schema: Schema, rows: any[] }
  // visual: { chartSpec: VegaLiteSpec, imageRef?: string }
  // structural: { layout: LayoutSpec, children: FragmentRef[] }
}
```

### 1.2 Radicals: Rendering Instructions

Radicals are **rendering instructions** that transform fragments into concrete artifacts. They specify:

- **Input:** Which fragment(s) to render
- **Target format:** Document, slide, sheet, or external format (PDF, PPTX, HTML)
- **Rendering parameters:** Theme, layout, styling, data bindings
- **Output constraints:** Size limits, accessibility requirements, performance budgets

**Example Radical (Conceptual):**

```json
{
  "id": "radical-001",
  "name": "Fragment to Document Section",
  "inputFragmentLevels": [0, 1, 2],
  "targetFormat": "ikam-document",
  "template": {
    "type": "section",
    "heading": { "fromFragment": 0, "field": "summary" },
    "body": { "fromFragment": 1, "field": "content.blocks" },
    "details": { "fromFragment": 2, "field": "content.blocks" }
  },
  "parameters": {
    "theme": "default",
    "maxDepth": 3,
    "includeCharts": true
  }
}
```

**Radical Model:**

```typescript
interface Radical {
  id: string;
  name: string;
  description?: string;
  inputSpec: RadicalInputSpec;
  outputFormat: "ikam-document" | "ikam-slideDeck" | "ikam-sheet" | "pdf" | "pptx" | "html";
  template: RadicalTemplate;    // Rendering template (JSON or code reference)
  parameters: Record<string, any>;
  version: string;
}

interface RadicalInputSpec {
  fragmentLevels: number[];     // Which levels to consume (e.g., [0,1,2])
  minFragments?: number;        // Minimum fragments required
  fragmentTypes?: FragmentType[];  // Filter by type
}

interface RadicalTemplate {
  type: "json" | "jinja2" | "code";
  source: string;               // Template content or reference
}
```

### 1.3 Forja Package: Fragment Decomposition/Reconstruction

**Forja** (Spanish for "forge") is a new subpackage within `packages/ikam` responsible for:

1. **Decomposition:** Breaking artifacts into fragments at specified levels
2. **Reconstruction:** Assembling fragments back into artifacts using radicals
3. **Translation:** Converting fragments between formats (doc↔slide↔sheet)
4. **Validation:** Ensuring fragment integrity and render path consistency

**Package Structure:**

```
packages/ikam/src/ikam/
├── forja/
│   ├── __init__.py
│   ├── decomposer.py         # Artifact → Fragments
│   ├── reconstructor.py      # Fragments + Radicals → Artifact
│   ├── translator.py         # Fragment format conversion
│   ├── validator.py          # Fragment integrity checks
│   └── models.py             # Fragment/Radical Pydantic models
├── almacen/                  # Storage abstraction layer (NEW)
│   ├── __init__.py
│   ├── base.py               # Abstract storage interfaces
│   ├── registry.py           # Pluggable backend registry
│   ├── policy.py             # Tier labels + policy-based routing
│   ├── postgres.py           # PostgreSQL backend (reference)
│   ├── minio.py              # Object storage backend (reference)
│   └── models.py             # Storage metadata models (optional)
```

**Note:** The `almacen` package migrates document chunking/storage logic from `modelado` into IKAM, providing a clean abstraction for tiered storage independent of modeling concerns.

**Core APIs:**

```python
# Decomposition
def decompose_artifact(
    artifact: Artifact,
    target_levels: int,
    strategy: DecompositionStrategy = "semantic"
) -> List[Fragment]:
    """Break artifact into hierarchical fragments."""
    pass

# Reconstruction
def reconstruct_artifact(
    fragments: List[Fragment],
    radicals: List[Radical],
    target_format: str,
    render_levels: Optional[List[int]] = None  # None = all levels
) -> Artifact:
    """Assemble fragments into artifact using radicals."""
    pass

# Translation
def translate_fragments(
    fragments: List[Fragment],
    source_format: str,
    target_format: str,
    radical: Radical
) -> List[Fragment]:
    """Convert fragments from one format to another."""
    pass

# Storage abstraction (almacen package)
from ikam.almacen import (
  StorageBackend,
  BackendRegistry,
  TierPolicy,
  TieredStorage,
  Tier,
)

def store_fragment(
  fragment: Fragment,
  storage: TieredStorage,
  tier: Optional[str] = None  # If None, policy decides
) -> FragmentStorageRef:
  """Store fragment via policy-driven tier routing (engine-agnostic)."""
  pass

def retrieve_fragment(
  fragment_id: str,
  storage: TieredStorage,
) -> Fragment:
  """Retrieve fragment; policy may hint tier, engines are pluggable."""
  pass

def migrate_fragment_tier(
  fragment_id: str,
  source_tier: str,
  target_tier: str,
  storage: TieredStorage,
) -> None:
  """Migrate fragment between storage tiers (mapping is config-driven)."""
  pass
```

### 1.4 Salience-Based Storage (Almacén Package)

**Design Decision:** IKAM manages fragmentation instructions and artifact semantics, but delegates storage operations to a separate `almacen` (Spanish for "warehouse") subpackage. This provides clean separation between content modeling and storage infrastructure.

**Migration from Modelado:**
- Document chunking logic from `packages/modelado/src/modelado/documents/chunking.py` migrates to `almacen`
- Storage concerns decoupled from modeling concerns
- IKAM's `forja` focuses on fragment semantics; `almacen` handles storage tiers

Fragments are stored in **layered tiers** based on their salience score (importance).  
Tiers are logical labels (hot, warm, cold); the concrete storage engine per tier is
selected via configuration and can be swapped without code changes. Example mapping:

- hot → "postgresql" backend (reference)
- warm → "s3"/"minio" backend (reference)
- cold → "s3-archive" backend (reference)

These are examples only—the tier→engine mapping is pluggable and environment-specific.

**Storage Backend: PostgreSQL (Retained)**

**Decision:** Continue using PostgreSQL with pgvector extensions rather than switching to a document database or RocksDB.

**Rationale:**
- **Existing investment:** PostgreSQL already deployed with pgvector for embeddings
- **ACID guarantees:** Fragment versioning and derivation graphs benefit from transactional consistency
- **Mature tooling:** Alembic migrations, connection pooling, monitoring already in place
- **JSONB performance:** PostgreSQL JSONB with GIN indexes provides document-like flexibility with relational guarantees
- **Unified stack:** Single database for relational (projects, teams) and semi-structured (fragments) data

**Future considerations:**
- If hot tier becomes bottleneck, consider read replicas or caching layer (Redis)
- RocksDB embedding within PostgreSQL (via FDW) could support custom heap storage if needed
- Document DB migration deferred until evidence of PostgreSQL limitations

**Storage Model:**

```typescript
interface FragmentStorage {
  fragmentId: string;
  tier: "hot" | "warm" | "cold";
  location: StorageLocation;
  metadata: FragmentMetadata;
}

interface StorageLocation {
  type: string;              // e.g., "postgres", "minio", "s3", "fs"
  path: string;
  compressed?: boolean;
}

interface FragmentMetadata {
  size: number;               // Bytes
  lastAccessed: string;
  accessCount: number;
  salience: number;
  compressionRatio?: number;
}
```

**Storage Migration Rules:**

1. **Hot → Warm:** Fragment not accessed in 7 days and salience < 0.7
2. **Warm → Cold:** Fragment not accessed in 30 days or salience < 0.3
3. **Cold → Warm:** Access request triggers promotion
4. **Warm → Hot:** High access frequency (>10 accesses/day) and salience ≥ 0.5

---

### 1.5 Engine-Agnostic Tiering: Policy, Registry, Capabilities

To avoid coupling tiers to specific engines, IKAM defines a storage coordination layer with three components:

- BackendRegistry — runtime registry of pluggable storage backends (by name)
- TierPolicy — decides which logical tier (hot/warm/cold) to use for a given fragment and operation
- TieredStorage — orchestrates put/get/delete by consulting the policy and resolving the configured backend for each tier via the registry

Reference interface (see `packages/ikam/src/ikam/almacen/`):

```python
from ikam.almacen import (
  StorageBackend, BackendRegistry, TierPolicy, TieredStorage, Tier,
  FragmentKey, FragmentRecord, Capability,
)
```

Capabilities advertise optional features a backend supports:

- PUT, GET, DELETE, LIST — core ops
- VERSIONS, METADATA_QUERY — optional metadata and version history
- CAS — content-addressable addressing (e.g., blake3 IDs)

Example capability matrix (illustrative):

| Backend      | PUT | GET | DELETE | LIST | VERSIONS | METADATA_QUERY | CAS |
|--------------|-----|-----|--------|------|----------|----------------|-----|
| postgresql   | ✅  | ✅  | ✅     | ✅   | ✅       | ✅             | ✅  |
| s3/minio     | ✅  | ✅  | ✅     | ✅   | ❌       | ❌             | ✅  |
| filesystem   | ✅  | ✅  | ✅     | ✅   | ❌       | ❌             | ✅  |

Config-driven tier→backend mapping (YAML):

```yaml
storage:
  backends:
    postgresql:
      driver: ikam.almacen.backends.postgres:PostgresBackend
      dsn: ${DATABASE_URL}
    s3:
      driver: ikam.almacen.backends.minio:MinioBackend
      endpoint: http://minio:9000
      bucket: artifacts
      accessKey: ${MINIO_ACCESS_KEY}
      secretKey: ${MINIO_SECRET_KEY}
    fs:
      driver: ikam.almacen.backends.fs:FilesystemBackend
      root: /data/fragments
  tiers:
    hot: postgresql
    warm: s3
    cold: s3
```

Fallbacks are supported by mapping a tier to an ordered list: `hot: [postgresql, s3]`. `TieredStorage` will try backends in order for reads; writes use the first compatible backend based on capabilities.

Policy examples:

- SaliencePolicy: choose tier by salience thresholds (e.g., ≥0.7 → hot)
- SizePolicy: inline small payloads (REP) else CAS to object storage
- AccessFrequencyPolicy: demote after inactivity windows; promote on spikes
- CombinedPolicy: weighted logic with hysteresis to avoid oscillations

Wiring example (Python):

```python
registry = BackendRegistry()
registry.register(PostgresBackend(name="postgresql", dsn=...))
registry.register(MinioBackend(name="s3", endpoint=..., bucket=...))

policy = CombinedPolicy(
  salience_thresholds={Tier.HOT: 0.7, Tier.WARM: 0.3},
  size_inline_threshold=4096,
  inactivity_days={Tier.WARM: 7, Tier.COLD: 30},
)

ts = TieredStorage(
  registry=registry,
  tier_backend={Tier.HOT: "postgresql", Tier.WARM: "s3", Tier.COLD: "s3"},
  policy=policy,
)
```

## 2. Integration with IKAM v1

### 2.1 Backward Compatibility

IKAM v2 is **fully backward compatible** with v1:

- All v1 artifacts remain valid (documents, slides, sheets)
- v1 rendering pipelines continue to work unchanged
- Fragments are an **optional enhancement** layer on top of v1 models

**Migration Path:**

1. **Opt-in fragmentation:** Use `forja.decompose_artifact()` to generate fragments for existing artifacts
2. **Hybrid mode:** Store both v1 artifact and v2 fragments; serve based on client capabilities
3. **Full v2 mode:** All new artifacts created as fragments from the start

### 2.2 v1 Foundation Leverage

IKAM v2 builds on v1 components:

- **Block model:** Fragments reference v1 blocks (paragraph, heading, table, chart, etc.)
- **Rendering pipelines:** Radicals invoke v1 renderers (PPTX, HTML, PDF, Notion)
- **Storage patterns:** Object storage (e.g., S3/MinIO) or other engines extended with tier metadata; mapping is policy-driven
- **Derivation graph:** Fragments create new derivation types (fragment→artifact, fragment→fragment)

**Example Derivation:**

```json
{
  "id": "deriv-abc123",
  "sourceArtifactId": "fragment-L2-econ-001",
  "targetArtifactId": "doc-investor-memo",
  "derivationType": "fragment-reconstruction",
  "parameters": {
    "radicalId": "radical-doc-section",
    "renderLevels": [0, 1, 2]
  }
}
```

### 2.3 API Extensions

New endpoints in `base-api` for fragment operations:

```
POST   /api/model/fragments/decompose      # Decompose artifact → fragments
POST   /api/model/fragments/reconstruct    # Reconstruct fragments → artifact
POST   /api/model/fragments/translate      # Translate fragments between formats
GET    /api/model/fragments/{fragmentId}   # Get fragment by ID
PATCH  /api/model/fragments/{fragmentId}   # Update fragment content
GET    /api/model/artifacts/{id}/fragments # List fragments for artifact
POST   /api/model/radicals                 # Create/register radical
GET    /api/model/radicals                 # List available radicals
```

---

## 3. Use Cases & Examples

### 3.1 Deterministic Full Rendering

**Scenario:** Reproduce exact artifact from fragments

```python
# Load all fragments for artifact
fragments = get_fragments(artifact_id="doc-investor-memo")

# Reconstruct with full render path (all levels)
artifact = reconstruct_artifact(
    fragments=fragments,
    radicals=get_radicals(["doc-section", "chart-embed", "table-format"]),
    target_format="ikam-document",
    render_levels=None  # All levels
)

# Result: Bit-identical to original artifact
assert artifact == original_artifact
```

### 3.2 Non-Deterministic Partial Rendering

**Scenario:** Generate executive summary (L0-L1 only)

```python
# Reconstruct with partial levels
summary_doc = reconstruct_artifact(
    fragments=fragments,
    radicals=get_radicals(["doc-summary"]),
    target_format="ikam-document",
    render_levels=[0, 1]  # Core + high-level only
)

# Result: Valid document with simplified content
# Core value prop + business model, no detailed financials
```

### 3.3 Cross-Format Translation

**Scenario:** Convert document fragments to slide deck

```python
# Decompose document into fragments
doc_fragments = decompose_artifact(
    artifact=document,
    target_levels=3,
    strategy="semantic"
)

# Translate to slide-friendly fragments
slide_fragments = translate_fragments(
    fragments=doc_fragments,
    source_format="ikam-document",
    target_format="ikam-slideDeck",
    radical=get_radical("doc-to-slide")
)

# Reconstruct as slide deck
slides = reconstruct_artifact(
    fragments=slide_fragments,
    radicals=get_radicals(["slide-layout", "chart-slide", "table-slide"]),
    target_format="ikam-slideDeck",
    render_levels=[0, 1, 2]
)
```

### 3.4 Salience-Based Retrieval

**Scenario:** Fast access to critical fragments

```python
# Query hot fragments only (fast path)
hot_fragments = get_fragments(
    artifact_id="model-financial-proj",
    tier="hot",
    min_salience=0.7
)

# Reconstruct from hot fragments (incomplete but fast)
quick_view = reconstruct_artifact(
    fragments=hot_fragments,
    radicals=get_radicals(["quick-view"]),
    target_format="ikam-document"
)

# If full detail needed, promote warm/cold fragments
if needs_full_detail:
    all_fragments = get_fragments(artifact_id="model-financial-proj")
    promote_to_hot(all_fragments)
```

---

## 4. Development Plan

### 4.1 Phase 2 Integration Points

**Completed Phase 2 Work (Prerequisite):**
- ✅ Package architecture refactor (ikam, interacciones, modelado, narraciones)
- ✅ IKAM CRUD layer (artifact registry, derivations, provenance)
- ✅ Instruction parser agent (confidence gating, intent classification)
- ✅ MCP worker bridges (agent registration, progress events)

**IKAM v2 Work (New in Phase 2):**

1. **Fragment Model Definition** (Milestone 1)
   - Define Fragment, Radical, FragmentContent Pydantic models
   - Add to `packages/ikam/src/ikam/forja/models.py`
   - Acceptance: Models pass schema validation tests

2. **Forja Decomposer** (Milestone 2)
   - Implement `decompose_artifact()` for documents (MVP)
   - Support 3 levels: L0 (core), L1 (business model), L2 (financials)
   - Acceptance: Decompose sample document into 15+ fragments

3. **Forja Reconstructor** (Milestone 3)
   - Implement `reconstruct_artifact()` for documents (MVP)
   - Support full and partial level rendering
   - Acceptance: Round-trip document → fragments → document with 100% fidelity

4. **Radical System** (Milestone 4)
   - Define 5 MVP radicals: doc-section, doc-summary, slide-layout, chart-embed, table-format
   - Store radicals in database (radicals table)
   - Acceptance: Reconstruct with different radicals produces valid artifacts

5. **Salience Storage** (Milestone 5)
   - Add tier metadata to fragment storage
   - Implement hot/warm/cold migration logic
   - Acceptance: Fragments auto-migrate based on access patterns

6. **Cross-Format Translation** (Milestone 6)
   - Implement `translate_fragments()` for doc→slide
   - Support slide layout inference from document structure
   - Acceptance: Translate 10-page doc to 8-slide deck

### 4.2 Implementation Sequence

**Week 1-2: Foundation**
- [ ] Create `packages/ikam/src/ikam/forja/` subpackage
- [ ] Create `packages/ikam/src/ikam/almacen/` subpackage
- [ ] Define Fragment/Radical models (forja/models.py)
- [ ] Define storage abstractions (almacen/base.py, almacen/models.py)
- [ ] Migrate chunking logic from modelado to almacen
- [ ] Add database migrations for fragments, radicals tables
- [ ] Write model validation tests (test_fragment_models.py)

**Week 3-4: Decomposition & Storage**
- [ ] Implement semantic decomposer (forja/decomposer.py)
- [ ] Implement PostgreSQL hot tier storage (almacen/postgres.py)
- [ ] Implement MinIO warm/cold tier storage (almacen/minio.py)
- [ ] Add decomposition strategy: semantic, structural, manual
- [ ] Test decomposition on 3 sample documents (investor memo, business plan, quarterly report)
- [ ] Validate fragment hierarchy and parent links
- [ ] Test storage tier assignment based on salience

**Week 5-6: Reconstruction (Deterministic)**
- [ ] Implement reconstructor (forja/reconstructor.py)
- [ ] Add full-level rendering (all fragments)
- [ ] Test round-trip fidelity (artifact → fragments → artifact)
- [ ] **MILESTONE: Deterministic rendering validated** — 100% fidelity for full-level reconstruction
- [ ] Measure reconstruction performance (<500ms for 20 fragments)

**Week 7: Reconstruction (Non-Deterministic)**
- [ ] Add partial-level rendering (subset of fragments)
- [ ] Test partial reconstruction (L0-L1 only, L0-L2 only)
- [ ] Validate partial artifacts are valid subsets
- [ ] **MILESTONE: Non-deterministic rendering validated** — Partial rendering produces valid artifacts

**Week 8: Radicals & Translation**
- [ ] Implement radical registry and execution engine
- [ ] Add 5 MVP radicals (see Milestone 4)
- [ ] Implement doc→slide translator (forja/translator.py)
- [ ] Test cross-format translation accuracy
- [ ] **Note:** May require MCP agent interactions for complex rendering (e.g., chart generation, layout optimization)

**Week 9-10: Full Integration & Polish**
- [ ] Implement tier migration logic (almacen/tier_manager.py)
- [ ] Add salience scoring (based on access, importance, size)
- [ ] Integrate with MCP agents for rendering orchestration
- [ ] Add fragment storage monitoring (Prometheus metrics)
- [ ] Write comprehensive integration tests
- [ ] **MILESTONE: Full system integration** — End-to-end workflows with MCP agents

### 4.3 Testing Strategy

**Unit Tests:**
- Fragment model validation (schema, required fields, parent links)
- Decomposition strategy correctness (level assignment, content extraction)
- Reconstruction fidelity (round-trip equality)
- Radical template rendering (input→output correctness)
- Salience scoring (edge cases, boundary values)

**Integration Tests:**
- End-to-end decomposition → reconstruction
- Cross-format translation (doc→slide→doc)
- Storage tier migration (hot→warm→cold→hot)
- Derivation graph updates (fragment creation adds derivations)
- API contract tests (POST /api/model/fragments/decompose)

**Performance Tests:**
- Decompose 100-page document (<5s)
- Reconstruct 50 fragments (<1s)
- Translate 20 fragments doc→slide (<2s)
- Query 1000 hot fragments (<100ms)
- Tier migration batch (1000 fragments in <30s)

**Determinism Tests:**
- Full render path reproduces identical artifact (byte-level comparison)
- Partial render produces valid subset (schema validation)
- Fragment ordering preserves document flow (topological sort)

**Non-Determinism Tests:**
- Partial level rendering omits expected content
- Different radical produces different output format
- Salience-based retrieval returns subset
- Translation changes structure but preserves semantics

**Property-Based Tests (Hypothesis):**
- Decompose + reconstruct is identity (full levels)
- Partial reconstruct is subset of full reconstruct
- Fragment parent links form valid tree
- Salience scores in [0.0, 1.0]

---

## 8. Package Migration Strategy

### 8.1 Migrating Chunking Logic from Modelado

**Current State:**
- `packages/modelado/src/modelado/documents/chunking.py` contains document chunking logic
- Used for analysis pipeline (chunking documents for embeddings)
- Tightly coupled with modeling concerns

**Target State:**
- `packages/ikam/src/ikam/almacen/chunking.py` contains generalized chunking
- Decoupled from modeling; focused on storage optimization
- Supports fragment-level chunking for salience-based storage

**Migration Steps:**

1. **Create almacen package structure** (Week 1)
   ```bash
   mkdir -p packages/ikam/src/ikam/almacen
   touch packages/ikam/src/ikam/almacen/__init__.py
   touch packages/ikam/src/ikam/almacen/base.py
   touch packages/ikam/src/ikam/almacen/models.py
   touch packages/ikam/src/ikam/almacen/postgres.py
   touch packages/ikam/src/ikam/almacen/minio.py
   touch packages/ikam/src/ikam/almacen/tier_manager.py
   ```

2. **Copy and refactor chunking.py** (Week 2)
   ```bash
   cp packages/modelado/src/modelado/documents/chunking.py \
      packages/ikam/src/ikam/almacen/chunking.py
   ```
   - Remove analysis-specific logic (embedding calls)
   - Generalize for fragment storage
   - Add salience-based chunking strategies

3. **Update imports in services** ✅ (Completed Dec 4, 2025)
   ```python
   # Before (deprecated)
   from modelado.documents.chunking import chunk_document
   
   # Now (standard)
   from ikam.almacen.chunking import chunk_document
   ```

4. **Remove modelado.documents.chunking** ✅ (Completed Dec 4, 2025)
   - Removed deprecated module entirely
   - Updated all consumers to use ikam.almacen
   - All tests passing with clean imports

5. **almacen features enabled** ✅ (Completed Dec 4, 2025)
   - Fragment-level chunking
   - Tier assignment logic
   - Compression utilities

**Backward Compatibility:**
- Modelado package re-exports almacen.chunking for one sprint
- All new code uses ikam.almacen directly
- Deprecation warnings guide migration

### 8.2 Storage Backend Abstraction

**Abstract Interface:**

```python
# packages/ikam/src/ikam/almacen/base.py

from abc import ABC, abstractmethod
from typing import Optional
from ikam.forja.models import Fragment

class StorageBackend(ABC):
    """Abstract storage backend for fragments."""
    
    @abstractmethod
    def store(self, fragment: Fragment, tier: str) -> str:
        """Store fragment, return storage reference."""
        pass
    
    @abstractmethod
    def retrieve(self, fragment_id: str) -> Fragment:
        """Retrieve fragment by ID."""
        pass
    
    @abstractmethod
    def delete(self, fragment_id: str) -> None:
        """Delete fragment."""
        pass
    
    @abstractmethod
    def exists(self, fragment_id: str) -> bool:
        """Check if fragment exists."""
        pass
    
    @abstractmethod
    def get_tier(self, fragment_id: str) -> Optional[str]:
        """Get current storage tier for fragment."""
        pass
```

**PostgreSQL Implementation:**

```python
# packages/ikam/src/ikam/almacen/postgres.py

from .base import StorageBackend
from ikam.forja.models import Fragment

class PostgresBackend(StorageBackend):
    """Hot tier storage using PostgreSQL JSONB."""
    
    def __init__(self, connection_string: str):
        self.connection_string = connection_string
    
    def store(self, fragment: Fragment, tier: str = "hot") -> str:
        """Store fragment in fragments table."""
        # INSERT INTO fragments (id, content, tier) VALUES (...)
        pass
    
    def retrieve(self, fragment_id: str) -> Fragment:
        """Retrieve from fragments table."""
        # SELECT content FROM fragments WHERE id = %s
        pass
```

**MinIO Implementation:**

```python
# packages/ikam/src/ikam/almacen/minio.py

from .base import StorageBackend
from ikam.forja.models import Fragment
import minio

class MinioBackend(StorageBackend):
    """Warm/cold tier storage using MinIO."""
    
    def __init__(self, endpoint: str, access_key: str, secret_key: str):
        self.client = minio.Minio(endpoint, access_key, secret_key)
    
    def store(self, fragment: Fragment, tier: str = "warm") -> str:
        """Store fragment in MinIO bucket."""
        # self.client.put_object(bucket, object_name, data)
        pass
    
    def retrieve(self, fragment_id: str) -> Fragment:
        """Retrieve from MinIO."""
        pass
```

### 8.3 Database vs. Document Store Decision

**Question:** Should we switch from PostgreSQL to a document database (MongoDB, CouchDB) or key-value store (RocksDB)?

**Decision:** **Retain PostgreSQL** with JSONB as a reference backend; keep tiering engine-agnostic.

**Comparison:**

| Feature | PostgreSQL JSONB | MongoDB | RocksDB |
|---------|------------------|---------|---------|
| **ACID guarantees** | ✅ Full ACID | ⚠️ Eventual consistency (default) | ❌ No transactions |
| **Query flexibility** | ✅ SQL + JSONB operators | ✅ Rich query language | ❌ Key-value only |
| **Indexing** | ✅ GIN indexes on JSONB | ✅ Secondary indexes | ⚠️ Limited indexing |
| **Existing investment** | ✅ Already deployed | ❌ New infrastructure | ❌ New infrastructure |
| **pgvector integration** | ✅ Embeddings in same DB | ❌ Separate embedding store | ❌ Separate embedding store |
| **Schema evolution** | ✅ Alembic migrations | ⚠️ Schema-less (manual versioning) | ❌ No schema support |
| **Operational maturity** | ✅ Well-known to team | ⚠️ Learning curve | ⚠️ Learning curve |

**Rationale for PostgreSQL:**
1. **Existing infrastructure:** Already running PostgreSQL with pgvector
2. **ACID guarantees:** Fragment versioning and derivation graphs need consistency
3. **Unified storage:** Relational (projects, users) + semi-structured (fragments) in one DB
4. **Query power:** Can join fragments with artifacts, derivations, provenance
5. **Mature tooling:** Backups, replication, monitoring already in place

**When to reconsider:**
- Hot tier becomes read bottleneck (→ add read replicas or Redis cache)
- JSONB query performance degrades (→ benchmark MongoDB)
- Need distributed writes (→ consider CockroachDB or YugabyteDB)

**RocksDB Use Cases:**
- Embedded in services for local caching
- PostgreSQL FDW for custom heap storage (future)
- Not primary storage backend

---

## 9. Open Questions & Design Decisions

### 9.1 Resolved Design Decisions

✅ **Storage Backend:** Engine-agnostic tiers with pluggable backends; reference implementations include PostgreSQL and MinIO — See section 8.3  
✅ **Package Structure:** Separate `forja` (fragmentation) and `almacen` (storage) subpackages  
✅ **Chunking Migration:** Move from modelado to ikam.almacen — See section 8.1 for migration plan  
✅ **Testing Phases:** Phase 1 (deterministic) → Phase 2 (non-deterministic) → Phase 3 (integration) — See test plan  
✅ **MCP Integration:** Required for Week 8+ (radicals, rendering orchestration)

### 9.2 Open Design Questions

1. **Fragment granularity:** How fine-grained should fragments be? Paragraph-level? Section-level?
   - **Proposal:** Start with section-level (L0=abstract, L1=section, L2=subsection), refine based on usage

2. **Radical versioning:** How to handle breaking changes in radical templates?
   - **Proposal:** Semver for radicals; artifacts store radical version in metadata

3. **Salience scoring:** What algorithm determines fragment importance?
   - **Proposal:** Weighted formula: `salience = 0.4 * access_frequency + 0.3 * user_rating + 0.2 * derivation_count + 0.1 * size_penalty`

4. **Conflict resolution:** What happens if fragments are edited independently and diverge?
   - **Proposal:** Fragment edit triggers artifact re-reconstruction; user reviews diff

5. **External format radicals:** Should we support user-defined radicals (e.g., custom PPTX templates)?
   - **Proposal:** Phase 3 feature; Phase 2 supports only built-in radicals

### 9.3 Technical Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| **Decomposition accuracy** | Low-quality fragments → poor reconstructions | Start with manual decomposition, add ML later |
| **Reconstruction performance** | Slow for large artifacts (>100 fragments) | Implement fragment caching, parallel rendering |
| **Storage cost** | 3x storage (v1 artifact + v2 fragments + tiers) | Aggressive cold tier compression, opt-in fragmentation |
| **Schema drift** | v1 and v2 models diverge over time | Automated schema sync tests, shared base classes |
| **Radical complexity** | Templates become unmanageable | Limit radical features, provide visual editor (Phase 3) |

### 9.4 Dependencies

**Phase 2 Prerequisites:**
- ✅ Package architecture (ikam package exists)
- ✅ IKAM CRUD layer (artifact registry operational)
- ✅ Database migrations system (Alembic in place)

**External Dependencies:**
- Pydantic 2.7+ for fragment models
- PostgreSQL JSONB for hot tier storage
- MinIO for warm/cold tier storage
- (Optional) Vector database for semantic search across fragments

---

## 6. Success Metrics

**Phase 2 Completion Criteria:**
- [ ] Fragment/Radical models defined and validated
- [ ] Decompose 5 sample artifacts (3 docs, 1 slide deck, 1 sheet)
- [ ] Reconstruct with 100% fidelity (full levels)
- [ ] Reconstruct with valid partial levels (L0-L1 only)
- [ ] Translate 1 doc→slide with >80% semantic accuracy
- [ ] Salience storage demonstrates 50% cost reduction (warm+cold vs all-hot)
- [ ] API endpoints operational with <1s p95 latency
- [ ] 90% test coverage for forja package
- [ ] Documentation complete (this spec + API docs + tutorial)

**Long-Term Success (Phase 3+):**
- Fragments power 80% of artifact operations (rendering, derivation, export)
- Salience storage reduces DB size by 40%
- Cross-format translation supports 6+ formats (doc, slide, sheet, PDF, PPTX, Notion)
- User-defined radicals enable custom export templates
- Fragment search enables semantic queries across all artifacts

---

## 7. Next Steps

**Immediate Actions (Week 1):**
1. Review this specification with team → gather feedback
2. Create Milestone 1 tickets in project tracker
3. Set up `packages/ikam/src/ikam/forja/` subpackage skeleton
4. Write first fragment model tests (TDD approach)
5. Update `PHASES_SPRINTS_ROADMAP.md` with IKAM v2 milestones

**Short-Term (Weeks 2-4):**
1. Implement Fragment/Radical Pydantic models
2. Add database migrations (fragments, radicals, fragment_storage tables)
3. Build MVP decomposer for documents
4. Test decomposition on 3 sample documents

**Medium-Term (Weeks 5-10):**
1. Implement reconstructor with full/partial rendering
2. Build radical registry and execution engine
3. Add doc→slide translator
4. Implement salience storage tiers

---

## Appendix A: Fragment Schema Examples

### A.1 Text Fragment (Level 0: Core Value Prop)

```json
{
  "id": "frag-L0-001",
  "artifactId": "doc-investor-memo-abc",
  "level": 0,
  "parentFragmentId": null,
  "type": "text",
  "content": {
    "summary": "Marketplace connecting freelance designers with startups",
    "blocks": [
      {
        "type": "paragraph",
        "id": "p-001",
        "content": [
          { "text": "We connect early-stage startups with vetted freelance designers through an AI-powered matching platform." }
        ]
      }
    ]
  },
  "radicalRefs": ["radical-doc-section"],
  "salience": 1.0,
  "createdAt": "2025-11-15T10:00:00Z",
  "updatedAt": "2025-11-15T10:00:00Z"
}
```

### A.2 Data Fragment (Level 2: Unit Economics)

```json
{
  "id": "frag-L2-econ-003",
  "artifactId": "doc-investor-memo-abc",
  "level": 2,
  "parentFragmentId": "frag-L1-econ-002",
  "type": "data",
  "content": {
    "summary": "Detailed unit economics with CAC, LTV, margins",
    "schema": {
      "columns": [
        { "key": "metric", "label": "Metric", "type": "string" },
        { "key": "value", "label": "Value", "type": "currency" },
        { "key": "benchmark", "label": "Industry Benchmark", "type": "currency" }
      ]
    },
    "rows": [
      { "metric": "CAC", "value": 150, "benchmark": 200 },
      { "metric": "LTV", "value": 1200, "benchmark": 900 },
      { "metric": "Gross Margin", "value": 0.65, "benchmark": 0.55 }
    ]
  },
  "radicalRefs": ["radical-table-format"],
  "salience": 0.8,
  "createdAt": "2025-11-15T10:05:00Z",
  "updatedAt": "2025-11-15T10:05:00Z"
}
```

### A.3 Visual Fragment (Level 1: Revenue Forecast Chart)

```json
{
  "id": "frag-L1-visual-004",
  "artifactId": "doc-investor-memo-abc",
  "level": 1,
  "parentFragmentId": "frag-L0-001",
  "type": "visual",
  "content": {
    "summary": "3-year revenue forecast chart",
    "chartSpec": {
      "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
      "mark": "line",
      "encoding": {
        "x": { "field": "year", "type": "ordinal" },
        "y": { "field": "revenue", "type": "quantitative" }
      },
      "data": { "values": [
        { "year": "2025", "revenue": 500000 },
        { "year": "2026", "revenue": 1500000 },
        { "year": "2027", "revenue": 4000000 }
      ]}
    }
  },
  "radicalRefs": ["radical-chart-embed"],
  "salience": 0.9,
  "createdAt": "2025-11-15T10:10:00Z",
  "updatedAt": "2025-11-15T10:10:00Z"
}
```

---

## Appendix B: Radical Examples

### B.1 Document Section Radical

```json
{
  "id": "radical-doc-section",
  "name": "Document Section Builder",
  "description": "Renders fragments as document sections with heading, body, and optional details",
  "inputSpec": {
    "fragmentLevels": [0, 1, 2],
    "minFragments": 1,
    "fragmentTypes": ["text", "data"]
  },
  "outputFormat": "ikam-document",
  "template": {
    "type": "jinja2",
    "source": "{% for frag in fragments %}\n  <section>\n    <h2>{{ frag.content.summary }}</h2>\n    {{ frag.content.blocks | render_blocks }}\n  </section>\n{% endfor %}"
  },
  "parameters": {
    "theme": "default",
    "includeCharts": true,
    "maxSectionDepth": 3
  },
  "version": "1.0.0"
}
```

### B.2 Slide Layout Radical

```json
{
  "id": "radical-slide-layout",
  "name": "Slide Layout Builder",
  "description": "Converts fragments to slide deck with title, content, and visual slides",
  "inputSpec": {
    "fragmentLevels": [0, 1],
    "minFragments": 1,
    "fragmentTypes": ["text", "visual"]
  },
  "outputFormat": "ikam-slideDeck",
  "template": {
    "type": "code",
    "source": "ikam.forja.radicals.slide_layout"
  },
  "parameters": {
    "theme": "pitch-deck",
    "slidesPerFragment": 1,
    "includeAppendix": false
  },
  "version": "1.0.0"
}
```

---

## Appendix C: Storage Tier Schema

### C.1 Fragment Storage Table

```sql
CREATE TABLE fragment_storage (
  fragment_id UUID PRIMARY KEY REFERENCES fragments(id) ON DELETE CASCADE,
  tier TEXT NOT NULL CHECK (tier IN ('hot', 'warm', 'cold')),
  backend_name TEXT NOT NULL, -- e.g., 'postgres', 'minio', 's3', 'fs'
  location_path TEXT NOT NULL,
  compressed BOOLEAN DEFAULT FALSE,
  metadata JSONB NOT NULL,
  created_at TIMESTAMPTZ DEFAULT NOW(),
  updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_fragment_storage_tier ON fragment_storage(tier);
CREATE INDEX idx_fragment_storage_salience ON fragment_storage((metadata->>'salience'));
```

### C.2 Fragments Table

```sql
CREATE TABLE fragments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    artifact_id UUID NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    level INTEGER NOT NULL CHECK (level >= 0),
    parent_fragment_id UUID REFERENCES fragments(id) ON DELETE CASCADE,
    type TEXT NOT NULL CHECK (type IN ('text', 'data', 'visual', 'structural')),
    content JSONB NOT NULL,
    radical_refs TEXT[] DEFAULT '{}',
    salience NUMERIC(3,2) NOT NULL CHECK (salience >= 0.0 AND salience <= 1.0),
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_fragments_artifact ON fragments(artifact_id);
CREATE INDEX idx_fragments_level ON fragments(level);
CREATE INDEX idx_fragments_parent ON fragments(parent_fragment_id);
CREATE INDEX idx_fragments_salience ON fragments(salience DESC);
```

### C.3 Radicals Table

```sql
CREATE TABLE radicals (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    input_spec JSONB NOT NULL,
    output_format TEXT NOT NULL,
    template JSONB NOT NULL,
    parameters JSONB DEFAULT '{}',
    version TEXT NOT NULL,
    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_radicals_output_format ON radicals(output_format);
```

---

## Document History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 2.0.0-draft | 2025-11-15 | System | Initial v2 design exploration |

---

**References:**
- [IKAM v1.0.0 Specification](./ikam-specification.md)
- [IKAM Project Specification](./ikam-project-specification.md)
- [IKAM Sheet Specification](./ikam-sheet-specification.md)
- [Phase 2 Roadmap](../planning/PHASES_SPRINTS_ROADMAP.md)
