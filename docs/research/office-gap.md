# Office/markdown storage gap: literature review

Sources were fetched and read in a session cache; they are not committed. Re-fetch by id or URL.

## Questions
1. How should one build a dictionary for many small, similar files: COVER/RLZ construction, dictionary size vs sample count, per-family vs global?
2. Can resemblance detection plus delta compression (super-features, Finesse) store near-duplicate members as deltas?
3. What is known about compressing small XML files or deflate containers (precomp/preflate)?
4. Why might zstd plus a dictionary over whole zips beat zstd over each unpacked member?

## Papers and sources read

1. **Liao, Petri, Moffat, Wirth, "Effective Construction of Relative Lempel-Ziv Dictionaries", WWW 2016, doi:10.1145/2872427.2883042.** *I could not read the paper itself* (the ACM, WWW and author-site PDFs were blocked, and OpenAlex/S2 returned 429). I read zstd's implementation, `lib/dictBuilder/cover.c`, which cites the paper.
   - **Method (from the code):** the corpus is split into epochs, one per segment slot. Each epoch contributes the k-byte segment whose *not-yet-covered* d-mers have the highest total frequency. The frequencies of those d-mers are then set to 0, and segments are written back-to-front so the best ones get small offsets.
   - **Early stop:** training stops after `maxZeroScoreRun = max(10, min(100, epochs/8))` epochs in a row with no scoring segment.
   - **Defaults:** `ZDICT_trainFromBuffer` (used by `zstd::dict::from_samples`) runs the fastCover optimizer with d=8 and steps=4. `zdict.h` recommends "a few thousands samples" and a sample total of "~x100 times the target size of dictionary".
   - **Mapping (inferred):** the kernel requests total/10, which is 10x larger than that guidance. It also trains on *exact-deduplicated* members, which removes the repeated occurrences that create high d-mer frequencies. The early stop is a plausible reason the trainer returned only 6.9 KB. I have not verified this.

2. **Hoobin, Puglisi, Zobel, "Relative Lempel-Ziv Factorization for Efficient Storage and Retrieval of Web Collections", PVLDB 5(3), 2011.** Full text read; the tables were garbled in extraction.
   - **Method:** the dictionary is simply evenly spaced 1 KB raw samples of the collection, with no frequency scoring. Each document is LZ-parsed against it.
   - **Results:** a dictionary of ~0.1% of the collection beats block-compressed gzip/lzma unless documents are URL-sorted. Single-document blocks were "the largest of the block-oriented encodings as there was less redundancy to exploit". A dictionary built from a 1% prefix lost only 1.35% (Table 10: 10.68% vs 12.04%).
   - **Caveat:** parts of the dictionary go unused.
   - **Mapping:** a *raw-content* dictionary made of real member bytes is a viable alternative to COVER. zstd accepts any bytes as a dictionary.

3. **Kuruppu, Puglisi, Zobel, "Reference Sequence Construction for Relative Compression of Genomes", SPIRE 2011, arXiv:1106.3791.** Full text read.
   - **Reference choice matters:** on 39 yeast genomes, compressed size varied from 16.65 MB to 24.42 MB depending on which single sequence was the reference.
   - **Built references win:** references assembled from the corpus's repeats (Re-pair or Comrad grammar rules) beat even the best single sequence. The largest gain was ~2x on E. coli.
   - **Mapping:** a dictionary should be a set of corpus-wide repeats, not one exemplar. For office files that means one copy of each common XML part type (styles, theme, sheet skeleton).

4. **Bille, Gørtz, Puglisi, Tarnow, "Hierarchical Relative Lempel-Ziv Compression", arXiv:2208.11371 (SEA 2023).** Read the abstract, method and results.
   - **Method:** each string uses its *parent* in a tree as the RLZ reference. The tree is a minimum-weight spanning arborescence where edge weight is the RLZ phrase count; an LSH-sparsified graph gives near-equal compression.
   - **Results:** ~2x fewer phrases than the best single reference on E. coli, with smaller gains on SARS-CoV-2 and chr19. Maximum tree depth was 31–206.
   - **Mapping:** this is the principled version of "delta each member against its most similar stored member" (`--patch-from`). Depth must be bounded for decode cost.

5. **Zhang, Xia, Feng, Jiang, Hua, Wang, "Finesse: Fine-Grained Feature Locality based Fast Resemblance Detection for Post-Deduplication Delta Compression", FAST 2019,** https://www.usenix.org/conference/fast19/presentation/zhang. Full text read.
   - **Baseline it improves on (N-transform super-features):** take the maximum over windows of (m_i·Rabin_j+a_i) mod 2³², over 48-byte Rabin windows, to get N features. Hash groups of k features into super-features (SFs); 3 SFs × 4 features is standard.
   - **Finesse's change:** split the chunk into N fixed sub-chunks, take one max feature per sub-chunk, then group by rank.
   - **Results:** 3.2–3.5x faster detection, with delta compression ratio (DCR) within −3.2% to +7.4%. Xdelta is the encoder. The paper cites ~2x extra compression beyond dedup plus local compression on backups.
   - **Mapping:** this is a cheap way to find a base for each non-identical XML member. The kernel's seed anchors could supply the features.

6. **Shilane, Huang, Wallace, Hsu, "WAN Optimized Replication of Backup Datasets Using Stream-Informed Delta Compression", FAST 2012.** Full text read.
   - **Results:** delta compression adds 1.9–4.4x on top of dedup (Table 1). Multi-level delta adds 1.03–1.18x over 1-level.
   - **Key detail for us:** local GZ compression is applied to **~128 KB compression regions of chunks**, not chunk by chunk.
   - **Caveat on base selection:** with repeated headers, more than 10,000 candidates can tie on SF matches, so which candidate is chosen changes the result.
   - **Mapping:** production dedup systems do *not* compress small units one at a time. The kernel's per-blob zstd frames (members are often 0.2–5 KB) give up exactly this cross-unit context.

7. **Augeri et al., "An Analysis of XML Compression Efficiency", ExpCS 2007, arXiv:2410.07603.** Full text skimmed.
   - **Findings:** 14 compressors were tested. "A general-purpose compressor is often the best choice". Binary formats such as WBXML do best on files under ~6 KB.
   - **Limits:** old, and no zstd or dictionaries. This is weak support that XML-specific transforms are not where the gap is.

8. **preflate README (v0.3.5) and precomp README.** These are documentation, not peer-reviewed.
   - **Approach:** split a deflate stream into raw data plus reconstruction info. For zlib streams only the zlib parameters are needed (the kernel's `deflate-pack` level search is a reduced form of this).
   - **Example:** silesia.zip is 99.7% of its size under 7z alone, and **69.7%** after precomp. The gain comes from recompressing the *inflated streams together* with a strong solid compressor.
   - **Mapping:** unpacking members pays off only when they are then compressed jointly, or against a strong shared context. Unpacking and then compressing each member alone at zstd-3 is about equal to deflate per member (inferred).

9. **zstd README "Small Data" section, `zstd.h`, `zstd.1` (`--patch-from`).**
   - **Dictionaries:** "Dictionary gains are mostly effective in the first few KB". "there is no universal dictionary … one dictionary per type of data will provide the greatest benefits."
   - **`--patch-from`:** "effectively dictionary compression with … windowSize > srcSize".
   - **Header trimming:** the frame format can drop the magic (`ZSTD_f_zstd1_magicless`), the content size (`ZSTD_c_contentSizeFlag=0`) and the dictionary ID (`ZSTD_c_dictIDFlag=0`).

Not read: Broder 1997 and Odess (ICDE'21), which I know only through citations in 5 and 6. OnPair (arXiv:2508.02280) was abstract only and is about short-string dictionaries for databases, so it is marginal here.

## Code observations (from reading the code, not measured)
- **Per-blob overhead:** `dict.rs::encode_plain` prefixes every "Y" blob with 1 + 32 bytes (the dictionary id), plus the zstd frame header (magic 4 B, a 2–6 B header that includes the 4 B dictID and content size). That is roughly 40+ B per blob. Across ~2,600 member blobs plus 238 manifests and tree objects, this could plausibly reach 50–100 KB of the 125 KB gap. The size of this effect is unmeasured.
- **Unequal accounting:** the baseline (`bench.rs::baselines`) counts no tree, commit, manifest or id overhead. The kernel's `stored()` counts every object.
- **Training input:** training runs on all bytes-stored content after member dedup (`repack.rs:53-64`), sorted largest first, as one global set.

## Hypotheses

**H1 – Per-object framing dominates the small-blob tail** (sources 6 and 9; partly inferred).
- **Change:** store the dictionary reference once (as a repo-wide default, or a 1-byte index into a dictionary table) instead of 32 B per blob. Also emit magicless zstd frames with `contentSizeFlag=0` and `dictIDFlag=0`; the kernel already knows the size and the dictionary.
- **Expected effect:** ~35–45 B saved per Y blob, which is tens of KB on office and a noticeable share on markdown (343 files × ~40 B ≈ 14 KB of 108 KB).
- **Measure:** first add a breakdown of stored bytes by object kind (Y/Z/B/tree/manifest) and count to `bench --json`, then compare before and after.
- **Status:** `open` (not yet run).

**H2 – The dictionary is undersized and mis-shaped** (sources 1, 2, 3, 9).
- **Change A:** build a *raw-content* dictionary up to the 110 KB budget from concatenated representative members, Hoobin/Kuruppu style. Pick one or a few exemplars per member path family (`xl/styles.xml`, `xl/worksheets/sheet*.xml`, `ppt/slides/slide*.xml`, `word/document.xml`, `theme*.xml`, `[Content_Types].xml`, the zip manifests), with the most frequent first and placed at the dictionary's end. Finalize it with `ZDICT_finalizeDictionary` to add entropy tables.
- **Change B:** keep COVER but train on *pre-dedup* member occurrences, or weight samples by reference count, and request ~total/100. Sweep the size over 16, 32, 64 and 110 KB.
- **Expected effect:** fewer misses on first-occurrence XML vocabulary. Large effect on office; small on markdown.
- **Measure:** office kernel bytes against the 391,236-byte baseline, plus the dictionary size reported.
- **Status:** `open` (not yet run).

**H3 – Per-family dictionaries beat one global dictionary** (source 9: "one dictionary per type"; sources 3 and 4).
- **Change:** cluster stored blobs by container member path (or, for markdown, by file extension or by the first SF). Train one dictionary per cluster with ≥8 samples, and fall back to the global dictionary otherwise.
- **Expected effect:** a gain on office (xlsx, docx and pptx XML share little vocabulary), offset by dictionary storage costs. It could be a net loss at this corpus size (238 files). This is uncertain.
- **Measure:** total bytes, with dictionary bytes included.
- **Status:** `open` (not yet run).

**H4 – Delta against a similar member** (sources 4, 5, 6).
- **Change:** for each unique member not covered by exact or slice dedup, compute Finesse-style SFs (3 SF × 4 features on 48 B windows; the kernel's existing anchors may serve). Pick the base with the most SF matches, ties broken by same member path. Store the member as zstd with the base as a raw dictionary, i.e. `--patch-from` semantics through `Compressor::with_dictionary(level, base)`. Allow the base to be a delta only up to depth N (HRLZ/Shilane: 1-level gets most of the gain).
- **Expected effect:** large on sheet, slide and document XML that differ only in cell values or text. Smaller than the dedup gain on markdown.
- **Measure:** office bytes, plus the count of members encoded as deltas and their average delta ratio.
- **Status:** `open` (not yet run).

**H5 – Joint (solid) compression of small blobs** (sources 2, 6, 8).
- **Change:** pack small unique blobs, up to ~64–128 KB per group ordered by similarity or path, into one zstd frame with an offset index. This is the Shilane "compression region" approach, and it trades random-access cost.
- **Expected effect:** likely the largest single gain on markdown (500 B files) and on office.
- **Measure:** bytes, plus read latency for a single blob.
- **Status:** `open` (not yet run).

**H6 – Accounting parity** (from the code, not the literature).
- **Change:** report the kernel's metadata (trees, commits, manifests, function-application objects) separately, or add the same metadata cost to the baseline.
- **Expected effect:** it shows how much of the 32% gap is real compression loss and how much is bookkeeping. Run this first.
- **Status:** `open` (not yet run).

## Unknowns and weak evidence
- I did not read the Liao 2016 paper itself; its description here comes from zstd's code. I also did not read Odess, Broder, or any "delta compression for dedup" survey.
- No source studies office/OOXML members specifically. The XML evidence is from 2007.
- The reason the trainer returned only 6.9 KB is a hypothesis (early stop after d-mer frequencies run out once exact duplicates are removed). Check it by running the trainer with notification level ≥2 or with explicit `ZDICT_trainFromBuffer_cover` parameters.
- Why baseline-on-zips does well is inferred, not read. Common members deflated at the same level produce identical deflate bytes, so a dictionary over zips (and zstd's long matches) covers them. Zip headers are similarly repetitive. No paper tests this directly.
- The effect sizes for H1–H5 are estimates. The datasets in sources 5 and 6 are GB-to-TB backups, far from this corpus of hundreds of KB.
