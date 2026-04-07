# IKAM Staging and Process Model

**Version:** 1.0 (2026-02-09)
**Status:** Canon
**Purpose:** Defines the processes for graph manipulation and the transactional guarantees of the Staging Area.

## 1. The Unified Fragment Graph

The IKAM system operates on a single **Unified Fragment Graph** where all data—whether raw bytes, semantic concepts, or extracted entities—is represented as typed `Fragment` nodes connected by typed `Relation` edges.

- **Source Fragments:** Exact byte sequences (`text/plain`, `application/pdf`). Reconstructible.
- **Concept Fragments:** Normalized semantic propositions (`application/ikam-concept+json`). Dense.
- **Entity Fragments:** Knowledge graph nodes (`application/ikam-entity+json`). Connected.

There are NO layers. All fragments coexist in the same CAS namespace.

## 2. Core Processes

Processes operate on the graph to increase density, connectivity, or utility. They are composable and non-destructive.

### Process A: Structural Chunking
Splits artifacts into logical sub-units (paragraphs, sections) based on syntax or visual structure.
- **Goal:** Lossless reconstruction.
- **Determinism:** High. `reconstruct(root) == original_bytes`.

### Process B: Semantic Normalization
Transforms raw text into canonical semantic forms using AI models.
- **Goal:** Concept deduplication.
- **Determinism:** Low (mitigated by provenance freezing).
- **Mechanism:** Embedding clustering + LLM verification.

### Process C: Enrichment
Extracts structured knowledge (entities, relationships) from unstructured content.
- **Goal:** Discovery.
- **Mechanism:** Graph Transformation (LLM-driven).

### Process D: Artifact Composition
Assembles fragments into user-facing deliverables (Documents, Slide Decks, Answers).
- **Goal:** Delivery.
- **Mechanism:** DAG construction.

## 3. The Staging Area Contract

To maintain graph hygiene and prevent explosion from draft/failed operations, all mutations MUST occur in a **Staging Area**.

**Contract:**
1. **Isolation:** Staged fragments are invisible to the main graph until promoted.
2. **Identity Stability:** A fragment's ID is `BLAKE3(canonical_content)`. It is identical in Staging and Production.
3. **Atomic Promotion:** An Artifact version is promoted atomically with its full dependency closure.
4. **Discard on Failure:** If a process fails or is abandoned, the Staging Area is discarded. No orphaned fragments leak to Production.

**Implementation Strategy (Sidecar):**
- **Write:** Generate fragments to temporary storage (e.g., Redis, Temp Table).
- **Validate:** Verify graph constraints (acyclic, connected) in Staging.
- **Promote:** Copy the *reachable subgraph* from the new Artifact root to `ikam_fragments`.
- **Deduplicate:** Use `ON CONFLICT DO NOTHING` during promotion. If a fragment already exists in Production, the Staging copy is redundant and ignored.

## 4. References

- `docs/ikam/IKAM_FRAGMENT_ALGEBRA_V3.md`: Algebra definition.
- `docs/ikam/IKAM_MONOID_ALGEBRA_CONTRACT.md`: Axioms A1-A4, C1-C2.
- `docs/ikam/MUTATION_AND_VARIATION_MODEL.md`: Handling updates.
