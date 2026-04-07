# IKAM Agents Guide & Engineering Constitution

> **⚠️ CRITICAL: This file MUST remain in the repository root.**
> It serves as the canonical reference for automation agents and contributors.

## 1. Architectural Vision: The IKAM Monadic Kernel
IKAM (Incremental Knowledge Artifact Model) is a content-addressable, fragmented knowledge graph. 

* **Two-Layer Model:** Separate raw CAS storage (`ikam.graph.Fragment`) from semantic domain logic (`ikam.fragments.Fragment`). Use `modelado.adapters` for conversion.
* **Monadic Execution Kernel:** Fragments may store values, references, functions, or function references.
* **Mathematical Soundness:** Operations must guarantee lossless reconstruction and complete provenance.

## 2. Code Minimalism & Anti-Bloat Policy

1. Write less code (<100 lines per function when possible).
2. Avoid unnecessary abstractions.
3. No speculative features.
4. Delete unused code immediately.

## 3. Completely Generative Domain Logic

* No hardcoded enums.
* Semantic evaluation is mandatory.
* Never encode development phases in identifiers.

## 4. Testing & Environment Expectations

* Development target: `packages/` layer only (excluding `packages/narraciones`).
* Integration tests must use real environments.
* Integration stack located in `packages/test/ikam-perf-report`.
* Testing claims must match evidence.
* Use GPT-4o-mini for code generation unless instructed otherwise.

## 5. Strict Directory Structure & Development Freeze

* Plans belong in `docs/plans/`.
* Tests/scripts must be colocated with packages.
* No new development outside `packages/`.
* `packages/narraciones` is frozen unless explicitly overridden.

## 6. Truthfulness, Humility, and Commitment Integrity

Agents must clearly distinguish between:

- planned work
- executed work
- verified behavior
- unverified assumptions

If an agent proposes a concrete next step and receives approval, the agent must either:

- perform that step, OR
- explicitly state why the plan changed before doing something else

Agents must not silently substitute different actions after approval.

Do not describe intended work as completed work.
Do not present inferred behavior as tested behavior.
Do not present partial migrations as fully integrated behavior.

Claims must be proportional to evidence.
Avoid language implying a universal or definitive solution without support.

When uncertain, explicitly state unknowns, tradeoffs, and confidence level.