# ikam [![Docs](https://img.shields.io/badge/docs-Read%20the%20Docs-blue)](https://narraciones.readthedocs.io/en/latest/ikam/)

Core models for IKAM (Integrated Knowledge & Artifacts Model).

## Install

Development (editable):

```bash
pip install -e /Users/Shared/p/ingenierias-lentas/narraciones-de-economicos/packages/ikam
```

Wheel (release):

```bash
pip install /path/to/narraciones/packages/ikam/dist/ikam-0.1.0-py3-none-any.whl
```

## Usage

```python
from ikam import fragments
```

## Architecture

- Layer 0 (Standalone): IKAM domain models with zero hard deps.
- Upstream packages `interacciones` and `modelado` depend on `ikam`.

## Python support

Requires Python 3.11+.

## Releasing

Tagged releases publish via CI to GitHub Packages.

1. Bump `version` in `packages/ikam/pyproject.toml`.
2. Commit and push.
3. Tag and push:
    ```bash
    git tag -a ikam-v0.1.1 -m "ikam v0.1.1"
    git push origin ikam-v0.1.1
    ```
4. CI builds and publishes. Install from consumers:
    ```bash
    pip install ikam --extra-index-url https://pypi.pkg.github.com/<owner>/
    ```

## Documentation

- API and guides: https://narraciones.readthedocs.io/en/latest/ikam/

### Release Checklist

- Update `packages/ikam/pyproject.toml` `version`
- Add entry to `packages/ikam/CHANGELOG.md`
- Commit changes
- Create and push tag `ikam-vX.Y.Z`
# IKAM (Incremental Knowledge Artifact Model)

**Layer 0: Standalone domain models for fragment-based knowledge representation with zero dependencies**

## Overview

IKAM is a content-addressable storage (CAS) system for hierarchical knowledge artifacts. It decomposes documents, spreadsheets, and presentations into reusable fragments with complete provenance tracking.

### Key Features

- **Lossless decomposition:** `reconstruct(decompose(A)) = A` (byte-level equality)
- **Content-addressable storage:** Automatic deduplication via BLAKE3 hashing
- **Provenance tracking:** Complete derivation chains with Fisher Information metrics
- **Storage efficiency:** Δ(N) = S_flat(N) − S_IKAM(N) grows monotonically with N outputs
- **Mathematical guarantees:** I_IKAM(θ) ≥ I_RAG(θ) + Δ_provenance(θ)

## Architecture Position

```
Layer 0 (Standalone): ikam ← YOU ARE HERE
Layer 1 (Frameworks):  modelado, interacciones (depend on ikam)
Layer 2 (Application): narraciones (business logic)
```

**Dependencies:** None (only Pydantic v2 for data models)

## Installation

```bash
# From PyPI (when published)
pip install ikam

# Local editable install
pip install -e packages/ikam

# From wheel
pip install packages/ikam/dist/ikam-0.2.0-py3-none-any.whl

# With optional BLAKE3 (faster hashing)
pip install ikam[blake3]
```

## Quick Start

### Decompose a Document

```python
from ikam import decompose_document, DecompositionConfig, DecompositionStrategy

config = DecompositionConfig(
    strategy=DecompositionStrategy.SEMANTIC,
    preserve_structure=True
)

fragments = decompose_document(
    content="# Report\n\nQ1 Revenue: $1.5M\n\nGrowth: 15%",
    artifact_id="report-123",
    config=config
)

# fragments is a list of Fragment objects with hierarchy
for frag in fragments:
    print(f"{frag.level}: {frag.type} ({frag.salience})")
```

### Reconstruct from Fragments

```python
from ikam import reconstruct_document, ReconstructionConfig

config = ReconstructionConfig(
    format="markdown",
    include_metadata=False
)

original_content = reconstruct_document(fragments, config)
# Byte-level equality guaranteed
```

### Content-Addressable Storage

```python
from ikam.graph import Fragment

# Storage layer uses content hashing for deduplication
frag = Fragment(
    bytes=b"Revenue: $1.5M",
    mime_type="text/plain"
)

# ID is BLAKE3(bytes) - identical content = same ID
print(frag.id)  # blake3:abc123...
```

### Provenance Tracking

```python
from ikam.provenance import DerivationRecord, DerivationType

# Record decomposition
record = DerivationRecord(
    source_key="artifact:report-123",
    target_key="fragment:abc123",
    derivation_type=DerivationType.DECOMPOSITION,
    operation="decompose_document",
    metadata={"strategy": "semantic"}
)

# Query derivations
derivations = provenance.get_derivations(target_key="fragment:abc123")
```

## Core Concepts

### Fragment Model (Two Layers)

**Storage Layer** (`ikam.graph.Fragment`):
- Content-addressable storage with BLAKE3 hashing
- Fields: `id`, `bytes`, `mime_type`, `size`
- Immutable once persisted

**Domain Layer** (`ikam.fragments.Fragment`):
- Hierarchical knowledge representation
- Fields: `artifact_id`, `level`, `type`, `content`, `radicals`, `salience`, `provenance`
- Ephemeral, materialized from storage + metadata

### Decomposition Strategies

- **SEMANTIC:** Context-aware chunking with meaning preservation
- **STRUCTURAL:** Respect document structure (headings, sections)
- **HYBRID:** Combine semantic and structural boundaries

### Salience Tiers

Fragments are ranked by importance:
- `CRITICAL` (1.0): Core insights, key metrics, executive summary
- `HIGH` (0.8): Important context, supporting data
- `MEDIUM` (0.6): Supplementary details
- `LOW` (0.4): Background, references
- `NEGLIGIBLE` (0.2): Boilerplate, formatting

## Mathematical Guarantees

### Storage Monotonicity
```
Δ(N) = S_flat(N) − S_IKAM(N) ≥ 0
```
Storage savings grow monotonically with N shared outputs.

### Fisher Information Dominance
```
I_IKAM(θ) ≥ I_RAG(θ) + Δ_provenance(θ)
```
IKAM preserves more information than RAG via complete provenance.

### Lossless Reconstruction
```
reconstruct(decompose(A)) = A
```
Byte-level equality for all supported formats.

## Supported Formats

- ✅ **Documents:** Markdown, plain text
- ✅ **Spreadsheets:** Excel (.xlsx), CSV
- ✅ **Presentations:** Slides (planned)
- ⏳ **Code:** Python, JavaScript (planned)

## Usage Examples

### Demo: 3-Output Deduplication

See `scripts/ikam/ikam_demo_three_outputs.py` for a runnable example showing:
- Decomposition of 3 outputs (document, deck, summary)
- CAS deduplication (shared fragments stored once)
- Provenance recording (DECOMPOSITION + REUSE edges)
- Fisher Information calculation
- Storage savings summary

```bash
PYTHONPATH=packages/ikam/src \
python scripts/ikam/ikam_demo_three_outputs.py \
  --db postgresql://narraciones:narraciones@localhost:5432/narraciones
```

### Integration with Modelado

The `modelado` package provides adapters for IKAM:

```python
from modelado.adapters import domain_to_storage, storage_to_domain
from ikam.fragments import Fragment as DomainFragment

# Convert domain fragment to storage record
storage_frag = domain_to_storage(domain_fragment)

# Convert storage record back to domain fragment
domain_frag = storage_to_domain(storage_frag, metadata)
```

## API Reference

### Core Functions

- `decompose_document(content, artifact_id, config)` → List[Fragment]
- `reconstruct_document(fragments, config)` → str
- `decompose_sheet(workbook, config)` → List[Fragment]
- `reconstruct_sheet(fragments, config)` → Workbook

### Models

- `Fragment` (domain layer)
- `Fragment` (storage layer)
- `DecompositionConfig`
- `ReconstructionConfig`
- `DerivationRecord`
- `SalienceTier`

### Enums

- `DecompositionStrategy` (SEMANTIC, STRUCTURAL, HYBRID)
- `DerivationType` (DECOMPOSITION, DELTA, REUSE)
- `FragmentLevel` (ARTIFACT, DOCUMENT, SECTION, PARAGRAPH, SENTENCE)

## Testing

```bash
# Run all IKAM tests
pytest packages/ikam/tests/ -v

# Run E2E pipeline tests (requires Postgres)
PYTHONPATH=packages/ikam/src \
  TEST_DATABASE_URL=postgresql://narraciones:narraciones@localhost:5432/narraciones \
  pytest packages/ikam/tests/test_e2e_ikam_pipeline.py -v
```

## Documentation

- [Storage Gains Example](../../docs/ikam/STORAGE_GAINS_EXAMPLE.md)
- [Fisher Information Gains](../../docs/ikam/FISHER_INFORMATION_GAINS.md)
- [Mutation and Variation Model](../../docs/ikam/MUTATION_AND_VARIATION_MODEL.md)
- [Almacén Backend](ALMACEN_README.md)
- [Fragment Codec](FRAGMENTCODEC_README.md)

## Dependencies

**Core:**
- `pydantic >= 2.0` — Data validation and serialization

**Optional:**
- `blake3` — Fast hashing (fallback to SHA-256 if unavailable)

**Zero internal dependencies** — IKAM is Layer 0 and stands alone.

## Contributing

IKAM is part of the Narraciones monorepo. See [AGENTS.md](../../AGENTS.md) for development guidelines.

**Key principles:**
- Preserve mathematical guarantees (storage monotonicity, FI dominance, lossless reconstruction)
- Maintain zero internal dependencies (Layer 0 constraint)
- Add tests for all new decomposition strategies
- Document provenance completeness for new operations

## License

Proprietary (internal use). See [LICENSE](../../LICENSE).

## Version

Current: **0.2.0** (December 2025)

See [CHANGELOG.md](CHANGELOG.md) for release history.
