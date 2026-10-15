# Latent reasoning (latent chain-of-thought)

Question: does "latent thinking" fit IKAM's research directions — semantic deduplication, graph structuring, intermediate representations, and filtering? Latent thinking means models that reason in hidden states or compressed tokens instead of written-out text. Source list: https://github.com/EIT-NLP/Awesome-Latent-CoT (about 288 entries up to 2026-06). It is the paper list for the survey *Reasoning Beyond Language* (Chen et al., arXiv:2505.16782).

How this was read: through a delegated research pass in this session. The read depth for each paper is listed honestly below. Equations and some table cells were lost when the text was extracted, so a few numbers come from the prose only.

## The field in one paragraph

The survey splits methods into two families.
- **Token-wise ("horizontal")** methods replace written reasoning steps with latents. Those latents can be discrete (pause or planning tokens, VQ codes) or continuous (hidden states fed back in, soft mixtures of token embeddings).
- **Layer-wise ("vertical")** methods reason by looping or recurring depth instead of emitting tokens (Huginn, looped transformers, HRM, TRM).
- The survey's own GSM8K table (LLaMA-3-1B) has latent methods mostly *below* explicit CoT: CoT-SFT 66.7, CODI 55.6, Coconut 45.3, CoLaR 40.1.
- The open problems the authors name are training instability, generalisation, and the faithfulness and verification of latents.

## Papers read

1. **Hao et al., Coconut (COLM 2025), arXiv:2412.06769.** Full text, §4–5.
   - Technique: the last hidden state is fed back in as the next input.
   - GPT-2 on GSM8K: 34.1% with 8.2 tokens, vs CoT's 42.9% with 25 tokens.
   - On ProsQA (graph path search): 97.0% with 14.2 tokens, vs CoT's 77.5% with 49.4. But iCoT reaches 98.2% and "no thought" 95.5%, so much of the gain comes from the curriculum.
   - Probing suggests the latents hold several search frontiers at once (BFS-like).
2. **Shen et al., CODI (EMNLP 2025), arXiv:2502.21074.** Full text, §4–5.
   - Technique: self-distillation from explicit CoT into continuous thoughts.
   - Accuracy: 99% of CoT-SFT on GSM8K with GPT-2, 90% with LLaMA-1B.
   - Compression: 3.1× on GSM8K-Aug, 8.2× on its verbose variant. These are token counts, not bytes.
   - Decoding the latents back into intermediate results (top-5 match): 97.1% / 83.9% / 75.0% for 1, 2 and 3 steps.
3. **Tan et al., CoLaR (NeurIPS 2025), arXiv:2505.16552.** Full text, method and results.
   - Technique: c token embeddings are merged into one latent by a scaled sum, so their order is lost.
   - c=2: −53% chain length for −4.8% accuracy.
   - With RL: up to +5.4% accuracy on MATH and −82.8% length.
   - Fails beyond the compression factors it was trained on.
4. **Su et al., Token Assorted, arXiv:2502.03275.** Method in full; results from the abstract and figure captions only.
   - Technique: a VQ-VAE turns each 16-token chunk of early reasoning into one discrete code (codebook of 1,024).
   - Results: +4.1–13.3 points on maths benchmarks, with traces 17% shorter.
5. **CIRF, arXiv:2605.28292.** Full text.
   - Technique, in steps:
     - embed reasoning steps;
     - **subtract each question's mean step embedding** to remove topic bias;
     - VQ-code the result into reusable "functional units".
   - Qwen3-8B GSM8K: 71.2% vs CODI's 25.5%.
   - The codes learn *operations* (addition, multiply/divide), not shared strings.
6. **R-Capsule, arXiv:2509.22131.** Abstract and §3.2–3.5.
   - Technique: only the high-level plan is compressed into latents; the steps stay explicit.
   - Results: −48–60% tokens and +3–4 points on GSM8K.
   - Compressing the detailed steps did worse.
7. **PLaT, arXiv:2601.21358.** Abstract and §4.2–4.6.
   - Technique: a deterministic latent planner, with a sampled decoder that writes the plan out as text.
   - Pass@1 is worse than CODI's (36.2% vs 54.6%), but Pass@128 is better (80.6% vs 72.4%).
   - The authors position it as a candidate generator for a downstream verifier.
8. **"Do Latent Tokens Think?", arXiv:2512.21711.** Abstract; §4.3 and §5.3 skimmed.
   - Swapping Coconut latents between examples leaves MMLU accuracy unchanged (60.9 → 61.0). Swapping CoT text drops it (62.8 → 43.4).
   - So the latents carry little information specific to each example.
9. **Geiping et al., Huginn, arXiv:2502.05171.** Abstract, §5.3 and §6.
   - Recurrent depth with a per-token early exit when successive steps converge.
   - It uses more steps on harder categories.
   - Saunshi et al., arXiv:2502.17416 (abstract and introduction): looped models match deeper ones on reasoning, but not on memorisation.

Abstract only:
- **DEBATER**, arXiv:2502.12974: retrieval embeddings shaped by latent reasoning.
- **LaME**, arXiv:2606.13061: 60× faster than embedders that write out explicit CoT, on MMEB-v2.
- **LaMR**, arXiv:2605.15315: up to 31% more tokens pruned from a coding agent's context.

## How it maps onto the kernel

The bound: the kernel stores bytes and derivations it verifies exactly. Every latent above has four problems for storage:
- it is lossy (sums of embeddings, many-to-one VQ codes);
- it is tied to one model;
- it is not deterministic across hardware (`kernel-direction.md`: BF16 vs FP16 diverges on 49–100% of prompts);
- its decoder weights cost far more to store than the corpora they would encode.

So a latent can be a **proposal** (an ordering, a shortlist, a candidate program, a candidate claim edge), never stored truth. This is the same propose → verify → keep-if-cheaper pattern the kernel already uses for embeddings (E019) and LLM programs (E025).

Their "compression" figures are reasoning-token counts judged by answer accuracy, not bytes with exact reconstruction, so none of them compares to our score. E017–E020 also found that measured compression beat every resemblance signal tried. Theme by theme:
- **Semantic deduplication:** latent equality is not byte equality. It is not even reliable semantic equality: per paper 8, latents can be swapped between examples with no effect. Latents as dedup keys: **not applicable**. Semantic equivalence beyond bytes still belongs to claims, whose edge weights are measured.
- **Graph structuring:** latents may propose *candidate* claim edges. Weights stay measured `conditional_gain`.
- **Intermediate representation:** the derivation DAG already is the explicit, verifiable "plan". A latent plan can be content-addressed as opaque bytes, but it can't be a derivation unless its decoding is exact and deterministic (CPU WASM).
- **Filtering:** spend LLM effort adaptively (after Huginn), using cheap *measured* difficulty signals rather than latents.

## Hypotheses

The read sources above ground each hypothesis. Every expected effect is an inference until a benchmark run tests it.

- **H-L1: sample k candidate programs and let the kernel verify them.** Source: PLaT, Coconut's BFS behaviour. **Open.**
  - Change: in `functions/write_programs.py`, sample k independent programs per file instead of 3 sequential attempts.
  - Measure: exact@k and dollars per exact program on `generated`; bench score only when a smaller program wins.
  - Needs no latent model.
  - Limit: E025's failures (server.log, mandelbrot) were under-determined by the samples, so more samples may not fix them [inference].
- **H-L2: mean-centred embeddings in the embedding proposer.** Source: CIRF. **Open.**
  - Change: in `semantic/embed.py` → `order.txt`, subtract each document's (or tree's) mean member embedding, so members cluster by role (styles.xml with styles.xml) rather than by topic.
  - Measure: the E019 per-proposer bytes on md, office and repo-history.
  - Run it after `semantic-grouping.md` H3 (path order), which captures role more cheaply. Expected small.
- **H-L3: a segment codebook to seed template clusters.** Source: Token Assorted, CIRF. **Open, speculative.**
  - Change: VQ codes of lines or paragraphs propose which contents `induce_into` tries to template together.
  - Measure: invoices and synthetic-history bytes, and the count of verified `fill` derivations.
  - Risk: codes group operations, not shared bytes [inference].
- **H-L4: latent-reasoning embeddings as candidate generators for claims.** Source: DEBATER, LaME (abstracts). **Open.**
  - Change: a committed list of embedding neighbours adds candidates to `claims::relate`, beyond shared seeds.
  - Measure: `cargo run --release --example relatedness` against 53.4% structural and 26.5% same-company. Bench sizes unchanged.
  - The most natural fit, because relatedness is a semantic target and grouping is not.
- **H-L5: adaptive LLM effort.** Source: Huginn early exit; CIRF's code length tracks difficulty. **Open.**
  - Change: choose which files go to `write_programs.py`, and how many attempts each gets, from a measured difficulty proxy (the file's vs_dict ratio or the compression rate of its samples).
  - Measure: dollars per byte saved.

Not applicable:
- Latent traces or plans as stored content: not verifiable and not economical.
- Latent compression of reasoning transcripts as a storage method. If transcripts become a corpus (a benchmark change), byte-level grouping and dictionaries apply.

## Creative directions (speculative)

The idea here is a reframing, not adopting latent models. IKAM's derivation DAG already *is* an explicit, verified "chain of thought" that produces each file:
- `concat` of slices, `fill` of templates and WASM programs are the reasoning steps;
- the stored bytes are the premises.

So the latent-CoT literature can be read as ideas about *traces*: shorter traces, internalised traces, adaptive depth, plans versus steps. Each idea below is an inference, and none has been measured.

- **C1: trace shortening on the read path (cf. Coconut and CODI: skip the explicit intermediate steps).**
  - The evidence: E045's md reads cost 2,135,673 against a 177,939 baseline (12x). Possible cause [unverified]: layered templates rebuild each intermediate template whole before filling it.
  - The idea: evaluate `fill` lazily, resolving each output piece through the levels as slices, without materialising intermediate templates. Read work then tracks the output size rather than the sum over levels.
  - It needs no new storage, much as E043 made graph reads single-pass.
  - First step: instrument which functions produce md's read work.
  - **Status: refuted by E048, and redirected.** md's read cost is all group-prefix decoding (no function work), which the planner already prices within 1%. The same instrumentation found the planner over-pricing slice plans by 40–58%. Fixing that (one read per source per derivation) was kept: score 0.3005 -> 0.2972.
- **C2: compression-induced concept nodes (graph structuring from what compresses together).**
  - Templates, groups and dictionaries are latent concepts the planner discovers and verifies: shared structure across instances, kept only because it saves bytes (MDL).
  - The idea: expose them as graph nodes (`instance-of` template, `member-of` group), so the storage plan yields semantic structure for free.
  - Test: `examples/relatedness.rs`. Do shared-template and shared-group edges predict the structural (53.4%) and same-company (26.5%) pairs better than seed candidates? Storage stays unchanged.
  - **Status: partial (E049, on its branch).** Group membership is at chance. The member stored next to a file in its group matches measured gain on structural relatedness (52.8% vs 53.4% same filename) at no cost, but not same-company relatedness (8.7% vs 26.5%). It covers only grouped files. Template and slice concepts are untested, because md's plan has none.
- **C3: operation / result split for programs (cf. CIRF's functional tokens versus result tokens).**
  - Anti-unify *programs* the way `template.rs` anti-unifies bytes: one shared program skeleton (the operation), with per-file parameters as fillers (the result).
  - E025's failures (server.log, mandelbrot) hinged on per-file constants. A skeleton plus parameter slots would let the LLM write the operation once while the kernel fits the parameters.
  - Measure: `generated` bytes and programs reproduced exactly.
  - **Status: not applicable (E053, analysis).** The mandelbrot attempts produced broken programs, not wrong constants. server.log's 64-bit seed can't be fitted from its residues. No current corpus has a family of files sharing one procedure.
- **C4: plan-first ingest (cf. R-Capsule and PLaT: compress the plan, sample, verify).**
  - The idea: a cheap proposer emits a small plan for a new file, for example "version N+1 of F with edits at lines a..b" or "instance of template T". The kernel realises it exactly or rejects it. Today ingest searches by seeds.
  - Measure: ingest time, and bytes at equal score. A failed plan falls back to search, so correctness is unaffected.
  - **Status: refuted as a score lever (E054).** Ingest is 3% of run time and repack re-plans. Kept slices come 99.7% from the same path, but mostly not from the previous version, because the planner picks a shared base.
- **C5: adaptive repack depth (cf. Huginn's convergence exit).**
  - The idea: run repack's induce, group and recompress rounds until the plan cost stops falling, per region rather than globally, spending effort where cost still drops.
  - Every round's plan is content-addressed, so rounds cache.
  - This is the "background optimisation" question raised earlier, reduced to an anytime loop with a measured stopping rule.
  - **Status: refuted (E052).** Repack is a fixed point after one pass on 7 of 9 corpora. A second pass saves 32 B on generated but grows repo-graph, because repack's acceptance test isn't the score.
- **C6: agent workloads as the missing range-read evidence (cf. LaMR context pruning).**
  - Agents that filter their context read *slices* of stored content, not whole files. That is the range-heavy access pattern E047 found verified blocks need (range reads weighted 10x or more).
  - Proposal: record or simulate an agent read trace (grep, open file at line, follow a graph edge) as a benchmark workload, so that f is measured rather than assumed. A benchmark change.
  - **Status: measured (E050, on its branch).** In a real coding-agent trace, files over 16 KiB were never read whole (41 range reads) but were grep-scanned 124 times. f = 0.33 when search decodes content, so verified blocks do not pay. The lever is search without decoding (next hypothesis).
- **C7: reasoning transcripts as a corpus.**
  - Agent transcripts quote files, tool outputs and earlier turns verbatim. Each quote can be a slice derivation of content already stored.
  - Expected: large exact dedup, not the lossy token compression of the papers.
  - Needs a corpus with no secrets or personal data (synthetic or opt-in). A benchmark change.
  - **Status: corpus added (E055, kept as a benchmark change).** The kernel dedups session versions but no quotes: line numbers and JSON escaping break byte runs. A `quote` derivation could save up to about 45% of the corpus (zstd bound with the quoted sources as context).
- **C8: the swap test as a proposer diagnostic (from "Do Latent Tokens Think?").**
  - Swap a proposer's output between corpora, or shuffle it within one. If bytes don't move, the proposer carries no information specific to the instance.
  - A cheap ablation to add to every proposer experiment (embeddings, LLM order, codebooks).
  - **Status: applied (E051), now a protocol step.** Removing, shuffling or reversing the embedding order leaves every corpus byte-identical, so that proposer has no effect.
- **C9 (far-out): model state as content.**
  - Prompt and KV caches are already prefix-addressed in inference servers.
  - IKAM could store them with ids that include (model, hardware, precision), since exact reproduction holds only within that tuple.
  - Out of scope for the kernel today; noted for the architecture plan.
  - **Status: noted** in `docs/plans/2026-10-06-system-architecture.md` with the exactness constraint and what would make it concrete. Not built.

## Unknowns

- No paper found measures latent or embedding similarity against compressed bytes.
- Results are mostly from 1–8B models on GSM8K-style maths. Whether they transfer to file content is untested.
- Partial reads are noted per paper above.
