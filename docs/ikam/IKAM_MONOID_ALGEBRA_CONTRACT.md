# IKAM Monoid Algebra Contract

Date: 2026-02-07
Status: Draft Contract (normative for implementation and verification; amended 2026-02-08)
Scope: Corpus decomposition, fragment identity, graph composition, query grounding, replay/publication

---

## 1) Purpose

This contract defines the full algebraic and operational behavior of IKAM corpus fragmentation.

Primary intent:

- Preserve all meaningful shared information via scoped fragment reuse (within-case by default).
- Avoid duplicate semantic units represented as separate fragments with link-only reconciliation.
- Maintain strict replayability and provenance completeness even when decomposition or generation contains stochastic/effectful steps.

This document is normative where it uses MUST/SHALL language.

---

## 2) Core Model

### 2.1 Sets and Functions

- `A`: set of artifact versions.
- `U`: set of extracted candidate units from artifacts.
- `B`: canonical byte strings.
- `F`: CAS fragment identities (`fid`).
- `M`: manifests (ordered fragment-reference compositions).
- `Q`: natural-language queries.
- `E`: evidence subgraphs returned by retrieval.

Functions:

- `extract: A -> P(U)`
- `C: U -> B` (deterministic canonicalization)
- `H: B -> F` (content hash identity)
- `manifest: A -> M`
- `R: M -> bytes` (deterministic reconstruction)

Derived identity rule:

- `fid(u) = H(C(u))`

### 2.2 Equivalence Relation

Define `u1 ~ u2` iff `C(u1) = C(u2)`.

`~` is the semantic identity relation for storage and reuse. Two units equivalent under `~` MUST map to the same fragment ID.

---

## 3) Monoid Semantics

### 3.1 Artifact Composition Monoid

Treat each artifact manifest as an ordered composition over fragment references.

- Carrier: `M`
- Operator: `⊕` (manifest composition / concatenation with structural metadata merge)
- Identity: `epsilon` (empty manifest)

Laws:

1. Associativity: `(m1 ⊕ m2) ⊕ m3 = m1 ⊕ (m2 ⊕ m3)`
2. Identity: `m ⊕ epsilon = epsilon ⊕ m = m`

Notes:

- Ordering is part of manifest semantics.
- Structural placement metadata is part of monoid elements, not external side channels.

### 3.2 Membership-Defined Sharedness

Cross-artifact sharedness is defined by shared manifest membership of the same `fid`, not by post-hoc equivalence links between duplicate fragments.

Scope note (2026-02-08 amendment):

- Benchmark/evaluation cases are independent by default.
- Reuse optimization target is within-case unless an explicit semantic-approval merge path is used.
- Cross-case sharing is opt-in through approved merge flows; it is not a default decomposition objective.

---

## 4) Axioms (System Invariants)

### A. Identity and Storage

- **A1 Canonical Identity:** `C(u1)=C(u2) => H(C(u1))=H(C(u2))`.
- **A2 CAS Immutability:** stored bytes for a `fid` are immutable.
- **A3 No Duplicate Semantic Units:** system SHALL NOT store two distinct `fid`s with identical canonical bytes.
- **A4 Scoped Shared Fragment Reuse:** if artifacts within the same corpus scope contain equivalent units under `~`, manifests SHALL reference the same `fid`. Cross-scope (for example cross-case) reuse is opt-in and requires explicit semantic approval policy.

### B. Composition and Reconstruction

- **B1 Manifest Monoid:** `(M, ⊕, epsilon)` satisfies associativity and identity.
- **B2 Deterministic Reconstruction:** for fixed manifest + referenced fragment objects, `R` is deterministic.
- **B3 Lossless-Supported Types:** for supported types, `R(manifest(A)) = bytes(A)`.
- **B4 Append-Only Versioning:** new artifact states are new manifests/heads; prior heads remain valid.

### C. Provenance and Query Grounding

- **C1 Provenance Closure:** all answer claims must map to concrete fragment IDs and derivation paths.
- **C2 Fail-Closed:** missing required provenance/evidence SHALL produce explicit failure, not degraded uncited output.
- **C3 Universal Query Path:** all queries SHALL use the same generic retrieval and reasoning pipeline (no business-specific intent path).

### D. Effectful Realizations

- **D1 Materialized Outcome Identity:** every effectful outcome is canonicalized and assigned `fid_out = H(C(outcome))`.
- **D2 Realization Record Completeness:** each effectful realization SHALL persist full replay metadata.
- **D3 Replay Determinism:** replay from persisted realization records + manifests is deterministic.

---

## 5) Decomposition Policy Contract

### 5.1 Policy Goals

Decomposition SHALL maximize reusable identity while preserving reconstruction fidelity.

### 5.2 Mandatory Pipeline Stages

1. **Artifact Intake:** create deterministic ingestion manifest (path, mime, hash, size, timestamp).
2. **Unit Extraction:** parse into candidate units (modality-aware).
3. **Canonicalization:** apply deterministic normalization to each unit.
4. **Identity Resolution:** compute `fid = H(C(unit))`.
5. **Manifest Assembly:** compose artifact manifest with ordered references and placement metadata.
6. **Provenance Emission:** persist extraction/canonicalization/assembly traces.

### 5.3 Shared Information Rule

The system SHALL NOT restrict sharedness to a predefined class taxonomy.

Any information that canonicalizes identically, regardless of modality or semantic label, MUST reuse the same fragment identity.

### 5.4 Ingestion Exclusion Rule

Evaluation-side oracle files (e.g., `idea.md`) MUST be excluded from initial corpus ingestion and used only by evaluation harnesses.

---

## 6) Fragment Identity Policy

### 6.1 Identity Inputs

Fragment identity is a function only of canonical content bytes.

Identity computation MUST exclude ephemeral fields:

- runtime IDs,
- artifact IDs,
- timestamps,
- parent pointers,
- transport/storage offsets.

### 6.2 Identity Stability

- Equal canonical bytes => equal `fid`.
- Unequal canonical bytes => distinct `fid` (collision resistance assumed by hash choice).

### 6.3 Parent/Structure Separation

Hierarchy and structure are represented in manifests/metadata edges, not in fragment identity bytes.

---

## 7) Retrieval and Answer Contract

### 7.1 Generic Retrieval Pipeline

For any query `q in Q`, retrieval SHALL use the same pipeline:

1. candidate generation (lexical, semantic, topology),
2. score normalization and fusion,
3. evidence-subgraph extraction,
4. grounded answer synthesis from evidence only.

### 7.2 Explanation Contract

Every ranked result SHALL include traceable contributions:

- per-signal scores,
- normalized fusion weights,
- selected evidence paths,
- contradiction/drift indicators where applicable.

### 7.3 No Domain Hardcoding

Query answering SHALL NOT depend on hardcoded business-domain fields or intent branches for specific question types.

---

## 8) Deterministic vs Stochastic Modes

### 8.1 Deterministic Mode (benchmark/strict)

Requirements:

- identical corpus + config + seed => identical manifests,
- stable ranking ties per deterministic policy,
- exact replay and byte equality for lossless-supported artifacts.

### 8.2 Stochastic Mode (production AI-informed)

Allowed:

- variable decomposition boundaries,
- variable intermediate reasoning paths.

Required:

- deterministic fragment identity per emitted unit,
- stable quality bands (coverage, grounded precision, contradiction handling),
- deterministic replay from persisted state.

Bounded non-determinism is acceptable only before publication freeze.

---

## 9) Replay and Publication Properties

### 9.1 Freeze-on-Publish

Publication SHALL bind outputs to:

- immutable artifact head/manifests,
- exact fragment references,
- exact realization records for effectful steps.

### 9.2 Deterministic Replay Envelope

Replay inputs SHALL include:

- manifest/object IDs,
- renderer/operator version IDs,
- policy snapshot IDs,
- effectful realization metadata.

### 9.3 Post-Publish Stability

Published outputs MUST remain reproducible despite future model/provider/version changes.

---

## 10) Effectful Operation Rules

### 10.1 Effectful Operator Abstraction

An effectful operator is `g(spec, inputs, policy, exogenous_context) -> outcome`.

Every execution MUST persist a realization record `rho` with at least:

- operator specification/version,
- input references (artifact/fragment IDs),
- policy snapshot,
- exogenous fingerprint,
- outcome fragment ID,
- verifier data,
- timestamp.

### 10.2 Invariant Gate

Outcome admission to publishable manifests requires invariant checks over declared constraints. Failed checks MUST fail closed.

### 10.3 No Silent Regeneration

The system SHALL NOT silently regenerate effect outcomes for already frozen/published manifests.

---

## 11) Property Test Suite (Normative)

### 11.1 Identity and Dedup

- **P-ID-1:** equal canonical bytes produce equal `fid`.
- **P-ID-2:** no duplicate canonical bytes under different `fid`s.
- **P-ID-3:** cross-artifact overlap yields shared `fid` membership.

### 11.2 Monoid and Reconstruction

- **P-MON-1:** associativity of manifest composition.
- **P-MON-2:** identity element behavior.
- **P-REC-1:** lossless round-trip for supported types.
- **P-REC-2:** unchanged content across versions reuses same `fid`s.

### 11.3 Query Grounding

- **P-Q-1:** universal generic path used for all query texts.
- **P-Q-2:** every answer claim has fragment-backed evidence.
- **P-Q-3:** fail-closed on missing required evidence/provenance.

### 11.4 Stochastic Quality

- **P-S-1:** coverage threshold met across reruns.
- **P-S-2:** grounded precision threshold met across reruns.
- **P-S-3:** reuse efficiency remains within configured bands.

### 11.5 Effectful Replay

- **P-E-1:** realization record completeness.
- **P-E-2:** replay from persisted records reproduces published output.
- **P-E-3:** invariant gate rejects invalid outcomes.

---

## 12) Operational Checks (SLO/Guardrails)

Track at minimum:

- `dedup_ratio`
- `shared_fid_reuse_count`
- `manifest_reuse_rate`
- `provenance_closure_rate`
- `replay_match_rate`
- `grounded_precision`
- `evidence_coverage`
- `invariant_reject_rate`

Release gate minimums (recommended defaults):

- `provenance_closure_rate = 100%`
- `replay_match_rate = 100%` (published outputs)
- `grounded_precision >= 0.80` on eval corpus
- `evidence_coverage >= 0.70` on eval corpus

---

## 13) Parser Trace Contract

Each decomposition/query operation SHOULD emit machine-readable trace events containing:

- operation ID + mode (`deterministic` or `stochastic`),
- input refs and config/policy snapshots,
- candidate units before/after canonicalization,
- identity decisions (`unit -> fid`),
- scoring signal contributions,
- selected evidence paths,
- final output refs.

Trace records are mandatory for debugging quality regressions and proving behavior.

---

## 14) Anti-Patterns (Explicitly Forbidden)

- Creating duplicate fragments for equivalent canonical content and reconciling with equivalence-link nodes.
- Special query route for specific domain question classes.
- Publishing answers without complete fragment-level provenance.
- Silent fallback from semantic/evidence path to uncited generic text.
- Non-replayable effectful outputs (missing realization metadata).

---

## 15) Compliance Checklist

Before implementation is accepted:

- [ ] Axioms A1-A4, B1-B4, C1-C3, D1-D3 are implemented and tested.
- [ ] `idea.md` (and equivalent oracles) excluded from ingestion path.
- [ ] Deterministic and stochastic mode property suites both pass.
- [ ] Replay/publication and effectful realization tests pass at 100%.
- [ ] Operational metrics wired and visible in CI/runtime checks.

---

## 16) Relationship to Existing IKAM Docs

This contract complements (does not replace):

- `docs/ikam/IKAM_FRAGMENT_ALGEBRA_V3.md`
- `docs/ikam/ikam-fragmentation-math-model.md`
- `docs/ikam/MUTATION_AND_VARIATION_MODEL.md`
- `docs/ikam/graph-model.md`
- `docs/ikam/ikam-v2-fragmented-knowledge-system.md`

In any conflict for corpus fragmentation and identity semantics, this contract governs implementation behavior.

---

## 17) V3 Algebra Transition Addendum (2026-02-08)

This section records the agreed target direction for the next algebra revision. It is normative for upcoming migration planning and conformance updates.

### 17.1 Core Types

- Fragment is the universal container with minimal fields: CAS reference and/or inline value plus MIME interpretation.
- Relation is represented as a fragment with MIME type `application/ikam-relation+json`.
- Artifact is a named entry point that references a root fragment for DAG evaluation.
- ProvenanceEvent remains append-only evidence for operations and evaluations.

### 17.2 Relation Execution Semantics

- Relations MAY include a function reference and one or more binding groups (slot-to-fragment assignments).
- A single relation can support multiple invocation groups without duplicating relation definitions.
- Function outcomes MAY be deterministic or non-deterministic.
- For non-deterministic outcomes, reproducibility context MUST be recorded in provenance metadata.

### 17.3 Evaluation History and Materialized Views

- Evaluation history is a view over provenance proofs, not separate authoritative state.
- Systems SHOULD maintain a materialized projection for latest evaluated output per `(relation_fragment, invocation_id)` to support low-latency reconstruction.
- The provenance event log remains the source of truth for audit and replay.

### 17.4 Deletions/Compression Targets

Planned consolidation in V3:

- Derivation and Variation records are absorbed into relation semantics plus provenance events.
- Legacy fragment content type proliferation should be collapsed into MIME-driven interpretation.
- Structural placement and retrieval annotations should not be encoded as fragment identity bytes.
