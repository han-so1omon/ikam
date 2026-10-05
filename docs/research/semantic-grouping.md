# Which small contents belong in the same group?

## Question

E016 (`docs/experiments/log.md`) compresses small contents together in groups: up to `GROUP_SIZE` = 64 KiB per group, zstd level 3 with the shared trained dictionary, and about 13 B of entry per member. Members are picked by sorting the contents by their raw bytes. The working tree also has an uncommitted `similar_groups` proposer in `packages/kernel/src/group.rs`. It grows each group with the item that shares the most matcher seeds with it, a MinHash-like resemblance.

The question: which signal best predicts whether a set of files compresses well together, and in what order should they go? The candidates are byte order, lexical sketches (MinHash or SimHash), compression distance (NCD) and semantic embeddings or LLM judgements. The store must stay deterministic. A model may only *propose* a grouping: each proposal is built, measured, and kept only if it is smaller. The benchmark must be reproducible offline.

## Sources read

The sources are cached in the session scratchpad under `papers/semantic/` (PDF and text).

1. **Ferragina & Manzini, "On compressing the textual web", WSDM 2010.** https://people.unipmn.it/manzini/papers/wsdm10.pdf. *Full text.*
   - **Mechanism and numbers:** pages of the UK50 collection (50 GB) are concatenated and compressed in blocks, under four orders:
     - **Crawl order:** gzip about 20%.
     - **Random permutation:** bzip\* 9.14% to 13.65%.
     - **URL order, host reversed:** 4.24% with bzip\* on 10 MB blocks, and 3.78% with lzma on 400 MB blocks. URL order helps most for small blocks, up to a factor of 2.7.
     - **Content clustering:** documents become bm25, tf-idf or boolean term vectors, k-means runs with k = 50 or 500, and pages are sorted by (cluster, URL). The result was about 3.95%, with bm25 best and tf-idf worst. That is *worse than URL order and much slower*.
   - An "LSH ordering" (projecting pages onto a random line) was worse still and is not reported. The authors note that shingle-based cluster compressors (refs [46, 24]) are subsumed by their experiment.
   - **Mapping:** this is the closest prior test of "semantic" (term-vector) clustering against a cheap structural order for LZ-family block compression. The structural order won. Our analogue of the URL is a content's path or tree position, which `group_small` does not see today.
2. **Hoobin, Puglisi & Zobel, "Relative Lempel-Ziv factorization for efficient storage and retrieval of web collections", PVLDB 5(3), 2011.** https://www.vldb.org/pvldb/vol5/p265_christopherhoobin_vldb2012.pdf. *Full text, skimmed for ordering.*
   - They quote the Ferragina and Manzini GOV2 numbers: zlib 18.69% in crawl order and 10.41% in URL order. lzma with 1 MB blocks reaches 5.67% on URL-sorted GOV2.
   - Their RLZ builds a dictionary from evenly spaced 1 KB samples, taking 0.1% of the collection. They argue that this *removes* sensitivity to order, and that URL order fails when there are no URLs or on mirrored content.
   - **Mapping:** our shared dictionary already plays the RLZ role, so grouping only has to capture what the dictionary misses: member-to-member redundancy.
3. **Dhulipala et al., "Compressing graphs and indexes with recursive graph bisection" (BP), KDD 2016.** arXiv:1602.08820. *Full text, skimmed.*
   - **Mechanism:** reorder documents by recursive bisection of the document–term bipartite graph, minimising a log-gap cost.
   - **Numbers:** BP gains about 50% over natural order and about 30% over MinHash order (lexicographic order of 10 min-hashes) on large graphs. It gains 22% and 15% on inverted indexes over the best alternatives, including TSP ordering (Shieh et al.) and Silvestri's URL sort.
   - **Mapping:** a better *lexical* objective beats MinHash order, but for posting gaps, not LZ.
4. **Broder, "On the resemblance and containment of documents", 1997.** https://www.cs.princeton.edu/courses/archive/spring13/cos598C/broder97resemblance.pdf. *Full text.*
   - **Mechanism:** resemblance is the Jaccard overlap of w-shingle sets, estimated from fixed-size min-hash sketches. Building a sketch is linear in document length, and comparing two sketches is linear in sketch size.
   - **Numbers:** 30 million AltaVista documents (150 GB) at 50% resemblance gave 3.6 million clusters covering 12.3 million documents.
   - **What it captures:** shared token runs, which is close to what LZ matches exploit.
5. **Charikar, "Similarity estimation techniques from rounding algorithms", STOC 2002.** https://www.cs.princeton.edu/courses/archive/spr04/cos598B/bib/CharikarEstim.pdf. *Full text, skimmed.*
   - **Mechanism:** random-hyperplane hashing gives a bit sketch whose Hamming distance estimates the cosine between weighted feature vectors (SimHash).
   - **What it captures:** cosine over feature weights. It loses token order, unlike shingles of length w.
6. **Manku, Jain & Das Sarma, "Detecting near-duplicates for web crawling", WWW 2007.** https://research.google.com/pubs/archive/33026.pdf. *Full text, skimmed.*
   - **Numbers:** 64-bit SimHash with k ≤ 3 differing bits is a reasonable setting at 8 billion pages, using about 10 permuted tables. The fingerprints are 8 B per page.
   - **Mapping:** a near-duplicate detector, too coarse to rank partial overlap between members.
7. **Cilibrasi & Vitányi, "Clustering by compression", IEEE TIT 2005.** arXiv:cs/0312044. *Full text, sections 3 and 4.*
   - **Mechanism:** NCD(x,y) = (C(xy) − min(C(x),C(y))) / max(C(x),C(y)), which ranges from 0 to 1+ε. gzip and bzip2 gave values above 1. The compressor's window bounds what it can see: gzip has 32 KB. The paper clusters with a quartet tree, which costs O(n⁴).
   - **Cost:** each pair needs a compression of the concatenation, so n² compressions for a full matrix.
   - **Mapping:** with C = our own zstd-3 plus dictionary, the marginal cost C(G+x) − C(G) is *exactly* the quantity a group pays. It is the compression-native similarity, and our 64 KiB groups lie inside zstd's window.
8. **Park et al., "DeepSketch: a new machine-learning-based reference search technique for post-deduplication delta compression", FAST 2022.** arXiv:2202.10584. *Full text, sections 1 to 4.*
   - **Measured problem:** super-feature (Finesse-style) sketches miss a usable reference for 35.7% of blocks on average (up to 75.5%). They pick a suboptimal reference for 23.1%. Compared with brute force, data reduction falls to 0.562 in the missed cases and 0.669 in the suboptimal ones.
   - **Fix:** a neural network learns a hash that is trained on *compression-derived labels*. Its DK-Clustering assigns 4 KiB blocks by their actual Xdelta reduction ratio against cluster means. Lookups use approximate nearest-neighbour search.
   - **Result:** up to 33% more data reduction, 21% on average.
   - **Mapping:** the strongest "ML proposer" precedent. The model learns *compression gain*, not semantics, and it beats lexical sketches.
9. **Abbas et al., "SemDeDup", 2023.** arXiv:2303.09540. *Full text, sections 1 to 5.*
   - **Mechanism:** embed documents (CLIP for images, OPT-125M for text), run k-means (k = 50,000 for images, 11,000 for text), compute pairwise cosine within each cluster, and drop pairs above 1−ε. k-means cuts the n² comparisons by a factor of k.
   - **Numbers:**
     - 50% of LAION-440M was removed with minimal loss.
     - On C4 at 4% pruning, it performed comparably to MinHash NearDup.
     - What it removes from text: templated pages with a few words changed (which MinHash also catches), plus "semantically redundant" pages with no string overlap, such as Nike adverts.
   - **Mapping:** the second kind, same topic with different bytes, is exactly what gives LZ little to match. [inf]
10. **Tirumala et al., "D4", 2023.** arXiv:2308.12284. *Full text, method and appendix A.7.*
    - **Mechanism:** SemDeDup, then prototypicality pruning, on top of MinHash (20 hashes and 20 buckets in Spark MinHashLSH).
    - **Caveats about embeddings:**
      - OPT last-token embeddings cluster documents by a shared *ending*, such as an email footer.
      - SentenceTransformer models (all-MiniLM-L6-v2, all-mpnet-base-v2) truncate input to 256 or 384 tokens, so they see only the start of a document.
      - Embedding clusters are dominated by templates that MinHash with more aggressive settings would already remove.
    - **Numbers:** about 20% training efficiency at 6.7B parameters. These are training-quality results, not compression results.
11. **Delétang et al., "Language modeling is compression", ICLR 2024.** arXiv:2309.10668. *Full text, skimmed.*
    - **Numbers:** Chinchilla 70B reaches 8.3% raw on enwik9 against 32.3% for gzip. Counting the model's size, the adjusted rate is far worse. The context is 2,048 bytes.
    - **Mapping:** this paper does not test whether an LLM's notion of similarity predicts LZ compressibility. It only shows that the LLM is the better *predictor*.
    - **Same for LLMZip** (Valmeekam et al. 2023, arXiv:2306.04050, *skimmed*): LLaMA-7B with an arithmetic coder reaches 0.71 bits per character on 1 MB of text8, and nothing is said about grouping.
12. **zstd README, "The case for small data compression".** https://github.com/facebook/zstd. *Read.*
    - **Claim:** dictionary gains are "mostly effective in the first few KB". After that, the compressor uses previously decoded content.
    - **Mapping:** inside a 64 KiB group, a member's earlier group-mates replace the dictionary. Who comes first and who sits next to whom therefore matters.
13. **2026 work found through alphaXiv** (orx keyword and embedding searches; results in `papers/semantic/orx_search*.txt`).
    - **SemHash-LLM** (arXiv:2607.01601). *Abstract and skim.* LLM-distilled semantic hashing with attention-weighted MinHash for deduplication. It reports no storage or compression numbers.
    - **H3D** (arXiv:2607.08382). *Skim.* A benchmark of MinHash, SimHash, Winnowing and BGE-embedding hashes for duplicate *detection*. It does not measure compression.
    - I found **no paper (2023–2026) that groups files with LLMs or embeddings for storage compression and reports bytes.**

## Hypotheses

The hypotheses are ordered by expected gain over cost.

### H1. Seed-resemblance growth as a second proposer next to byte order

- **Source:** Broder 1997; Ferragina and Manzini's discussion of shingles; BP's MinHash baseline.
- **Change:** this is the uncommitted `similar_groups`. Grow each group greedily by shared matcher seeds, with ties broken by byte order. Build both proposals and keep whichever group set is smaller.
- **Expected effect:**
  - **md and office:** a small gain, because templated near-copies get placed together. Byte order already clusters contents that share a *prefix*, but not ones that share a middle. [inf]
  - **synthetic and repo-graph:** about 0.
- **Cost:** O(Σ seeds) for the index. The greedy step is O(n) per member, so O(n²) per run, which is fine at hundreds of members.
- **Status:** `open` (not yet run).

### H2. Refine by marginal compressed cost, a compression-native NCD

- **Source:** Cilibrasi and Vitányi 2005; DeepSketch (labels and brute force by actual delta gain).
- **Change:** in the greedy step, take the top K = 8 candidates by seed score. Pick the one that minimises `zstd3_dict(G ‖ x) − zstd3_dict(G)`; ties go to byte order. Optionally, order members inside a group the same way: start from the member that compresses best with the dictionary alone. This is still deterministic, since zstd is deterministic at a fixed level and version.
- **Expected effect:** gains where seeds are a poor proxy, for example short members whose overlap falls below the seed length, or overlap with the dictionary rather than with group-mates. DeepSketch's numbers suggest a proxy loses 33% to 44% of the available reduction on the blocks where it guesses wrong, but those blocks are delta pairs, not 64 KiB groups. [inf]
- **Cost:** about K × members compressions of at most 64 KiB each, so a few hundred ms per corpus at level 3. [inf, unmeasured]
- **Status:** `open` (not yet run).

### H3. Path order (the URL analogue) as a proposer

- **Source:** Ferragina and Manzini (URL order beat bm25 and tf-idf k-means clustering, 3.78% against about 3.95%, and gained up to 2.7x on small blocks); Silvestri's URL sort as cited by BP.
- **Change:** `group_small` only sees `(bytes, id, size)` today. Pass the first tree path that names each content, and propose groups sorted by (extension, reversed directory, name).
- **Expected effect:**
  - **office and pdf:** contents from the same document or folder sit together. [inf]
  - **repo-history:** gains if versions of a file are still separate small contents. [inf]
- **Cost:** plumbing paths into repack, plus one sort.
- **Status:** `open` (not yet run).

### H4. Embedding or LLM-informed proposer, offline-reproducible

- **Source:** SemDeDup and D4 (embeddings, k-means and cosine); DeepSketch (a learned proposer).
- **Change:** a separate tool (not in the kernel) embeds each small content with a pinned local model. Candidates are all-MiniLM-L6-v2 via ONNX, with the weights' SHA-256 recorded, or a text summary of the file for an LLM. It writes `embeddings.bin`, keyed by content id, next to the corpus, *committed or hashed*, so the benchmark never calls a model.
  - The kernel reads an optional proposal file: an ordered list of ids. It packs groups in that order and keeps them only if they are smaller than H1 or H2 by the existing check.
  - The ordering is greedy nearest neighbour by cosine, with ties broken by byte order.
  - LLM variant: the model assigns a cached "template family" label per file, which becomes a sort key.
- **Expected effect:** small or none on md and office relative to H1. Embeddings group by *topic*, while LZ needs shared byte runs of at least the minimum match length. Ferragina and Manzini's term-vector clustering lost to URL order, and D4 shows that embedding clusters are distorted by truncation and document endings. A gain is possible only where same-topic files share boilerplate that seeds miss. [inf]
  - The best learned variant would follow DeepSketch: train on measured marginal gains (H2's numbers), not on semantic similarity.
- **Cost:** model inference offline, and a proposal file per corpus.
- **Status:** `open` (not yet run).

## Unknowns

- No source measures embedding cosine against LZ or zstd marginal gain for small files. My expectation that embeddings predict compressibility worse than shingles comes from Ferragina and Manzini's term-vector result, which used bag-of-words rather than neural embeddings, and from what LZ exploits. [inf]
- Ferragina and Manzini's blocks were 10 to 400 MB. Our groups are 64 KiB, closer to their small-block regime, where order mattered *most*. [inf]
- Whether repack can see paths for H3 is unverified.
- How much of a member's redundancy the shared dictionary already captures, which would leave little for grouping to gain. The E016 dictionary shrank from 9,807 to 2,824 B, which suggests that groups and dictionary trade off. [inf]
- The cost of H2 at level 3 has not been measured, and neither has whether H2 orders members better within a group than byte order does.

## Not accessed

- Silvestri, "Sorting out the document identifier assignment problem", ECIR 2007: the author page returned HTML, not the paper. I know its claims only as cited by BP and Hoobin et al.
- Shieh et al. (TSP reordering) and Blandford and Blelloch: not fetched. I know them only through BP's related-work section.
- zstd's `--patch-from` and long-mode documentation: not read in detail. Long mode is irrelevant inside 64 KiB groups.
- The OpenAlex and Semantic Scholar APIs were rate-limited (HTTP 429), so venue-wide searches beyond alphaXiv were not done.
