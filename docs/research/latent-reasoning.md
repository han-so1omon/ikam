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

## Unknowns

- No paper found measures latent or embedding similarity against compressed bytes.
- Results are mostly from 1–8B models on GSM8K-style maths. Whether they transfer to file content is untested.
- Partial reads are noted per paper above.
