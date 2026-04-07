# IKAM Graph Specification

_Version 2.0.0 | November 2025_

## Overview

The **IKAM Graph** is Narraciones' foundational layer for knowledge representation, providing content-addressable storage (CAS), lossless reconstruction, and complete provenance tracking. It implements the mathematical guarantees defined in IKAM v2 and serves as the storage substrate for all artifacts (documents, slides, sheets, models, media).

**Key Properties:**
- **Content-Addressable Storage (CAS):** Fragments identified by blake3 hash; automatic deduplication
- **Lossless Reconstruction:** `reconstruct(decompose(A)) = A` with byte-level equality
- **Storage Monotonicity:** `Δ(N) = S_flat(N) - S_IKAM(N) ≥ 0` for N ≥ 2 artifacts sharing fragments
- **Fisher Information Dominance:** `I_IKAM(θ) ≥ I_RAG(θ) + Δ_provenance(N)` via complete derivation tracking
- **Deterministic Composition:** Fragment ordering and delta application are reproducible

**⚠️ Semantic Evaluation Principle:**
Artifact kind classification and schema inference use **semantic interpretation as a mandatory core feature**. Artifact types are inferred from content analysis (table detection → spreadsheet, heading structure → document, slide markers → presentation) and metadata hints—not from hardcoded `ArtifactKind` enums checked with strict predicates. Missing semantic infrastructure for content inspection is a fatal configuration error. See root `AGENTS.md` and `docs/testing/SEQUENCING_FRAMEWORK_PERSONAS_SPEC.md` for details.

---

## Design Principles

1. **CAS-First** — All content stored as immutable, content-addressed fragments
2. **Graph Structure** — Artifacts, fragments, derivations, and variations form a directed acyclic graph (DAG)
3. **Provenance Complete** — Every artifact records complete lineage with seeds, renderer versions, and policies
4. **Lossless by Default** — No lossy operations in MVP; partial rendering deferred to Phase 3+
5. **Delta Chain Bounded** — Maximum chain length L ≤ 3 enforced via deterministic rebase
6. **Multi-Target Export** — Single canonical representation renders to PPTX, PDF, HTML, etc.

---

## Core Entities

## Two-layer fragment model (storage vs domain)

IKAM uses two complementary fragment representations:

- **Storage fragments (CAS)** are immutable byte blobs addressed by `blake3(bytes)` for deduplication and byte-level proofs.
- **Domain fragments (materialized)** are typed semantic objects reconstructed on read from `(storage fragment bytes + metadata tables)`; they support hierarchy, rendering, and higher-level semantics.

Rule: never mix layers directly—conversions go through adapters. For the operator framing (O1–O10), grounding retrieval (O5), and relational fragments (connections + descriptions), see [IKAM Graph Algorithms Plan](./IKAM_GRAPH_ALGORITHMS_PLAN.md).

### Fragment

**Purpose:** Immutable, content-addressed byte sequences forming the atomic storage unit.

**Schema:**
```python
class Fragment(BaseModel):
    id: str                    # blake3 hex digest of bytes (64 chars)
    bytes: bytes               # Raw content (UTF-8 text, binary, etc.)
    size_bytes: int            # len(bytes)
    mime_type: str | None      # e.g., "text/plain", "image/png"
    created_at: datetime       # When first stored

    @classmethod
    def from_bytes(cls, data: bytes, mime_type: str | None = None) -> "Fragment":
        """Create fragment with CAS ID from content."""
        return cls(
            id=_cas_hex(data),
            bytes=data,
            size_bytes=len(data),
            mime_type=mime_type,
            created_at=datetime.utcnow()
        )
```

**Properties:**
- **Immutable:** Once stored, fragments never change (updates create new fragments)
- **Deduplication:** Identical content yields identical `id`; stored once
- **Deterministic:** `blake3(bytes)` ensures reproducible IDs across systems

**Note:** This entity is the **storage fragment**. “Domain fragments” (typed semantic objects like paragraphs, slides, cells, and relational payloads) are materialized on read.

**Storage Table:**
```sql
CREATE TABLE ikam_fragments (
    id TEXT PRIMARY KEY,                          -- blake3 hex
    bytes BYTEA NOT NULL,
    size_bytes BIGINT NOT NULL,
    mime_type TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_id_length CHECK (length(id) = 64),
    CONSTRAINT chk_size_match CHECK (size_bytes = octet_length(bytes))
);
CREATE INDEX idx_fragments_created ON ikam_fragments(created_at);
CREATE INDEX idx_fragments_mime ON ikam_fragments(mime_type) WHERE mime_type IS NOT NULL;
```

---

### Artifact

**Purpose:** Logical knowledge units composed of ordered fragments; the user-facing entity (document, slide, sheet, model, image, video, file).

**Schema:**
```python
class ArtifactKind(str, Enum):
    DOCUMENT = "document"
    SLIDE_DECK = "slide_deck"
    SHEET = "sheet"
    ECONOMIC_MODEL = "economic_model"
    STORY_MODEL = "story_model"
    IMAGE = "image"
    VIDEO = "video"
    FILE = "file"
    EXTERNAL_DOC = "external_doc"

class Artifact(BaseModel):
    id: str                    # UUID v4 or stable uuid5 for migrations
    kind: ArtifactKind
    title: str | None
    version: str               # Semver (e.g., "1.0.0")
    fragment_ids: list[str]    # Ordered blake3 IDs; order matters for reconstruction
    created_at: datetime
    updated_at: datetime
    metadata: dict[str, Any]   # Kind-specific metadata (e.g., MIME type, dimensions)
```

**Reconstruction Invariant:**
```python
def reconstruct_artifact(artifact: Artifact, fragments: list[Fragment]) -> bytes:
    """Lossless reconstruction from ordered fragments."""
    assert len(fragments) == len(artifact.fragment_ids)
    assert all(f.id == fid for f, fid in zip(fragments, artifact.fragment_ids))
    return b"".join(f.bytes for f in fragments)
```

**Storage Tables:**
```sql
CREATE TABLE ikam_artifacts (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    title TEXT,
    version TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB
);
CREATE INDEX idx_artifacts_kind ON ikam_artifacts(kind);
CREATE INDEX idx_artifacts_updated ON ikam_artifacts(updated_at);

CREATE TABLE ikam_artifact_fragments (
    artifact_id TEXT NOT NULL REFERENCES ikam_artifacts(id) ON DELETE CASCADE,
    fragment_id TEXT NOT NULL REFERENCES ikam_fragments(id) ON DELETE RESTRICT,
    position INT NOT NULL,
    PRIMARY KEY (artifact_id, position),
    CONSTRAINT chk_position_nonnegative CHECK (position >= 0)
);
CREATE INDEX idx_artifact_fragments_frag ON ikam_artifact_fragments(fragment_id);
```

**Properties:**
- **Ordered Composition:** Fragment order preserved via `position` column
- **Shared Fragments:** Multiple artifacts can reference the same fragment → storage savings
- **Versioned:** Semantic versioning enables compatibility tracking

**Relational fragments (connections + descriptions):**
Artifacts may include fragments whose payloads describe relationships, constraints, or explanations about other graph entities (e.g., “why these numbers?”, “traceback/provenance”, policy predicates). These are stored in CAS like any other fragment, and can project into effective edges via deterministic edge-event appends.

---

### Derivation

**Purpose:** Explicit lineage tracking connecting derived artifacts to their sources with transformation metadata.

**Schema:**
```python
class DerivationType(str, Enum):
    COMPOSE = "compose"              # Combined multiple sources
    TRANSFORM = "transform"          # Applied function/operation
    REFERENCE = "reference"          # Cited or embedded
    CONTEXTUALIZE = "contextualize"  # Added context/annotations
    VARY = "vary"                    # Non-deterministic variation
    DELTA = "delta"                  # Deterministic delta chain

class Derivation(BaseModel):
    id: str                          # UUID v4
    derived_artifact_id: str         # The result artifact
    source_artifact_ids: list[str]   # One or more sources
    derivation_type: DerivationType
    operation: str | None            # E.g., "concat", "render", "rebase"
    parameters: dict[str, Any]       # Transformation params (seed, policy, etc.)
    created_at: datetime
```

**Storage Tables:**
```sql
CREATE TABLE ikam_derivations (
    id TEXT PRIMARY KEY,
    derived_artifact_id TEXT NOT NULL REFERENCES ikam_artifacts(id) ON DELETE CASCADE,
    derivation_type TEXT NOT NULL,
    operation TEXT,
    parameters JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_derivations_derived ON ikam_derivations(derived_artifact_id);

CREATE TABLE ikam_derivation_sources (
    derivation_id TEXT NOT NULL REFERENCES ikam_derivations(id) ON DELETE CASCADE,
    source_artifact_id TEXT NOT NULL REFERENCES ikam_artifacts(id) ON DELETE RESTRICT,
    position INT NOT NULL,
    PRIMARY KEY (derivation_id, position)
);
CREATE INDEX idx_derivation_sources_artifact ON ikam_derivation_sources(source_artifact_id);
```

**Fisher Information Guarantee:**
- Provenance completeness ensures `I_IKAM(θ) ≥ I_RAG(θ)`
- Every derived artifact traces back to canonical sources
- Test suite validates no information loss during derivation

---

### Variation

**Purpose:** Non-deterministic rendering artifacts with seeds for reproducibility.

**Schema:**
```python
class Variation(BaseModel):
    id: str                          # UUID v4
    base_artifact_id: str            # Canonical base
    variation_artifact_id: str       # Rendered variant
    seed: int | None                 # RNG seed for reproduction
    renderer_version: str            # E.g., "slides-renderer-1.2.3"
    policy: dict[str, Any]           # Rendering policy (theme, layout hints)
    created_at: datetime
```

**Properties:**
- **Reproducible:** Given (base, seed, renderer_version, policy) → deterministic output
- **Traceable:** Variation links to base artifact for provenance
- **Fisher Info Preserved:** Recording seed/policy prevents information loss

---

### ProvenanceEvent

**Purpose:** Audit log for artifact lifecycle events (creation, updates, deltas, exports).

**Schema:**
```python
class ProvenanceEvent(BaseModel):
    id: str                          # UUID v4
    artifact_id: str
    event_type: str                  # "created", "updated", "exported", "rebased"
    actor: str | None                # User ID or system identifier
    timestamp: datetime
    metadata: dict[str, Any]         # Event-specific data
```

**Storage Table:**
```sql
CREATE TABLE ikam_provenance_events (
    id TEXT PRIMARY KEY,
    artifact_id TEXT NOT NULL REFERENCES ikam_artifacts(id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    actor TEXT,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB
);
CREATE INDEX idx_provenance_artifact ON ikam_provenance_events(artifact_id, timestamp);
CREATE INDEX idx_provenance_type ON ikam_provenance_events(event_type);
```

---

## Edge Semantics

The IKAM Graph uses typed edges to represent relationships between entities:

### Composition Edges

**`composes`:** Artifact → Fragment (via `ikam_artifact_fragments`)
- **Semantics:** Artifact is composed of ordered fragments
- **Cardinality:** Many-to-many (artifacts share fragments)
- **Order:** Position-sensitive; order determines reconstruction
- **Constraint:** All fragment IDs must exist; deletion restricted

### Derivation Edges

**`derives_from`:** Artifact → Artifact (via `ikam_derivations` + `ikam_derivation_sources`)
- **Semantics:** Derived artifact produced from source artifact(s) via transformation
- **Cardinality:** Many-to-many (one artifact can derive from multiple sources; one source can produce multiple derivatives)
- **Type:** Tagged with `DerivationType` (compose, transform, reference, contextualize, vary, delta)
- **Constraint:** Must form DAG (no cycles)

**`references`:** Artifact → Artifact (via `DerivationType.REFERENCE`)
- **Semantics:** Artifact cites or embeds another artifact without structural transformation
- **Use Case:** Document references external source; slide embeds chart
- **Preservation:** Referenced artifact must remain available

**`contextualizes`:** Artifact → Artifact (via `DerivationType.CONTEXTUALIZE`)
- **Semantics:** Artifact adds context, annotations, or metadata to base artifact
- **Use Case:** Annotated screenshot, captioned video, commented model
- **Idempotence:** Multiple contextualization steps on same base

### Variation Edges

**`varies_from`:** Artifact → Artifact (via `ikam_variations`)
- **Semantics:** Non-deterministic rendering variation of base artifact
- **Reproducibility:** Seed + renderer version + policy enable exact reproduction
- **Directionality:** One-way from base to variation (variations don't vary further)

**`delta_of`:** Artifact → Artifact (via `DerivationType.DELTA`)
- **Semantics:** Deterministic delta chain; derived artifact is base + delta
- **Constraint:** Chain length L ≤ 3 enforced via rebase
- **Operations:** `apply_delta(base, delta) → derived` must be deterministic and lossless

### Provenance Edges

**`provenance_of`:** ProvenanceEvent → Artifact (via `ikam_provenance_events`)
- **Semantics:** Event describes action on artifact
- **Cardinality:** Many-to-one (many events per artifact)
- **Audit:** Immutable log; events never deleted

---

## Mathematical Guarantees

### 1. Lossless Reconstruction

**Invariant:**
```
∀ artifact A: reconstruct(decompose(A)) = A
```

**Implementation:**
- `decompose(A)` stores ordered fragments with position
- `reconstruct(A)` concatenates fragments in position order
- **Test:** `packages/ikam/tests/test_graph_roundtrip.py::test_single_fragment_roundtrip`

### 2. Storage Monotonicity

**Invariant:**
```
Δ(N) = S_flat(N) - S_IKAM(N) ≥ 0 for N ≥ 2
```

Where:
- `S_flat(N)` = total storage for N artifacts without deduplication
- `S_IKAM(N)` = total storage with CAS deduplication
- `Δ(N)` = storage savings (monotonically increasing with N)

**Break-Even:** N = 2 artifacts sharing ≥1 fragment → positive savings

**Test:** `packages/ikam/tests/test_graph_roundtrip.py::test_cas_deduplication_storage_savings`

**Observability:** See `docs/ikam/IKAM_STORAGE_OBSERVABILITY_HOWTO.md` for Prometheus metrics and Grafana dashboards tracking Δ(N).

### 3. Fisher Information Dominance

**Invariant:**
```
I_IKAM(θ) ≥ I_RAG(θ) + Δ_provenance(N)
```

Where:
- `I_IKAM(θ)` = Fisher information in IKAM representation
- `I_RAG(θ)` = Fisher information in baseline RAG system
- `Δ_provenance(N)` = additional information from provenance tracking

**Mechanism:** Complete derivation chains preserve all transformations, enabling better parameter estimation.

**Test:** `packages/ikam/tests/test_fisher_information.py` (pending)

**Reference:** See `docs/ikam/FISHER_INFORMATION_GAINS.md` for proof and empirical validation.

### 4. Deterministic Composition

**Invariant:**
```
∀ fragments F₁, F₂, ..., Fₙ: 
  compose([F₁, F₂, ..., Fₙ], order=[0,1,...,n-1]) 
  = compose([F₁, F₂, ..., Fₙ], order=[0,1,...,n-1])
```

**Implementation:** Fragment position stored in `ikam_artifact_fragments.position`; reconstruction uses `ORDER BY position`.

**Test:** `packages/ikam/tests/test_graph_roundtrip.py::test_multi_fragment_ordered_roundtrip`

---

## Delta Chain Management

### Policy

**Chain Length Bound:** L ≤ 3
- **Rationale:** Limits reconstruction complexity and error propagation
- **Enforcement:** Rebase policy triggers when L > 3

### Rebase Operation

**Purpose:** Collapse delta chains exceeding L=3 into a canonical base.

**Algorithm:**
```python
def rebase_delta_chain(chain: list[Artifact]) -> Artifact:
    """
    Collapse chain into single canonical artifact.
    
    Ensures: reconstruct(rebase(chain)) = reconstruct(apply_deltas(chain))
    """
    base = chain[0]
    deltas = chain[1:]
    
    # Apply deltas sequentially
    current = reconstruct_artifact(base)
    for delta_artifact in deltas:
        delta = parse_delta(delta_artifact)
        current = apply_delta(current, delta)
    
    # Create new canonical artifact
    canonical = decompose(current)
    
    # Record derivation with rebase metadata
    record_derivation(
        derived=canonical,
        sources=chain,
        type=DerivationType.DELTA,
        operation="rebase",
        parameters={"chain_length": len(chain)}
    )
    
    return canonical
```

**Invariant:** `reconstruct(rebase(chain)) = reconstruct(apply_deltas(chain))`

**Test:** `packages/ikam/tests/test_delta_rebase.py` (pending)

---

## API Endpoints

All graph endpoints are under `/api/model/graph` (follows allowed prefixes policy from `AGENTS.md`).

### Fragment Endpoints

**`GET /api/model/graph/fragments/{fragment_id}`**
- **Purpose:** Retrieve fragment metadata (without bytes)
- **Response:** `Fragment` (excluding `bytes` field for efficiency)
- **Status:** 200 OK, 404 Not Found

**`GET /api/model/graph/fragments/{fragment_id}/bytes`**
- **Purpose:** Download raw fragment bytes
- **Response:** Raw bytes with appropriate `Content-Type`
- **Status:** 200 OK, 404 Not Found

### Artifact Endpoints

**`GET /api/model/graph/artifacts/{artifact_id}`**
- **Purpose:** Retrieve artifact metadata and fragment list
- **Response:** `Artifact` (fragment IDs only, not full bytes)
- **Status:** 200 OK, 404 Not Found

**`GET /api/model/graph/artifacts/{artifact_id}/derivations`**
- **Purpose:** List all derivations where artifact is source or derived
- **Response:** `{"as_source": [Derivation], "as_derived": [Derivation]}`
- **Status:** 200 OK

### Provenance Endpoints

**`GET /api/model/graph/artifacts/{artifact_id}/provenance`**
- **Purpose:** Retrieve full provenance chain (recursive)
- **Response:** `{"events": [ProvenanceEvent], "derivations": [Derivation], "sources": [Artifact]}`
- **Status:** 501 Not Implemented (pending recursive CTE implementation)

---

## Repository API

Python persistence layer for graph operations (see `packages/modelado/src/modelado/ikam_graph_repository.py`).

### Fragment Operations

```python
def insert_fragment(conn, fragment: Fragment) -> None:
    """Insert fragment (idempotent via ON CONFLICT DO NOTHING)."""

def get_fragment_by_id(conn, fragment_id: str) -> Fragment | None:
    """Retrieve fragment including bytes."""
```

### Artifact Operations

```python
def insert_artifact(conn, artifact: Artifact, fragments: list[Fragment]) -> None:
    """
    Insert artifact and link fragments with positions.
    Atomic transaction ensures consistency.
    """

def get_artifact_by_id(conn, artifact_id: str) -> Artifact | None:
    """Retrieve artifact metadata and fragment IDs (ordered)."""
```

### Derivation Operations

```python
def insert_derivation(conn, derivation: Derivation) -> None:
    """Record derivation with source linkage."""

def get_derivations_for_artifact(conn, artifact_id: str) -> dict[str, list[Derivation]]:
    """
    Get derivations where artifact appears as source or derived.
    Returns: {"as_source": [...], "as_derived": [...]}
    """
```

---

## Migration & Backfill

### Schema Migration

**Script:** `scripts/database/apply_ikam_graph_migration.py`

**Safety Features:**
- Advisory locks prevent concurrent migrations
- Statement timeout (120s), lock timeout (10s), idle-in-transaction timeout (30s)
- Chunked writes for large migrations
- Dry-run mode for validation

**Usage:**
```bash
python scripts/database/apply_ikam_graph_migration.py \
  --database-url "$DATABASE_URL"
```

### Backfill uploaded_files

**Script:** `scripts/database/backfill_uploaded_files_to_ikam.py`

**Features:**
- Stable artifact IDs via `uuid5(namespace, file_id)`
- CAS deduplication (identical uploads → single fragment)
- Dry-run mode for validation
- Transactional commits

**Usage:**
```bash
# Dry-run (preview only)
python scripts/database/backfill_uploaded_files_to_ikam.py \
  --database-url "$DATABASE_URL" \
  --dry-run

# Actual migration
python scripts/database/backfill_uploaded_files_to_ikam.py \
  --database-url "$DATABASE_URL"
```

**Validation:**
```sql
-- Check fragment count vs uploaded_files
SELECT COUNT(DISTINCT id) AS unique_fragments FROM ikam_fragments;
SELECT COUNT(*) AS uploaded_files FROM uploaded_files;

-- Verify artifacts created
SELECT kind, COUNT(*) FROM ikam_artifacts GROUP BY kind;
```

---

## Testing

### Round-Trip Tests

**File:** `packages/ikam/tests/test_graph_roundtrip.py`

**Coverage:**
- Single fragment round-trip (byte equality)
- Multi-fragment ordered reconstruction
- CAS deduplication storage savings
- Identical content yields same fragment ID
- Empty artifact edge case
- Large fragment (2MB) preservation
- Binary data (non-UTF8) round-trip
- Storage monotonicity with variations

**Run:**
```bash
cd /path/to/narraciones-de-economicos
source .venv/bin/activate
export PYTHONPATH=packages/ikam/src:packages/modelado/src
export TEST_DATABASE_URL="postgresql://user:pass@localhost:5432/app"
pytest -v packages/ikam/tests/test_graph_roundtrip.py
```

### Integration Tests

**File:** `packages/modelado/tests/test_graph_roundtrip_integration.py`

**Coverage:**
- Repository insert → retrieve → byte equality
- Transaction handling
- Foreign key constraints

**Run:** Same as above, with `packages/modelado/tests/` path.

---

## Observability

### Metrics

**Prometheus Counters (Planned):**
- `ikam_graph_nodes_total{kind}` — Total artifacts by kind
- `ikam_graph_fragments_total` — Total fragments stored
- `ikam_graph_derivations_total{type}` — Derivations by type
- `ikam_graph_rebase_operations_total` — Delta chain rebases
- `ikam_graph_reconstruction_seconds{quantile}` — Reconstruction latency

**Gauges (Planned):**
- `ikam_graph_storage_delta_bytes` — Current Δ(N) storage savings
- `ikam_graph_chain_length_max` — Longest delta chain

### Diagnostics API

**Endpoint (Planned):** `GET /api/diagnostics/ikam/graph`

**Response:**
```json
{
  "nodes": {
    "artifacts": 1234,
    "fragments": 5678,
    "derivations": 910
  },
  "edges": {
    "composes": 3456,
    "derives_from": 789,
    "varies_from": 123
  },
  "storage": {
    "total_fragments_bytes": 123456789,
    "total_artifacts_bytes_flat": 234567890,
    "delta_bytes": 111111101
  },
  "chains": {
    "max_delta_chain_length": 2,
    "avg_delta_chain_length": 1.3
  }
}
```

---

## Future Work

### Phase 3+

- **Inference Hooks:** Fragment completion suggestions, variation inference
- **Collaboration Metadata:** User attribution, conflict resolution
- **Recursive Provenance Queries:** Recursive CTE for full ancestry chains
- **Denormalized Views:** Materialized edge counts for dashboard queries
- **Cold Storage Tiering:** Archive infrequently accessed fragments to S3/MinIO
- **Partial Rendering:** Defer non-critical fragments for streaming performance

### Research Areas

- **Adaptive Chain Length:** Dynamic L based on reconstruction latency
- **Fragment Granularity Optimization:** Semantic chunking for better deduplication
- **Cross-Project Deduplication:** Shared fragment pool across projects

---

## References

- **Mathematical Proofs:** See `docs/ikam/FISHER_INFORMATION_GAINS.md`, `STORAGE_GAINS_EXAMPLE.md`
- **Deltas & Variations:** See `docs/ikam/MUTATION_AND_VARIATION_MODEL.md`
- **Test Documentation:** See `packages/ikam/tests/ROUNDTRIP_TESTS_README.md`
- **Observability Guide:** See `docs/ikam/IKAM_STORAGE_OBSERVABILITY_HOWTO.md`
- **IKAM v2 Spec:** See `docs/ikam/ikam-v2-fragmented-knowledge-system.md`
- **Package Structure:** See `AGENTS.md` section on package architecture

---

## Changelog

### 2.0.0 (November 2025)
- Initial IKAM Graph specification
- Defined core entities (Fragment, Artifact, Derivation, Variation, ProvenanceEvent)
- Documented edge semantics and constraints
- Specified mathematical guarantees (lossless reconstruction, storage monotonicity, Fisher info dominance)
- Implemented delta chain rebase policy (L ≤ 3)
- Added API endpoints under `/api/model/graph`
- Created migration and backfill scripts
- Comprehensive round-trip test suite (10+ tests)

---

_For questions or contributions, see `AGENTS.md` and `TODO.md`._
