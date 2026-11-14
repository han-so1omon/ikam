# What ikam can learn from Tantivy, and from vector and hybrid search

The question: E247 to E249 made ikam's word index fast to update, but its queries and bytes still trail Tantivy's, and ikam has no semantic search. How does Tantivy store and query an index? What does the vector and hybrid literature say about indexes kept up to date from a log of changes? Which ideas are worth borrowing, and where in ikam (the search view, column blocks, the log, views)? Experiments E251 to E255.

## Sources read

**Tantivy 0.24.2 source**, read on disk (the crates `tantivy`, `tantivy-columnar`, `tantivy-bitpacker`, `tantivy-fst`, `tantivy-sstable` and `bitpacking`). The facts below are from the code, not from documentation.

### 1. Postings
- **Blocks:** doc ids are stored in blocks of 128 (`BitPacker4x`, SIMD bitpacking), as strict deltas from the previous block's last doc. Term frequencies are bitpacked beside them, stored minus one.
- **The tail:** a last block of fewer than 128 is stored as varints.
- **Skip data:** a term has skip data only when its document frequency is at least 128. Each full block has one entry:
  - 5 bytes for doc ids only: the last doc and its bit width;
  - 8 bytes with frequencies, adding the frequency bit width and the block's best (fieldnorm, tf) pair for block-max scoring;
  - 12 bytes with positions, adding a running tf sum.
- **Seek:** walk the skip entries to the block holding the target, decode it, then do a branchless binary search over its 128 ids.
- **Positions** are separate blocks of 128 deltas.
- **For ikam:** E244's index is one graph node per posting (`word US row`); E251 stores each word's list as Tantivy's blocks.

### 2. Term dictionary
- **The default** is an FST (BurntSushi's `fst`) from term to ordinal. SSTable is used only with the `quickwit` feature.
- **Term infos** are held in blocks of 256: each block stores its first entry whole and bitpacks the rest relative to it, so finding a term's info is O(1).
- **A TermInfo** holds the doc frequency and byte ranges into the postings and positions files.
- **Prefix, range, regex and fuzzy queries** run automata over the FST, then OR the matching postings into a bitset.
- **For ikam:** the graph is a sorted map already, so a prefix is a range scan. E251 measures what an FST saves.

### 3. Segments and commits
- **Merging** uses LogMergePolicy. Segments are grouped into levels by log2 of their size (0.75 apart, the smallest level 10,000 docs), and a level of 8 segments is merged. Segments over 10,000,000 docs are not merged, and deletes do not trigger a merge by default.
- **Deletes** are a queue of operations with opstamps, a counter that increases with every add, delete and commit. A document is deleted if its opstamp is below the delete's.
- **Each segment keeps an alive bitset**, in a `.del` file named by its delete opstamp.
- **Commit** writes the new segments, then replaces `meta.json` atomically: write a temporary file, sync it, rename it over the old one. `meta.json` lists the segments, their delete opstamps, the commit opstamp and an optional payload string Tantivy itself never reads.
- **Readers** reload by reading `meta.json` under a lock, so they always see one opstamp.
- **For ikam:** E247's segments are a base plus a delta, with dead postings standing in for the alive bitset. E248 already keeps the database state in the commit payload. An ikam commit is a content-addressed root, which already gives this consistency.

### 4. Fast fields
- **Codecs:** numeric columns are written as either bitpacked values (stored minus the minimum, divided by the gcd) or blockwise-linear (a line per block of 512 values, residuals bitpacked). Each codec estimates its size, and the smallest wins; a third, global linear codec exists but is not tried when writing.
- **Reading:** any value is read in O(1), one unaligned 8-byte load and a shift.
- **Optional and multivalued columns** map docs to rows through an index: sparse u16 lists or dense bitsets, in blocks of 65,536 docs.
- **String columns** are an SSTable dictionary plus ordinals.
- **For ikam:** E241's integer columns are zigzag deltas as varints, compressed by zstd 19, with no random access. E252 adds Tantivy's codecs.

### 5. Doc store and directory
- **The doc store** keeps documents in blocks of 16 KB, compressed with LZ4 by default (zstd level 3 as an option). A multi-level skip index finds a doc's block, and an LRU cache holds 100 decompressed blocks.
- **The `Directory` abstraction:** `FileSlice` gives zero-copy ranges of a file, mmap or RAM. `atomic_write` is a temporary file, a sync and a rename. `ManagedDirectory` garbage-collects files no living `meta.json` refers to.
- **For ikam:** ikam stores values as event contents and reads a range by decoding the whole content. E251 packs posting lists into contents of about 16 KB, as Tantivy's doc store does.

### 6. Query execution
- **Intersection** sorts the terms by size, leapfrogs the two rarest by seeking, then seeks the rest.
- **BM25** uses K1 = 1.2 and B = 0.75, with the fieldnorm part precomputed for each of its 256 one-byte values.
- **Block-max WAND,** for pure OR queries of terms, sums global maxima to find a pivot, then compares the blocks' maxima from the skip data with the top-k threshold, and skips blocks that cannot reach it without decoding them.
- **TopDocs** raises its threshold each time its buffer of 2k is truncated.
- **For ikam:** ikam ranks nothing, so block-max WAND has nothing to prune yet. Leapfrog intersection with skips is what E251 borrows.

### Vector and hybrid search
These were read partly through `orx paper`, which fetches a paper's full text that I then searched rather than read cover to cover, and partly through docs and web-search summaries. Each source says which.
- **Malkov and Yashunin, HNSW (arXiv 1603.09320; text searched).**
  - Parameters: M between 5 and 48, Mmax0 = 2M, mL = 1/ln M, efConstruction large enough for about 0.95 recall while building.
  - An insert costs about one search.
  - The graph takes about 60 to 450 bytes per element, plus the vectors.
  - The paper says nothing about deletes.
- **hnswlib (README, search summary) and pgvector (README):** deletes are tombstones, with periodic repair or a rebuild.
- **Xiao et al. (arXiv 2407.07871; abstract):** long runs of deletes leave HNSW nodes unreachable, and recall falls.
- **FreshDiskANN (arXiv 2105.09613; text searched):**
  - Deletes go on a list, and still help navigation until a batch "delete consolidation" at 1 to 10% deletes.
  - The index is a small in-memory index merged into a large one in the background.
  - At about 1B points: 1,800 inserts and 1,800 deletes a second sustained; merging a 10% change takes about 10% of a rebuild; 5-recall@5 of 95% or more.
- **SPFresh (arXiv 2410.14452; abstract and introduction):** partitions updated in place, with only vectors at partition boundaries reassigned, avoiding a global rebuild.
- **LSM-VEC (arXiv 2505.17152; summary only):** a proximity graph across LSM levels.
- **Faiss wiki, IVF (doc):**
  - Centroids must be trained on data distributed like the index's.
  - Search time grows with nprobe.
  - Adding vectors after training is supported.
- **pgvector (doc):** IVFFlat lists are rows/1,000; filters apply after the index scan. A filter keeping 10% returns about 4 rows at ef_search 40, which iterative scans address.
- **Lance (doc and search summary):**
  - Each write is a new dataset version.
  - Deletes are files per fragment, applied at compaction.
  - Rows not yet indexed are searched by a fallback scan until the index is optimized.
  - This is the closest existing design to a log-fed, versioned index.
- **Rust crates:**
  - `hnsw_rs` 0.3.5 (pure Rust, MIT/Apache) supports inserts and a filter during search; neither its README nor its changelog mentions delete.
  - `usearch` 2.26.6 wraps C++ and has `remove`.
  - `instant-distance` builds once.
  - Tantivy has no vector index. ParadeDB pairs Tantivy (pg_search) with pgvector through RRF.
- **Bruch et al., "An Analysis of Fusion Functions for Hybrid Retrieval" (arXiv 2210.11934; text searched):**
  - A tuned convex combination of normalized scores beats RRF on nDCG, and the choice of normalization matters little once alpha is tuned.
  - RRF is sensitive to its parameter and strong zero-shot.
- **Cormack, Clarke and Buettcher (SIGIR 2009; not read, formula as cited):** RRF score = Σ 1/(k + rank), with k = 60.
- **ACORN (arXiv 2403.04871; text searched):** pre-filtering scales poorly at medium-to-high selectivity, and post-filtering fails for restrictive or uncorrelated filters. ACORN filters inside the graph and falls back to pre-filtering below a selectivity threshold.
- **LangChain CacheBackedEmbeddings (source):** the key is namespace (the model) plus a hash of the text.
- **Weaviate (forum and docs, search summary):** a vectorizer cannot be changed in place; you re-vectorize into a new collection or add a named vector.
- **BCT (arXiv 2003.11942; abstract):** training a new model to stay compatible with the old one's embeddings avoids backfilling. Shown on face recognition.
- **WordLlama (`wordllama` 0.4.0 on PyPI, MIT; its bundled RESULTS.md and loader source, read for E256 to E259):**
  - A static embedding model: token embeddings concatenated from several large language models that share one tokenizer (the default, l2_supercat, uses Llama 2's), trained for similarity and mean pooled; no attention. The 256-dimension weights and the tokenizer ship in the wheel.
  - Its MTEB table (for the l3_supercat variant, which RESULTS.md says is a little better or worse than l2 depending on the task) puts it well below all-MiniLM-L6-v2 on retrieval (CQA DupStack 24.2 against 41.3 at 256 dimensions) and above GloVe. So it is a weak semantic signal by the standard of transformer embedders.
  - Mean pooling means a text's vector is a token-weighted mean of its parts' vectors before normalization, which bears on caching vectors by fragment (E256, E258).
- **Known-item and topic judgments (method, no paper read for E256 to E259):** BEIR's sets with human judgments could not be downloaded from the cloud machine. The harness builds two tasks from a sample of PyPI projects' own structure instead: a project's summary as the query for its README, and a Trove `Topic ::` classifier as the query for the projects tagged with it. Tags are incomplete, so topic scores are lower bounds, as BEIR's are with incomplete judgments.
- **Not read:** product quantization (Jégou et al. 2011), Filtered-DiskANN, the Ding and Suel block-max WAND paper, and Paul Masurel's blog posts (one URL found; not fetched).

## Hypotheses and status
1. **Tantivy's postings layout.** A base stored as bitpacked id blocks with skips is smaller and builds faster than one graph node per posting, and intersections are faster.
   - Change: E251's `SegmentOpts::block_base`.
   - Status: **confirmed by E251 for bytes (2.5× smaller) and build (5.4× faster).**
   - **Refuted for most queries:** turning ids back into row keys (a doc map, Tantivy's doc store) dominates them.
2. **An FST term dictionary** is far smaller and faster than a graph of word labels.
   - Change: none (E251's harness).
   - Status: **confirmed by E251 on synthetic syllable words** (12× smaller, 68× faster lookups). That vocabulary flatters an FST; real text would be less regular.
3. **Fast-field codecs.** Tantivy's codecs give column blocks O(1) reads of one value at a small cost in bytes.
   - Change: E252's `ColOpts::fast_numbers`.
   - Status: **confirmed by E252.** 0.2 µs against 2 ms a value, for 6 to 9% more bytes.
4. **IVF and HNSW.** Both can be kept from the kv diff; HNSW gives the best recall per millisecond, IVF is a stored view.
   - Change: E253's `vectors` (IVF as a view; HNSW in memory).
   - Status: **measured by E253.**
   - **E257 on real vectors (WordLlama, 13,055 READMEs and 100,000 paragraphs, real summary queries):** every approximate way recalls less than on E253's synthetic clusters (HNSW ef 64: 0.81 to 0.90, against 0.95; ef 128 reaches 0.87 to 0.94). Flat int8 stays exact but takes 128 ms at 100,000 rows. Data from regions the IVF centroids never saw (5 to 8% of rows) cost no recall.
5. **Bruch et al. on fusion.** Fusion beats either signal when both are informative; a convex combination holds up better than RRF when one is weak.
   - Change: E254's `rrf` and `convex`.
   - Status: **confirmed by E254 on synthetic data.**
   - **Confirmed by E258 on real text** (PyPI READMEs, Tantivy BM25, WordLlama vectors, judgments from summaries and Trove topics). Where the signals were matched (topics), fusion lifted nDCG@10 by 18 to 21%; where vectors were weak (known items), RRF fell 13% below BM25 and convex only 2%.
6. **ACORN and pgvector on filters.** Post-filtering loses recall as a filter tightens, filtering inside the graph keeps recall at growing cost, and an exact pre-filter is cheapest when the filter is tight.
   - Change: none (E254's harness).
   - Status: **confirmed by E254.**
7. **LangChain's cache keys.** Embeddings keyed by model and content hash embed each distinct fragment once, in a build, an update and a model change.
   - Change: E255's `embeds`.
   - Status: **confirmed by E255 for calls.** The synthetic model is too cheap to show the time saved.
   - **E256 on real READMEs and release histories: holds for edits, not for corpus-wide duplicates.** A changed README is 14 to 19% new paragraphs (updates embed 5 to 7 times fewer), but only 19% of a corpus's paragraphs repeat. With WordLlama (about 100 µs a paragraph) the view's own cost about cancels the saving; the case rests on a costlier model.

## Unknowns
- Ranking (BM25 and block-max WAND inside ikam) is not built. Neither are positions and phrase queries.
- Lance-style unindexed fragments and FreshDiskANN-style delete consolidation for HNSW are not tried. E253 keeps HNSW in memory with tombstones; storing it as a view is not tried.
- E256 to E259 use real text (PyPI READMEs) and a real but static embedding model (WordLlama), because the cloud machine could not download a transformer model or BEIR's judged sets. A transformer's cost per call (milliseconds on a CPU, by common report; not measured here) and quality, and human relevance judgments, are still untested.
