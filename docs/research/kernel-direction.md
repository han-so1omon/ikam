# Literature review: kernel design direction (2026-10-04)

Papers and extracted text are cached in a session cache (not committed). **[read]** means I read the text myself. **[inf]** means my own inference, not a claim from the paper.

## Questions
1. Lossless compression as program or library synthesis.
2. Using e-graph extraction to choose which derivations to store.
3. Learned models as exact compressors, and whether they are deterministic.
4. Compression-based similarity as the weight on claim edges.
5. Lossless template mining, the closest analogue to `fill`.

## Papers read

**1. Shi et al., "Lossless Tensor Compression as Program Synthesis" (Brevis), arXiv:2608.02162 (2026).** Full text, sections 1–5.
- Typed DSL with these operators: lit, const, concat, repeat, map, scan, merge. A `lit` fallback means every tensor always has some valid program.
- Search is target-directed: a node is expanded only if decomposing its target stream recomposes exactly (Eq. 8), so exactness holds by construction. Bounded A* ranks candidates by their full serialized size L(P), including codec tables.
- Results: 2.13 TB down to 1.41 TB (−33.9%), and 12.9% smaller than zstd.
- Ablation on a 4.98 GB shard (Table 2, §4.4):
  - A budget of **1 expansion** captures most of the gain. Budgets 32 and 256 save only 3.1 MB and 5.5 MB more, at 15× and 155× the time.
  - Removing A* costs +2.87 MB. Removing the learned prior costs **+389 bytes**.
- Tensors are synthesized independently; cross-tensor synthesis is future work.
- Maps to: `func.rs` builtins plus the ingest candidate choice. Our order is the reverse of theirs: we verify after building rather than building correct by construction. Lesson **[inf]**: a shallow search with an exact size objective takes almost all of the gain, and a learned search prior is close to worthless.

**2. Collet et al., "OpenZL: A Graph-Based Model for Compression", arXiv:2510.03203 (2025).** Sections 1–6 read in part.
- Compression is a DAG of codecs with a self-describing wire format, so one universal decoder reads any graph.
- A trainer clusters parsed streams by correlation.
- On psam CSV it is "55% better than xz-9".
- On enwik7 (text), training finds nothing and it "simply uses zstd-6".
- Maps to: the ledger is already a self-describing graph of functions. The universal decoder is our `reconstruct`. Their stream clustering is the missing piece for **office** (H2).

**3. Cao et al., "babble: Learning Better Abstractions with E-Graphs and Anti-Unification", POPL 2023, arXiv:2212.04596.** Sections 1–4 and 6.
- Candidate abstractions are only least-general anti-unifiers of pairs of subterms, and must be used at least twice.
- An equational theory exposes shared structure through equality saturation. Selection is "targeted CSE" with beam search. Corpus size includes the library.
- 2D CAD results (Table 2): compression ratio 9.23 → 10.90 (Nuts & Bolts) and 5.25 → 7.09 (Gadgets) when rewrites are added. 1–2 orders of magnitude faster than DreamCoder.
- Maps to: `template::anti_unify` is pairwise least-general generalization over bytes. Their rewrites correspond to our structure-exposing inverse transforms (`deflate-pack`) (H5).

**4. Bowers et al., "Top-Down Synthesis for Library Learning" (Stitch), POPL 2023, arXiv:2211.16605.** Abstract, introduction, §2 and §6 summary.
- Branch-and-bound top-down search. The upper bound on utility is the sum of subtree sizes over match locations.
- 3–4 orders of magnitude faster than DreamCoder and about 100× less memory, with comparable compression. Stopping early still gives good libraries ("anytime").
- Maps to: `repack::induce_into`. The star test "shares more bytes than a fill record costs" is a weaker form of their bound.

**5. Ellis et al., "DreamCoder", arXiv:2006.08381 (2020).** Abstract and §on wake/sleep.
- The abstraction-sleep objective is minimum description length: library description length plus the description length of the refactored programs.
- Maps to: the evaluator's rule "recurse while the estimated encoded size shrinks", where template bytes are counted. That is the same objective.

**6. Willsey et al., "egg", POPL 2021, arXiv:2004.03082.** Abstract, §2 and §4.3.
- Greedy bottom-up extraction is optimal only for *local* cost functions, where a node's cost depends only on its symbol and its children's costs.
- Maps to: our cost is not local. A stored id is paid for once, however many derivations read it.

**7. Yin et al., "e-boost", arXiv:2508.13020 (2025).** Abstract and introduction.
- Extraction under DAG cost (each shared subterm counted once) is NP-hard.
- Their method: heuristic pass, then pruning, then a warm-started ILP. 558× faster than plain ILP and 19% better than SmoothE.
- Maps to: `repack` stored-copy drops, which currently pick largest first, greedily (H4).

**8. Delétang et al., "Language Modeling Is Compression", ICLR 2024, arXiv:2309.10668.** Abstract, §2–3 and Table 1.
- Chinchilla-70B raw rate: 8.3% on enwik9. Counting the parameters, the adjusted rate is **14,008%** on 1 GB.
- A 3.2M-parameter transformer trained on enwik8 gets 17.0% raw and 17.7% adjusted, but only on in-distribution data.
- Prequential (online) coding counts only the training script, not the weights.
- Maps to: the "learned transforms" idea. At our corpus sizes (0.1–2 MB), any stored model larger than tens of KB cannot pay for itself **[inf from their adjusted-rate arithmetic]**.

**9. Valmeekam et al., "LLMZip", arXiv:2306.04050 (2023).** Abstract and method.
- LLaMA-7B plus arithmetic coding reaches 0.71 bits/char on 1 MB of text8.
- The decoder needs the identical model producing identical probabilities.

**10. Du et al., "Greedy Decoding Is Not Precision-Invariant", arXiv:2609.26621 (2026).** Abstract only.
- BF16 vs FP16 on the same hardware: 49–100% of prompts diverge.
- The FP32 fix is partial and fails at batch ≥ 8.

**11. Zhu et al., "HEAL", arXiv:2606.21023 (2026).** Abstract only.
- Divergence across GPUs comes from downcast truncation at kernel boundaries.
- Maps to (with 10): model-based codecs must run in the deterministic WASM sandbox on CPU, never on GPU kernels **[inf]**.

**12. Yoran et al., "The KoLMogorov Test", ICLR 2025, arXiv:2503.13992.** Abstract, §4–5 and Table 1.
- Prompted frontier models write programs that fail to reproduce the data 78% (Llama-405B) and 40% (GPT-4o) of the time on natural data. Even correct programs are longer than gzip (precision > 1).
- Trained 1.5B–8B "SeqCoders" beat gzip on synthetic data (rate 0.38) but transfer poorly to real data.
- More than 95% of correct programs on natural data are just repetitions.

**13. Cilibrasi & Vitányi, "Clustering by Compression", IEEE TIT 2005, arXiv:cs/0312044.** Abstract and §3.
- NCD(x,y) = (C(xy) − min(C(x),C(y))) / max(C(x),C(y)).
- It is a metric only for a "normal" compressor: idempotent, monotone, symmetric and distributive.

**14. Cebrián, Alfonseca & Ortega, "Common pitfalls using the NCD", Commun. Inf. Syst. 5(4) 2005, pp. 367–384.** Full text.
- NCD(x,x) jumps to about 0.9 once a pair exceeds gzip's 32 KB window. bzip2 has the same jump at half its 100–900 KB block size.
- The objects being compared must fit inside the compressor's window.

**15. Li et al., "LogShrink", ICSE 2024, arXiv:2309.09479.** Abstract, §3 observations, §5 and Tables 5 and 7.
- Logs are split into template plus variables, and the variables are stored **column-oriented**.
- Column layout alone cuts compressed size by 36–103% against row layout (Observation 2).
- The longest-common-subsequence and entropy "analyzer" adds 23% compression ratio on average (2.9–64.8%).
- Overall, 16–356% better than gzip/lzma/bzip2/LogZip/LogReducer.

## Hypotheses (measure with `cargo run --release --example bench -- --json`; every row must stay exact and fsck-clean)

**H1. Columnar fillers.** Source: LogShrink, OpenZL.
- **Change:** in `repack::induce_into`, store a cluster's fillers column-major. That is one content per hole index j holding every member's filler j, or one blob ordered by hole. Each member's filler list becomes `concat(header literals, ranges into the column blob)`, which needs no new builtin.
- **Expected:** fillers of the same kind (dates, amounts, names) end up adjacent, and zstd plus the dictionary compress them better.
- **Measure:** invoices, synthetic-history and pdf bytes after repack.
- **Risk [inf]:** the cost of each range in a derivation record may eat the gain. That is a reason to do "cheaper derivation records" first.
- **Status:** `open` (not yet run).

**H2. Per-cluster dictionaries.** Source: OpenZL stream clustering, plus our office gap.
- **Change:** reuse the star clusters from repack. Train a zstd dictionary per cluster, or per extension or member path for containers. Keep the existing rule that a dictionary must save more than its own size.
- **Measure:** office (516,710 vs 391,236 for the baseline), md, and repack time.
- **Status:** `open` (not yet run).

**H3. Normalized, window-safe claim weights.** Source: NCD, Cebrián.
- **Change part (a):** in `claims::relate`, rank by `gain / (8·C(subject))`, an NCD-style normalization, rather than raw `gain_bits`. Keep raw `gain_bits` stored.
- **Change part (b):** add a test of `conditional_gain(x, x)` against `8·C(x)` for |x| from 1 KB to 16 MB, to find where zstd level 3 stops matching against the dictionary content.
- **Measure:** `cargo run --release --example relatedness`. The baseline is 53.4% structural and 26.5% same-company. The bench should show no size change.
- **[inf]** Raw gain favours large neighbours, and the zstd window would quietly truncate gain for large objects.
- **Status:** `open` (not yet run).

**H4. DAG-cost extraction in repack.** Source: egg, e-boost, babble.
- **Change:** after all verified derivations are collected, choose stored ids by greedy marginal saving (bytes freed minus `read_weight·work`), recomputed after each drop. Follow with one local-improvement pass (try to un-drop and re-drop each id). This replaces the single largest-first pass.
- For small stores, an exact ILP could serve as an oracle to measure the greedy method's gap.
- **Measure:** all corpora, especially repo-history and synthetic-history, plus repack time. Also re-run ingest in both orders (the 34.3 vs 21.9 KB case) to check order independence.
- **Status:** `open` (not yet run).

**H5. Structure-exposing inverse transforms before templating.** Source: babble equational theories (CR 9.23 → 10.90); Brevis `merge`/`map`.
- **Change:** a PDF FlateDecode derivation, already in Next steps and analogous to `deflate-pack`. The inflated streams then become candidates for templates and the dictionary.
- **Measure:** pdf bytes (25,043 vs 24,643 for the baseline) and time.
- **Status:** `open` (not yet run).

## Matches and divergences

**Matches:**
- Verify-then-record is the same idea as KoLMogorov Test's "accuracy" gate and Brevis's bit-exact acceptance.
- Choosing by total serialized size including tables is the same as Brevis's L(P) and the minimum-description-length rule (library counted) in DreamCoder, babble and Stitch.
- Recursive templating with no level cap mirrors Stitch's iterated abstractions that build on each other.
- A universal fallback to raw bytes matches Brevis's `lit`.

**Divergences:**
1. Brevis and Stitch make candidates correct by construction or prune them by bounds. We build, then verify. That is fine, but it spends time on candidates that fail.
2. Our extraction is greedy under a non-local cost (H4).
3. Literature library learning works on trees. Ours works on bytes, so "modulo theory" exists only as container unpacking (H5).

**Likely dead ends [inf, from the evidence]:**
- **Learned search priors over our functions.** Brevis's prior is worth 389 B out of 4.98 GB.
- **Deep search budgets.** Brevis's budget of 1 already gets most of the gain.
- **Stored neural models at our corpus sizes.** Delétang's adjusted rates rule them out.
- **LLM-written programs as storage derivations for natural text.** KT: high failure rates, longer than gzip, and gains mostly from repetition.
- **Prequential, deterministic CPU context-mixing models as WASM functions.** This is the only learned path not ruled out. Untested.

## Unknowns
- Whether H1's per-range record overhead beats the gain from columnar layout. Our filler counts per cluster are small (30 invoices), unlike logs with millions of lines.
- The actual zstd window behaviour of `with_dictionary(3, object)` for large objects. I have not tested it.
- Whether a stored object that starts with the zstd dictionary magic number gets parsed as a formatted dictionary in `conditional_gain`. Unverified.
- I read e-boost, HEAL and the precision-divergence paper only at the abstract or introduction level. None of these hypotheses has been run.
