# Incremental views: joins and word search kept from the log

The question: E243 (joins kept up to date) lost to recomputing at 1,000 changes and gained nothing from sharing an index, and E244 (a word index kept as a view) updated 15 to 90 times slower than Tantivy. Where does the time go, and which designs from the literature close the gaps? Experiments E245 to E250.

## Papers read

- **McSherry, Lattuada, Schwarzkopf, Roscoe, "Shared Arrangements: practical inter-query sharing for streaming dataflows", VLDB 2020 (arXiv 1812.02639).** Full text, sections 4 and 5 read.
  - An arrangement is a trace of immutable, indexed batches of updates. Background merging keeps the trace at logarithmically many batches, and the merge work is amortized over the batches as they arrive.
  - Several operators share one arrangement through handles, so a new query reuses indexed state instead of building its own.
  - The join performs "alternating seeks" between the input batch and the other side's trace. Its work is linear in the smaller of the two, and it seeks rather than scans the larger.
  - The paper reports order-of-magnitude gains when many queries share state. It gives no numbers for two queries.
  - Mapping: E243's arrangement is one graph per table, edited in place, with no batches and no merges, and its join seeks once per touched key from the root. Sharing saves the arrangement's own upkeep, which E246 measures. E243 did not share the join itself.
- **"Write-Read Decoupling in Modern Large-Scale Search Engines" (survey, arXiv 2605.01260).** Sections 2.3 and 2.4 read.
  - Lucene keeps an index as immutable segments. New documents go to a RAM buffer that is flushed to a new segment, and TieredMergePolicy merges small segments into larger ones.
  - Deletes are marks against older segments.
  - Logarithmic merging costs O(N log N) total merge work with O(log N) live segments; the survey cites Büttcher and Clarke for the trade-offs.
  - The survey reports write amplification of 10 to 30 times under continuous ingestion.
  - Mapping: E244 edits one sorted graph in place. Its postings are ordered by word and every document holds about 25 words, so a few documents edit leaves across the whole index. A segment design writes only the new postings.
- **Lester, Moffat, Zobel, "Efficient online index construction for text databases", ACM TODS 2008 (doi 10.1145/1386118.1386125).** Abstract only.
  - Geometric partitioning of an online index trades insertion cost against query cost, and can be tuned to the balance of insertions and queries.
  - Mapping: E247's two levels (base plus delta) are the simplest form; more levels are not tried.
- **"Hybrid Product Search Without the Copy: Lexical, Semantic and Filtered Retrieval Served Directly from a Data Lake" (arXiv 2611.11031).** Abstract and introduction read.
  - The lake table is the single system of record, and the search indexes are derived from it.
  - The paper reports a lake commit searchable in 3.8 s.
  - Mapping: E248 keeps Tantivy as a view derived from the ikam log, recording the database state it reflects.

## Hypotheses

1. **Profile (not a paper).** At large batches, a join update's time goes to chunk writes (the planner's slice search on derived chunks) and to per-key seeks that decode the same chunks again.
   - Change: E245's `plain_writes` and `memo`, both opt-in.
   - Status: **confirmed by E245.** At 20,000 rows and 1,000 changes, 312 ms became 133 ms.
2. **Switch (from E243's own trade-off).** Recomputing above a share of touched keys keeps the small-batch win and caps large batches.
   - Change: E245's `recompute_above`.
   - Status: **confirmed for the small-batch win, refuted as a gain once E245's other options are on.** The switch still updates the arrangements, and it beat the faster update only at batches of 3,000 at 20,000 rows.
3. **Shared Arrangements.** Sharing pays as more views read the same indexed state.
   - Change: none (E246's harness).
   - Status: **confirmed by E246, in part.** Sharing the arrangements saves 10 to 24% at 8 views. Sharing the join itself saves (k−1)/k when the views read the same rows.
4. **Lucene segments and deletes, and logarithmic merging.** Updates that write only a small delta and dead postings, with periodic merges, close most of the gap to Tantivy.
   - Change: E247's segments, opt-in.
   - Status: **confirmed by E247, in part.** Updates are 3.6 to 10 times faster than E244's view, but still slower than Tantivy in RAM.
5. **The data-lake paper.** Tantivy derived from the log, caught up from the diff and stamped with the state it reflects, is a sound search path.
   - Change: none (E248's harness).
   - Status: **measured by E248.** The catch-up cost is dominated by ikam's diff (hypothesis 6).
6. **Profile (not a paper).** `graph_diff` lists every leaf of both graphs, so every incremental view pays for the table's size. Skipping subtrees both sides share makes small diffs cheap.
   - Change: E249's `Repo::prune_diff`, opt-in.
   - Status: **confirmed by E249 (search) and E250 (joins).**

## Unknowns

- Alternating seeks in one ordered pass, rather than E245's memo, are not tried.
- More than two levels of segments, as in geometric partitioning or Lucene's tiers, are not tried.
- Tantivy on disk syncs each commit. On this Mac that costs about 65 ms, which E244's cloud numbers did not show. The built-in indexes here ran in memory stores, so their durable costs are not measured.
