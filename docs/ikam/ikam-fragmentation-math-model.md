# IKAM Fragmentation: Mathematical Model and Storage Economics

Date: 2025-11-15
Status: Draft (for review)

## 1. Setup and Notation

- Let A be an artifact represented as a finite bitstring A ∈ {0,1}^*. Equivalently, A is a member of a measurable space (X, d) with a task-appropriate distance d (e.g., normalized edit distance for text, pixel/SSIM for images, structure-aware distance for slides).
- IKAM produces a hierarchy of fragment sets F_0 ⊆ F_1 ⊆ ... ⊆ F_L, where F_k is the set of fragments available up to detail level k (coarser → finer). Assume the decomposition is stable: each refinement only adds fragments (no mutation of existing content) or adds deltas that are monotone-complete (see 1.3).
- Let R be a deterministic reconstructor. Given a fragment set S ⊆ ⋃_k F_k and (optional) radicals r, R(S, r) ∈ X is a reconstruction of A.

Assumptions:
- Canonicalization C: We evaluate distance after projecting artifacts into a canonical form C(X) to eliminate irrelevant formatting differences: d_C(A, B) := d(C(A), C(B)). C is idempotent: C(C(X))=C(X).
- Compatibility: R(F_L, r*) = A for some radicals r* (full information sufficiency).

Monotone-consistency (distance form): For all k, there exists a choice of radicals r_{k+1} such that

$$ d_C\big(A, R(F_{k+1}, r_{k+1})\big) \le d_C\big(A, R(F_k, r_k)\big). $$

Delta safety: If refinement introduces a delta (patch) to an existing fragment, the system re-canonicalizes after patching and requires that applying deltas does not increase canonical distance (no regression under C).

## 2. Monotone Convergence to the Original

Define a partial order on fragment sets by inclusion. Consider the reconstruction sequence \( \hat{A}_k := R(F_k, r_k) \) where r_k are compatible radicals (possibly derived deterministically from F_k or inherited from r* when applicable).

### Theorem 1 (Monotone Non-Increase of Error)

Let \(e_k := d_C(A, \hat{A}_k)\). If (i) \(F_k \subseteq F_{k+1}\) (refinement), (ii) R is monotone-consistent in the distance sense above (including delta safety), and (iii) C is idempotent, then

$$ e_{k+1} \le e_k \quad \text{for all } k, $$

with equality only if the added fragments are redundant under R and C.

Proof sketch:
- Under (i), the reconstructor has (weakly) more information at step k+1.
- Monotone-consistency (ii) implies R(F_{k+1}, r_{k+1}) is at least as informative as R(F_k, r_k) with respect to C; i.e., no previously matched canonical structure is invalidated.
- Therefore the best achievable canonical distance cannot increase.

### Corollary 1 (Convergence to Zero Error)

If there exists L with sufficiency R(F_L, r*) = A, then the sequence \(e_k\) is monotonically non-increasing and reaches 0 at k = L. Thus, as details and fragments are added, we monotonically approach perfect data representation.

Notes:
- In practice, small oscillations in non-canonical metrics can be eliminated by C (e.g., layout jitter). Choosing an application-appropriate C is essential.

## 3. Storage Economics: Single vs. Multiple Artifacts

We compare flat storage (each artifact stored as a whole) to IKAM with content-addressable fragments (CAS) and radicals.

### 3.1 Cost Model

Let N ≥ 1 artifacts A_1, …, A_N derive from a common base dataset D with per-artifact variations (layout, selection, formatting). Define:
- |X|: byte size of object X
- S_flat := \(\sum_{i=1}^N |A_i|\) (conventional storage)
- IKAM decomposes into shared base fragments F_shared(D) and unique fragments U_i per artifact, plus radicals r_i and metadata overhead M.

Assume perfect deduplication via CAS: identical fragment payloads across artifacts are stored once.

Then total IKAM storage is

$$ S_{IKAM} = |F_{shared}| + \sum_{i=1}^N |U_i| + \sum_{i=1}^N |r_i| + M. $$

M includes indices, fragment graphs, and small REP envelopes; \(|r_i|\) are typically small.

### 3.2 Shared Fraction Model and Break-Even Analysis

Let each artifact’s flat size be approximately B bytes on average. Let s ∈ [0,1] be the expected shared fraction attributable to D (reused content) when comparing any pair of artifacts in the family. Then

- Expected shared size: \(|F_{shared}| \approx s\,B\) (stored once)
- Expected unique size per artifact: \(|U_i| \approx (1-s)\,B\)

Plugging in, and bounding radicals and metadata by constants (\(\bar{r}, \bar{M}\)):

$$ S_{IKAM} \approx s\,B + N\,(1-s)\,B + N\,\bar{r} + \bar{M}. $$

Conventional storage:

$$ S_{flat} = N\,B. $$

IKAM is better (smaller) when \(S_{IKAM} \le S_{flat}\), i.e.,

$$ s\,B + N\,(1-s)\,B + N\,\bar{r} + \bar{M} \le N\,B. $$

Rearrange:

$$ N\,B - N\,(1-s)\,B \ge s\,B + N\,\bar{r} + \bar{M} $$
$$ N\,s\,B \ge s\,B + N\,\bar{r} + \bar{M}. $$

For \(s>0\):

Exact bound (separating per-artifact and fixed overheads):

$$ N\,(sB - \bar{r}) \ge sB + \bar{M} \quad \Longrightarrow \quad N \ge \frac{sB + \bar{M}}{sB - \bar{r}} \quad (sB > \bar{r}). $$

Approximate first-order form: Define \(\bar{o} := \bar{r} + \bar{M}/N\). If \(\bar{o} \ll sB\), then using \(1/(1-x) \approx 1+x\) we get

$$ N \approx 1 + \frac{\bar{o}}{sB}. \tag{∗}\,\text{(approx.)} $$

Interpretation:
- For a single artifact (N=1): \(S_{IKAM} \approx B + \bar{r} + \bar{M} > B\). IKAM is typically less space-optimal due to overheads.
- For multiple artifacts with shared fraction s and large base size B, the break-even N decreases. As s or B grows, the right-hand side of (∗) shrinks, and IKAM becomes more space-efficient quickly.

Example: Suppose B=20MB, s=0.6, \(\bar{o}=50\)KB. Then RHS ≈ 1 + 50KB/(0.6·20MB) ≈ 1.004 — IKAM wins for N ≥ 2.

### Lemma: Storage Savings Monotonicity

The savings sequence \(\Delta(N) := S_{flat}(N) - S_{IKAM}(N)\) is non-decreasing in N if each added artifact reuses at least as many bytes as its per-artifact overhead.

Proof (per-artifact view): \(\Delta(N+1)-\Delta(N) = R_{N+1} - c_{N+1} \ge 0\) when reused bytes \(R_{N+1}\) exceed overhead \(c_{N+1}\). Strict increase when \(R_{N+1} > c_{N+1}.\)

Under the shared-fraction approximation: \(\Delta(N+1)-\Delta(N) = sB - \bar r\), so non-decreasing when \(sB \ge \bar r\).

### 3.3 Monotonicity of Savings (Two Views)

Define total savings \(\Delta(N) := S_{flat}(N) - S_{IKAM}(N)\).

- Shared-fraction model (affine form): From the expressions above,

$$ \Delta(N) = (N-1)\,sB - N\,\bar{r} - \bar{M}, \quad \Delta(N+1)-\Delta(N) = sB - \bar{r}. $$

Hence savings grow monotonically in N iff \(sB \ge \bar{r}\), strictly if \(sB > \bar{r}\).

- Per-artifact accounting (exact incremental condition): Let the (N+1)-th artifact reuse R_{N+1} bytes of existing fragments and introduce U_{N+1} new unique bytes, with per-artifact overhead c_{N+1}. Then

$$ \Delta(N+1) - \Delta(N) = (R_{N+1} + U_{N+1}) - (U_{N+1} + c_{N+1}) = R_{N+1} - c_{N+1}. $$

Thus, savings increase for the next artifact iff the reused bytes exceed its overhead; unique bytes cancel in the incremental comparison.

### 3.4 Tightening with Realistic Terms

- Deduplication across non-identical but semantically equivalent blocks improves effective s (via normalization and canonicalization).
- REP inlining for micro-payloads keeps hot-tier reads fast without large-size penalties; \(|r_i|\) and small inline reps are O(10^3) bytes.
- Index/graph metadata M grows sublinearly in N if content-addressable and sparse (DAG with shared parents).

### 3.5 Edge Cases

- s≈0 (no sharing): IKAM adds overhead and loses; choose flat storage or disable fragmentation for those artifacts.
- Very small B: overhead dominates; use policy to bypass fragmentation.
- Heterogeneous B_i: use weighted averages; the inequality generalizes by replacing B with E[B].

## 4. Information-Theoretic Perspective (Optional)

Let K(X) denote Kolmogorov complexity proxy (e.g., MDL under a chosen model). For a family {A_i} derived from D, a two-part code via IKAM approximates

$$ \sum_i K(A_i) \approx K(D) + \sum_i K(A_i\mid D) + O(1), $$

whereas flat storage approximates \(\sum_i K(A_i)\). When \(K(D) \ll \sum_i K(A_i)\), the IKAM factorization is strictly shorter, aligning with the inequality in §3.

## 5. Conclusions

1) Monotone approach: Adding fragments/refinements yields a non-increasing canonical error sequence, reaching zero at full sufficiency.  
2) Storage optimality: While a single artifact pays overhead, families of related artifacts benefit from shared-base deduplication; the break-even threshold (∗) quantifies when IKAM becomes more space-efficient.

## 6. Testable Implications

- Empirical monotonicity: construct F_0 ⊂ F_1 ⊂ … and measure d_C(A, R(F_k)). Verify non-increase and convergence.
- Storage curves: simulate varying N and s to validate (∗) against real fragment/radical sizes from datasets.
