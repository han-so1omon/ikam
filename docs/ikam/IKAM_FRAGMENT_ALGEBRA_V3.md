# IKAM Fragment Algebra V3

Date: 2026-02-08  
Status: Draft (normative target for migration and conformance)  
Supersedes (in scope): legacy domain fragment shape in `packages/ikam/src/ikam/fragments.py`

---

## 1) Purpose

IKAM Fragment Algebra V3 defines a minimal, algebra-first representation for knowledge and reconstruction.

Design goals:

- Minimize the core type system.
- Preserve content-addressed identity and replayability.
- Unify relational semantics and reconstruction semantics.
- Make evaluation history a projection of provenance proofs, not duplicated mutable state.
- Optimize reuse by default within corpus scope (benchmark case scope by default), with cross-scope reuse only via explicit semantic approval.

This document specifies:

- Core types and invariants.
- Relation execution semantics (deterministic and non-deterministic).
- Artifact reconstruction from root relation fragments.
- Provenance projection model for latest evaluation lookup.
- Migration and deletion plan from legacy models.

---

## 2) Core Type System

V3 keeps three first-class models:

1. `Fragment` (universal content container)
2. `Artifact` (named entry point into a fragment DAG)
3. `ProvenanceEvent` (append-only proof log)

All higher-level semantics are encoded through MIME-typed fragment payloads and graph structure.

### 2.1 Fragment

```python
class Fragment(BaseModel):
    fragment_id: Optional[str] = None
    cas_id: Optional[str] = None
    value: Optional[Any] = None
    mime_type: Optional[str] = None
```

Valid states:

- Graph-id-only: `fragment_id` set, `cas_id` and `value` absent.
- CAS-only: `cas_id` set, `value` absent.
- Inline-only: `value` set, `cas_id` absent.
- Dual: both set (inline cache + identity binding).

Fragment invariants:

- F1. At least one of `fragment_id`, `cas_id`, or `value` SHALL be present.
- F2. If both are present, canonicalization of `value` SHALL hash to `cas_id`.
- F3. `cas_id` identity SHALL exclude transport/runtime metadata.

Storage boundary:

- `ikam.fragments.Fragment` is the primary public/runtime fragment type.
- `ikam.graph.StoredFragment` is the storage-only CAS record type.
- `ikam.graph.Fragment` is not part of the clean-break public boundary.

### 2.2 Relation as Fragment

Relations are not a peer type. A relation is a fragment with MIME:

- `application/ikam-relation+json`

Its payload conforms to:

```python
class SlotBinding(BaseModel):
    slot: str
    fragment_id: str


class BindingGroup(BaseModel):
    invocation_id: str
    slots: list[SlotBinding]


class Relation(BaseModel):
    predicate: str
    directed: bool = True
    confidence_score: float = 0.80
    qualifiers: dict[str, Any] = {}

    # None => pure semantic relation
    function_cas_id: Optional[str] = None
    output_mime_type: Optional[str] = None

    # Multiple invocation slot-sets for one relation definition
    binding_groups: list[BindingGroup] = []
```

### 2.3 Artifact

```python
class Artifact(BaseModel):
    id: str
    kind: str
    title: Optional[str] = None
    root_fragment_id: str
    created_at: datetime
```

Artifact points to one root fragment. Reconstruction evaluates from this root.

### 2.4 ProvenanceEvent

Existing append-only model is retained. Evaluation history is represented as `action="evaluated"` events with structured metadata.

---

## 3) Algebraic Semantics

### 3.1 Carrier Sets

- `F`: set of fragment identities (`cas_id` domain)
- `A`: set of artifacts
- `P`: set of provenance events
- `R`: subset of `F` where fragment payload MIME is relation JSON

### 3.2 Identity

- Canonicalization function `C: value -> bytes`
- Hash `H: bytes -> cas_id`
- If fragment has inline value and identity: `cas_id = H(C(value))`

### 3.3 Relation Execution

For relation fragment `r in R` with payload relation `rel` and invocation `g`:

`eval(r, g, env) = out`

where:

- `rel.function_cas_id` resolves to executable function spec fragment.
- `g.slots` bind named inputs to source fragment identities.
- `env` is reproducibility context (may be empty).

Evaluation outcome is persisted by provenance event:

- `fragment_id = r`
- `action = "evaluated"`
- metadata includes `invocation_id`, `environment`, `output_cas_id`

### 3.4 Deterministic vs Non-Deterministic Functions

- Deterministic function: same slots + same function spec => same output bytes.
- Non-deterministic function: output may vary unless environment is fixed.

Required rule:

- If non-determinism exists, environment metadata SHALL be sufficient for replay of the published/frozen outcome.

---

## 4) Reconstruction Semantics

Given artifact `a`, reconstruct by evaluating DAG rooted at `a.root_fragment_id`.

Algorithm (high level):

1. Load root fragment.
2. If root MIME is not relation MIME, return its represented value/bytes.
3. If relation with `function_cas_id`:
   - For each binding group, resolve latest `output_cas_id` from provenance materialized view.
   - If latest exists, load output.
   - Else evaluate function, write output fragment, append provenance event.
4. Compose outputs according to root function semantics.

Purity boundary:

- Functional semantics are in relation/function fragments.
- Execution history is in provenance events.
- Latest-output acceleration is projection infrastructure, not source-of-truth data.

---

## 5) Provenance Projection Model

Evaluation history is a view over proofs.

Normative behavior:

- Source of truth: append-only `ProvenanceEvent` log.
- Fast lookup: materialized projection keyed by `(relation_fragment_id, invocation_id)` returning latest `output_cas_id` and metadata.

Minimum projection contract:

- Query API: `latest_output(relation_fragment_id, invocation_id) -> Optional[output_cas_id]`
- Ordering key: provenance timestamp (or monotonic sequence).
- Rebuildability: projection can be reconstructed from provenance log only.

Implementation note:

- Any stream/materialization engine is valid if it preserves ordering and rebuildability (for example, event-stream projections or SQL materialized patterns).

---

## 6) Scope and Reuse Policy

Default scope policy:

- Case-scoped reuse optimization is default for benchmark/eval corpora.
- Cross-case reuse is disabled by default.
- Cross-case merge/reuse is opt-in and requires explicit semantic approval path.

Metric semantics:

- Primary dedup metric for benchmark reports SHALL be within-case reuse efficiency.
- Cross-case reuse MAY be reported only as separate, explicitly labeled merge-mode metric.

---

## 7) Legacy Model Compression Map

The following legacy constructs are compressed into V3:

- `Derivation` -> relation structure + provenance events.
- `Variation` -> provenance environment metadata.
- `RadicalRef` -> relation `function_cas_id` + binding groups.
- `FragmentType` enum and specialized content classes -> MIME-driven interpretation.
- `level`, `artifact_id`, `salience`, parent pointers on core fragment identity -> moved to graph/projection/annotation layers as needed.
- storage-layer CAS records -> represented separately as `ikam.graph.StoredFragment`.

---

## 8) Migration Plan (Clean Break, Name Preservation)

Target requirement from design agreement:

- New model keeps the canonical name `Fragment`.
- Legacy domain/storage fragment models are temporarily renamed and deleted in planned phases.

### 8.1 Phase M1: Coexistence

- Introduce V3 `Fragment` in new module path.
- Rename legacy domain fragment to `LegacyFragment`.
- Rename legacy storage fragment to `LegacyStorageFragment`.
- Add compatibility adapters for old readers/writers.

### 8.2 Phase M2: Dual-path execution

- Reconstructor supports V3 root-fragment DAG path and legacy manifest path.
- Decomposer can emit V3 fragments and relation payloads under feature flag.
- Provenance events emitted for both paths.

### 8.3 Phase M3: Cutover

- Artifact writes switch to `root_fragment_id` semantics.
- Benchmarks and APIs default to V3.
- Materialized latest-evaluation projection enabled as standard.

### 8.4 Phase M4: Deletion

- Remove legacy fragment content class hierarchy.
- Remove derivation/variation persistence models and tests.
- Remove radical-ref-only execution paths.
- Remove legacy adapters after conformance green.

Deletion is complete only when V3 conformance matrix passes and legacy references are zero.

---

## 9) Conformance Test Requirements

Conformance updates MUST align with:

- `docs/ikam/IKAM_MONOID_ALGEBRA_CONTRACT.md`
- `docs/ikam/IKAM_MONOID_TEST_MATRIX.md`

Required V3 additions (minimum):

1. Minimal fragment state invariants (CAS-only, inline-only, dual).
2. Relation MIME payload round-trip.
3. Multiple binding groups for one relation.
4. Deterministic and non-deterministic evaluation behavior with environment capture.
5. Provenance-derived latest-evaluation projection correctness.
6. Artifact reconstruction from root relation DAG.
7. Within-case reuse metrics and explicit cross-case exclusion by default.

---

## 10) Integration with Existing IKAM Documentation

This document is designed to be read with:

- `docs/ikam/IKAM_MONOID_ALGEBRA_CONTRACT.md` (normative monoid and invariant clauses)
- `docs/ikam/IKAM_MONOID_TEST_MATRIX.md` (traceable test obligations)
- `docs/ikam/ikam-fragmentation-math-model.md` (mathematical context)
- `docs/ikam/MUTATION_AND_VARIATION_MODEL.md` (effectful/non-deterministic framing)

Conflict resolution order for V3 migration:

1. `IKAM_MONOID_ALGEBRA_CONTRACT.md` (normative contract)
2. `IKAM_FRAGMENT_ALGEBRA_V3.md` (structural/type migration target)
3. `IKAM_MONOID_TEST_MATRIX.md` (verification obligations)

When these docs diverge, update all three in one change-set.

---

## 11) Open Decisions (Explicitly Tracked)

1. Function-spec representation format (`function_cas_id` payload schema).
2. Relation payload schema versioning and backward compatibility strategy.
3. Projection implementation choice for latest evaluation lookup.
4. API migration strategy for clients expecting legacy fragment fields.

No implementation should silently choose defaults for these decisions in production paths without recording policy in provenance/config snapshots.

---

## 12) Within-Case Reuse Report Redesign

This section defines the required redesign of reuse reporting for benchmark/eval corpora.

### 12.1 Problem Statement

Legacy report behavior measured dedup/reuse globally across all cases. That is incorrect for current policy because benchmark cases are independent by default.

Required correction:

- Primary reuse metrics MUST be computed within each case boundary.
- Cross-case reuse MUST NOT be counted in primary metrics.
- Cross-case merge/reuse MAY be reported only as separate, explicitly labeled opt-in mode.

### 12.2 Required Reporting Modes

1. `within_case` (default, normative)
2. `cross_case_merge` (optional, explicit semantic approval path)

`within_case` report MUST include, per case:

- total emitted fragments,
- unique fragment identities,
- within-case dedup ratio,
- top reused fragments within case,
- relation-function reuse profile (number of invocations per relation fragment),
- scalar fragment reuse profile.

Aggregate metrics MAY summarize across cases, but only by aggregating per-case results (never by building a global identity pool first).

### 12.3 Semantic Breakdown Planning Step (Mandatory)

Before decomposition/report generation, the system SHALL execute a planning step per case to maximize within-case reuse.

Planning outputs (machine-readable):

- decomposition strategy per artifact modality,
- candidate reusable units (template, delta, algebraic composition opportunities),
- relation function candidates and expected slot schemas,
- expected reuse hotspots and acceptance thresholds.

This planning artifact becomes an input to decomposition and must be referenced in provenance.

### 12.4 Metric Definitions

Per case `c`:

- `N_total(c)`: total fragment references emitted for case artifacts
- `N_unique(c)`: unique `cas_id` count within case
- `reuse_ratio(c) = 1 - (N_unique(c) / N_total(c))`

Case-weighted aggregate:

- `reuse_ratio_weighted = 1 - (sum_c N_unique(c) / sum_c N_total(c))`

Important:

- `N_unique(c)` is computed with a case-local identity set.
- Identical CAS IDs appearing in different cases do not reduce `N_unique(c)` for any other case.

### 12.5 Output Contract for Report Generator

The report generator should emit:

1. Per-case tables and charts (normative)
2. Optional merge-mode appendix (non-default)
3. Explicit mode header and policy statement

Required header fields:

- `mode: within_case | cross_case_merge`
- `scope_policy: case_isolated_default`
- `cross_case_reuse_in_primary_metrics: false`

### 12.6 Test Obligations

Minimum tests for the redesigned report:

- Case isolation test: identical units in two cases do not affect each other's primary dedup metrics.
- Planning enforcement test: report generation fails closed if semantic breakdown plan is missing.
- Merge-mode separation test: cross-case metrics appear only in merge appendix/mode.
- Oracle exclusion test: `idea.md` excluded from ingestion and reuse accounting.
