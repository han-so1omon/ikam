# IKAM v2: Mutation Deltas and Render Variations

Date: 2025-11-16
Status: Proposed — documentation and planning only (no code changes)

## 1. Overview

This document extends the IKAM v2 model with two capabilities:

- Mutation tracking with deltas: succinctly represent small changes between representations of the same core idea across artifacts.
- Non-deterministic render/combination: allow a core idea to yield multiple unique outputs (e.g., different phrasings, subtitles) while preserving provenance and mathematical guarantees.

Both features must preserve IKAM’s core guarantees:
- Lossless Reconstruction: reconstruct(decompose(A)) = A (byte-level equality) for stored artifacts and fragments.
- Monotonic Storage Gains: Δ(N) = S_flat(N) − S_IKAM(N) is non-decreasing in N.
- Fisher Information Dominance: I(A,Y; θ) ≥ I(A; θ) with Δ_provenance_FI := E[I(Y; θ | A)] ⪰ 0.

## 2. Mutation Tracking with Deltas

### 2.1 Model

Let base fragment content be X (stored once via CAS). Let V_j denote a variant used by artifact j. We encode V_j as (X, Δ_j) where Δ_j is a deterministic delta/patch that transforms X → V_j.

- Storage: cost(V_j) = size(Δ_j) + overhead_patch; base cost size(X) is paid once.
- Provenance: record edge X --(delta Δ_j, schema v)--> V_j with metadata: delta algorithm, version, and semantic tags (optional).
- Reconstruction: apply Δ_j to X deterministically to obtain V_j.

### 2.2 Storage Impact

Let S_X be size(X), δ̄ = E[size(Δ_j)] and κ be average patch metadata. For M variants derived from the same base:

- Flat: S_flat = M·(S_X + structure)
- IKAM with deltas: S_IKAM = S_X + M·(δ̄ + κ + structure_ref)

Savings vs storing M full copies: Δ ≈ M·S_X − [S_X + M·δ̄] − M·(κ + ref_gap)

Break-even per-variant when δ̄ + κ + ref_gap < S_X. As δ̄ → 0 (tiny edits), savings approach M·S_X − S_X.

Early convexity: When multiple artifacts reuse the same high-salience base X with tiny deltas, Δ(N) can grow faster than linear initially.

### 2.3 Fisher Information Impact

Let θ denote content parameters. Let Δ be the delta random variable induced by editorial choices conditional on X. Then the joint Fisher Information satisfies the identity

I((X,Δ); θ) = I(X; θ) + E[ I(Δ; θ | X) ].

- If Δ depends on θ (e.g., content-specific formatting that encodes θ-relevant structure), then Δ_provenance_FI^Δ := E[I(Δ; θ | X)] ⪰ 0 adds nonnegative FI.
- If Δ ⫫ θ | X (pure formatting unrelated to θ), then E[I(Δ; θ | X)] = 0, so FI is unchanged relative to storing X only; provenance still does not reduce FI.

Conclusion: Deltas never reduce FI and may increase it when delta semantics correlate with θ.

### 2.4 Example: Doc vs Slide Delta

Core sentence X (doc): “LTV/CAC is 7.1 with 23% inventory savings.”

Slide variant V (bullet): “LTV/CAC 7.1; inventory −23%.”

- Base X stored once; slide V stored as Δ shortening phrases and adding symbols.
- Typical δ sizes (UTF-8 patch) ≈ 10–25% of S_X.
- Storage: For 1 doc + 3 slide variants across decks, storing 3 small Δ_j beats 3 full copies.
- FI: If the symbols (e.g., ‘−’) are purely formatting, FI w.r.t. θ (economic parameters) is unchanged; if we tag Δ with normalization/rounding operations used, that metadata can increase FI via better estimation procedures downstream.

## 3. Non-Deterministic Render/Combination

### 3.1 Model

We introduce a render variation variable Z capturing non-deterministic choices at render time (e.g., phrasing A/B, subtitle variants, layout alternates). We record:

- seed: 64-bit value used by the renderer
- renderer_version: implementation identifier
- policy_id: selection policy or audience profile
- variation_id: stable identifier for a generated variant

Outputs A_Z remain deterministic given (X, provenance Y, Z, renderer_version). Provenance records Z (or a hash) so the exact output can be reproduced.

### 3.2 Fisher Information with Dual Parameters

Let θ be content parameters (economics) and φ be audience/style parameters (render policy). Assume Z ⫫ θ,φ unless chosen via policy.

- If Z is independent of θ (and φ), then
  I((A,Z); θ) = I(A; θ) + E[I(Z; θ | A)] = I(A; θ).
  Non-determinism does not change FI about θ.

- If Z depends on φ but not θ, then FI about θ is unchanged; mutual information about φ increases: I((A,Z); φ) ≥ I(A; φ).

- If policy ties Z to content (Z depends on features of X influenced by θ), then
  I((A,Z); θ) = I(A; θ) + E[I(Z; θ | A)] ≥ I(A; θ),
  adding Δ_provenance_FI^Z ≥ 0. Strict increase occurs when p(z | A, θ) depends non-trivially on θ.

Block-structure: If (Z ⫫ θ | A) and (Z ⫫ φ | A) fail, cross-terms appear; otherwise the FI matrix w.r.t. (θ, φ) is block-diagonal.

### 3.3 Storage Impact

Non-deterministic variants that differ only in micro-phrasing can be stored as deltas relative to a canonical variant X* chosen per fragment:

- Canonicalization: choose X* deterministically (e.g., seed=0) for each fragment.
- Each realized variant stores Δ_Z relative to X*.
- Provenance keeps (seed, renderer_version, policy_id, variation_id) so exact reproduction is possible.

This reduces per-variant storage to size(Δ_Z) + small metadata, maintaining monotonic savings.

### 3.4 Example: Subtitles Variations for Visual Slide

Core idea: “Reduce stockouts and rush shipping” displayed under a chart.

Variants:
- Z1: “Fewer stockouts. Less rush shipping.”
- Z2: “Cut stockouts; curb rush shipments.”
- Z3: “Stockouts down; rush shipping down.”

- Store canonical subtitle X* and deltas Δ_Z1, Δ_Z2, Δ_Z3.
- Provenance records seeds or variation_ids for reproducibility.
- FI about θ (economic impact) unchanged if phrasing differences are stylistic; MI about φ (audience style) increases.

## 4. Constraints and Best Practices

- Determinism boundary: Decomposition and reconstruction remain deterministic. Non-determinism is allowed only in rendering/combination and must be fully recorded in provenance.
- Delta chains: Bound maximum patch chain length L (e.g., L ≤ 3); periodically rebase to keep access latency predictable.
- Canonical variant: Maintain a deterministic canonical base per fragment for delta storage.
- Provenance completeness: Record (delta algorithm, version, semantic tags) and (seed, renderer_version, policy_id, variation_id) where applicable.
- Testing: Round-trip with deltas; reproduce non-deterministic variants given recorded Z; verify FI non-decrease against θ; measure storage savings empirically across N.

## 5. Implications for IKAM Guarantees

- Storage monotonicity: Deltas reduce per-variant unique bytes, typically lowering break-even N and increasing slope of Δ(N) once reuse begins.
- Fisher information: Additional variables (Δ, Z) can only add nonnegative FI about θ via conditional terms E[I(Δ; θ | X)] and E[I(Z; θ | A)], and are zero when independent of θ.
- Lossless reconstruction: Guaranteed by deterministic application of deltas and recording of render randomness (Z) in provenance.

## 6. Acceptance Criteria (Docs & Tests)

- Docs updated to describe delta storage model and render variation model, with math consistent with joint FI identity.
- Add test plans: (a) delta round-trip and chain length bound; (b) variant reproduction from recorded Z; (c) FI comparison on synthetic θ-sensitive deltas vs θ-independent deltas.
- Update INDEX and AGENTS guardrails to include these capabilities and constraints.
